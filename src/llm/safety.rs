use globset::Glob;
use rig::{client::CompletionClient, completion::TypedPrompt, providers::openai};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    app::{AccessMode, CommandRisk},
    config::AppConfig,
    tools::{ShellCommandRequest, display_requested_shell_cwd, display_shell_command},
};

use super::agent_builder::reasoning_params;

#[derive(Clone)]
pub(crate) struct SafetyClassifier {
    agent: SafetyAgent,
}

#[derive(Clone)]
enum SafetyAgent {
    Completions(super::OpenAiCompletionsAgent),
    Responses(super::ResponsesAgent),
}

#[derive(Clone)]
pub(crate) enum SafetyClient {
    Completions(openai::CompletionsClient),
    Responses(super::ResponsesClient),
}

#[derive(Clone)]
pub(crate) struct SafetyClassification {
    pub(crate) risk: CommandRisk,
    pub(crate) risk_explanation: String,
    pub(crate) reason: String,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub(crate) enum SafetyClassifierRiskOutput {
    Low,
    Medium,
    High,
}

impl From<SafetyClassifierRiskOutput> for CommandRisk {
    fn from(value: SafetyClassifierRiskOutput) -> Self {
        match value {
            SafetyClassifierRiskOutput::Low => Self::Low,
            SafetyClassifierRiskOutput::Medium => Self::Medium,
            SafetyClassifierRiskOutput::High => Self::High,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SafetyClassifierOutput {
    risk: SafetyClassifierRiskOutput,
    explanation: String,
}

impl SafetyClassifier {
    pub(crate) fn from_client(client: &SafetyClient, config: &AppConfig) -> Self {
        let agent = match client {
            SafetyClient::Completions(client) => SafetyAgent::Completions(
                client
                    .agent(config.safety.model_name.clone())
                    .preamble(safety_classifier_preamble())
                    .additional_params(reasoning_params(
                        &config.safety.model_name,
                        config.safety.reasoning,
                    ))
                    .build(),
            ),
            SafetyClient::Responses(client) => SafetyAgent::Responses(
                client
                    .agent(crate::codex::api_model_name(&config.safety.model_name).to_string())
                    .preamble(safety_classifier_preamble())
                    .additional_params(reasoning_params(
                        &config.safety.model_name,
                        config.safety.reasoning,
                    ))
                    .build(),
            ),
        };
        Self { agent }
    }

    pub(crate) async fn classify(
        &self,
        access_mode: AccessMode,
        args: &ShellCommandRequest,
    ) -> SafetyClassification {
        let command = display_shell_command(&args.script);
        let heuristic = minimum_shell_risk(&command, &args.script);
        let reason = normalize_summary(&args.intent);
        let working_directory = display_requested_shell_cwd(args.cwd.as_deref());
        let prompt =
            safety_classifier_prompt(access_mode, &command, &working_directory, args, heuristic);
        let model_output = match &self.agent {
            SafetyAgent::Completions(agent) => agent
                .prompt_typed::<SafetyClassifierOutput>(prompt.clone())
                .await
                .ok(),
            SafetyAgent::Responses(agent) => agent
                .prompt_typed::<SafetyClassifierOutput>(prompt)
                .await
                .ok(),
        };
        let model_risk = model_output
            .as_ref()
            .map(|output| CommandRisk::from(output.risk))
            .unwrap_or(CommandRisk::High);
        let risk = max_command_risk(model_risk, heuristic.unwrap_or(CommandRisk::Low));
        let risk_explanation = match model_output {
            Some(output) if risk == model_risk => output.explanation,
            Some(_) if risk != model_risk => {
                "Local safety heuristic raised the final risk above the model response.".into()
            }
            _ => "Safety classifier did not provide a usable explanation.".into(),
        };

        SafetyClassification {
            risk,
            risk_explanation,
            reason,
        }
    }
}

pub(crate) fn safety_classifier_preamble() -> &'static str {
    concat!(
        "You classify shell commands for execution safety.\n",
        "Respond using the structured output schema.\n",
        "Set `risk` to Low, Medium, or High.\n",
        "Set `explanation` to a concise justification.\n",
        "Keep `explanation` to 10 words or fewer when possible.\n",
        "Low: clearly read-only and non-destructive inspection commands.\n",
        "Medium: changes are possible but local, non-destructive, and reversible.\n",
        "High: destructive, irreversible, repository-changing, system-changing, network-changing, package-installing, or otherwise risky commands.\n",
        "Classify based on side effects, not whether the command touches a remote or the network.\n",
        "Long-running, polling, watch-mode, or infinite commands can still be Low when they remain read-only and do not modify files, processes, permissions, packages, or system state.\n",
        "Git commands are not automatically High. Read-only git inspection commands such as status, diff, log, show, and ls-remote can be Low, including when they inspect remote refs or metadata. Git commands that modify repository state, update refs, transfer objects into the repo, or may discard work are usually High.\n",
        "If unsure, return High."
    )
}

fn safety_classifier_prompt(
    access_mode: AccessMode,
    command: &str,
    working_directory: &str,
    args: &ShellCommandRequest,
    heuristic: Option<CommandRisk>,
) -> String {
    format!(
        concat!(
            "Access mode: {}\n",
            "Display command: {}\n",
            "Working directory: {}\n",
            "Intent: {}\n",
            "Heuristic minimum risk: {}\n",
            "Script:\n{}\n\n",
            "Return a structured response with `risk` and `explanation`.\n",
            "`risk` must be Low, Medium, or High.\n",
            "`explanation` should be concise: 10 words or fewer when possible.\n",
            "Do not set `risk` below the heuristic minimum risk when one is provided.\n"
        ),
        access_mode.label(),
        command,
        working_directory,
        normalize_summary(&args.intent),
        heuristic.map(CommandRisk::label).unwrap_or("None"),
        args.script
    )
}

fn normalize_summary(summary: &str) -> String {
    let normalized = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        "No reason provided for this shell command".into()
    } else {
        normalized
    }
}

pub(crate) fn minimum_shell_risk(command: &str, script: &str) -> Option<CommandRisk> {
    let normalized = format!("{command}\n{script}").to_ascii_lowercase();
    let high_markers = [
        " rm ", "\nrm ", "rm -", "mkfs", "shutdown", "reboot", "kill ", "killall", "sudo ",
        "chmod ", "chown ", "dd ",
    ];
    if high_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return Some(CommandRisk::High);
    }

    let medium_markers = [
        "mkdir ", "touch ", " mv ", "\nmv ", " cp ", "\ncp ", "tee ", ">>", " >", "install ",
        "sed -i", "perl -pi",
    ];
    if medium_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return Some(CommandRisk::Medium);
    }

    None
}

fn max_command_risk(left: CommandRisk, right: CommandRisk) -> CommandRisk {
    use CommandRisk::{High, Low, Medium};
    match (left, right) {
        (High, _) | (_, High) => High,
        (Medium, _) | (_, Medium) => Medium,
        (Low, Low) => Low,
    }
}

pub(crate) fn shell_pattern_matches(pattern: &str, command: &str) -> bool {
    if pattern.contains('*') {
        Glob::new(pattern)
            .ok()
            .is_some_and(|glob| glob.compile_matcher().is_match(command))
    } else {
        command.starts_with(pattern)
    }
}
