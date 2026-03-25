use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{config::ReasoningEffort, model_registry};

pub const PLANNING_READY_START_TAG: &str = "<planning_ready>";
pub const PLANNING_READY_END_TAG: &str = "</planning_ready>";
pub const PROPOSED_PLAN_START_TAG: &str = "<proposed_plan>";
pub const PROPOSED_PLAN_END_TAG: &str = "</proposed_plan>";

const MAIN_PLANNING_PROMPT: &str = include_str!("../../../prompts/plan.md");
const SUBAGENT_PLANNING_PROMPT: &str = include_str!("../../../prompts/plan_subagent.md");

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlanningAgentConfig {
    pub model_name: String,
    pub reasoning_effort: ReasoningEffort,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlanningConfig {
    #[serde(default)]
    pub agents: Vec<PlanningAgentConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningJob {
    pub model_name: String,
    pub reasoning_effort: ReasoningEffort,
}

pub fn sanitize_planning_agents(
    current_main_model: &str,
    agents: &[PlanningAgentConfig],
) -> Vec<PlanningAgentConfig> {
    let mut seen = HashSet::new();
    let mut sanitized = Vec::new();

    for agent in agents {
        if agent.model_name == current_main_model {
            continue;
        }

        let Some(model) = model_registry::find_model(&agent.model_name) else {
            continue;
        };
        if !model.supports_reasoning(agent.reasoning_effort) {
            continue;
        }
        if !seen.insert(agent.model_name.clone()) {
            continue;
        }

        sanitized.push(agent.clone());
    }

    sanitized
}

pub fn default_planning_reasoning(model_name: &str) -> ReasoningEffort {
    model_registry::default_reasoning_for_model(model_name).unwrap_or(ReasoningEffort::Medium)
}

pub fn planning_jobs(
    current_main_model: &str,
    current_main_reasoning: ReasoningEffort,
    agents: &[PlanningAgentConfig],
) -> Vec<PlanningJob> {
    let mut jobs = vec![PlanningJob {
        model_name: current_main_model.to_string(),
        reasoning_effort: current_main_reasoning,
    }];
    jobs.extend(
        sanitize_planning_agents(current_main_model, agents)
            .into_iter()
            .map(|agent| PlanningJob {
                model_name: agent.model_name,
                reasoning_effort: agent.reasoning_effort,
            }),
    );
    jobs
}

pub fn planning_conversation_prompt(description: &str) -> String {
    format!(
        concat!(
            "{prompt}\n\n",
            "## Runtime instructions\n\n",
            "- You are starting this planning session before oat runs its automatic planning phase.\n",
            "- Stay in PHASE 1 and PHASE 2 until intent is stable.\n",
            "- If any high-impact ambiguity remains, use AskUser for clarifying questions whenever the ambiguity can be expressed as meaningful multiple-choice options, and do not output a final plan yet.\n",
            "- You may use AskUser multiple times across the planning session. Do not switch to plain-text clarification questions just because you already used AskUser earlier.\n",
            "- Do not output a {plan_start} block before oat has completed PHASE 3.\n",
            "- Once intent is solidified, reply with a single {ready_start} block containing a normalized planning brief and no {plan_start} block.\n",
            "- The normalized brief must include these headings in this order:\n",
            "  1. Summary\n",
            "  2. Success criteria\n",
            "  3. In scope\n",
            "  4. Out of scope\n",
            "  5. Constraints\n",
            "  6. Assumptions\n\n",
            "## Initial task request\n\n",
            "{description}\n"
        ),
        prompt = MAIN_PLANNING_PROMPT.trim(),
        ready_start = PLANNING_READY_START_TAG,
        plan_start = PROPOSED_PLAN_START_TAG,
        description = description.trim(),
    )
}

pub fn planner_prompt(description: &str) -> String {
    format!(
        concat!(
            "{prompt}\n\n",
            "## Stabilized task brief\n\n",
            "{description}\n"
        ),
        prompt = SUBAGENT_PLANNING_PROMPT.trim(),
        description = description.trim(),
    )
}

pub fn planning_finalization_prompt(
    description: &str,
    successful_plans: &[(PlanningJob, String)],
    failed_models: &[String],
) -> String {
    let successful_sections = successful_plans
        .iter()
        .enumerate()
        .map(|(index, (job, plan))| {
            format!(
                "Planner {}: model `{}` effort `{}`\n{}\n",
                index + 1,
                job.model_name,
                job.reasoning_effort.as_str(),
                plan.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let failure_note = if failed_models.is_empty() {
        "None.".to_string()
    } else {
        failed_models.join(", ")
    };

    format!(
        concat!(
            "{prompt}\n\n",
            "## Runtime instructions\n\n",
            "- oat has already completed PHASE 3 automatically.\n",
            "- Continue in PHASE 4.\n",
            "- Review the planner outputs, resolve conflicts using your judgment, and produce the final plan.\n",
            "- If a single remaining high-impact ambiguity still blocks a decision-complete plan, use AskUser when the clarification can be represented as meaningful multiple-choice options; only ask in plain text when that is genuinely not practical.\n",
            "- Do not emit {ready_start} again.\n",
            "- When the spec is decision complete, wrap the final answer in a {plan_start} block.\n\n",
            "## Stabilized brief\n\n",
            "{description}\n\n",
            "## Successful planner outputs\n\n",
            "{successful_sections}\n",
            "## Planner failures\n\n",
            "{failure_note}\n\n",
            "Do a targeted read-only verification pass when concrete references matter, but do not redo the full exploration effort of each planner.\n"
        ),
        prompt = MAIN_PLANNING_PROMPT.trim(),
        ready_start = PLANNING_READY_START_TAG,
        plan_start = PROPOSED_PLAN_START_TAG,
        description = description.trim(),
        successful_sections = successful_sections,
        failure_note = failure_note,
    )
}

pub fn extract_planning_ready_brief(text: &str) -> Option<String> {
    extract_tagged_block(text, PLANNING_READY_START_TAG, PLANNING_READY_END_TAG)
}

pub fn contains_proposed_plan(text: &str) -> bool {
    text.contains(PROPOSED_PLAN_START_TAG) && text.contains(PROPOSED_PLAN_END_TAG)
}

pub fn strip_planning_ready_tags(text: &str) -> String {
    strip_tag_lines(
        &strip_tagged_block(text, PLANNING_READY_START_TAG, PLANNING_READY_END_TAG),
        &[PLANNING_READY_START_TAG, PLANNING_READY_END_TAG],
    )
}

pub fn strip_proposed_plan_tags(text: &str) -> String {
    if let Some(inner) = text
        .trim()
        .strip_prefix(PROPOSED_PLAN_START_TAG)
        .and_then(|rest| rest.strip_suffix(PROPOSED_PLAN_END_TAG))
    {
        return inner.trim_matches('\n').to_string();
    }

    strip_tag_lines(text, &[PROPOSED_PLAN_START_TAG, PROPOSED_PLAN_END_TAG])
}

fn extract_tagged_block(text: &str, start_tag: &str, end_tag: &str) -> Option<String> {
    let start = text.find(start_tag)?;
    let content_start = start + start_tag.len();
    let end = text[content_start..].find(end_tag)? + content_start;
    Some(text[content_start..end].trim().to_string())
}

fn strip_tagged_block(text: &str, start_tag: &str, end_tag: &str) -> String {
    let mut output = String::new();
    let mut remaining = text;

    while let Some(start) = remaining.find(start_tag) {
        output.push_str(&remaining[..start]);
        let after_start = &remaining[start + start_tag.len()..];
        let Some(end) = after_start.find(end_tag) else {
            output.push_str(&remaining[start..]);
            return output;
        };
        remaining = &after_start[end + end_tag.len()..];
    }

    output.push_str(remaining);
    output
}

fn strip_tag_lines(text: &str, tags: &[&str]) -> String {
    let mut stripped = String::new();
    for raw_line in text.split_inclusive('\n') {
        let line = raw_line.trim();
        if tags.contains(&line) {
            continue;
        }
        stripped.push_str(raw_line);
    }
    stripped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_current_main_model_invalid_entries_and_duplicates() {
        let sanitized = sanitize_planning_agents(
            "gpt-5.4-mini",
            &[
                PlanningAgentConfig {
                    model_name: "gpt-5.4-mini".into(),
                    reasoning_effort: ReasoningEffort::Medium,
                },
                PlanningAgentConfig {
                    model_name: "gpt-5.4".into(),
                    reasoning_effort: ReasoningEffort::High,
                },
                PlanningAgentConfig {
                    model_name: "gpt-5.4".into(),
                    reasoning_effort: ReasoningEffort::Low,
                },
                PlanningAgentConfig {
                    model_name: "unknown".into(),
                    reasoning_effort: ReasoningEffort::Medium,
                },
            ],
        );

        assert_eq!(
            sanitized,
            vec![PlanningAgentConfig {
                model_name: "gpt-5.4".into(),
                reasoning_effort: ReasoningEffort::High,
            }]
        );
    }

    #[test]
    fn planning_jobs_always_include_main_model_first() {
        let jobs = planning_jobs(
            "gpt-5.4-mini",
            ReasoningEffort::Medium,
            &[PlanningAgentConfig {
                model_name: "gpt-5.4".into(),
                reasoning_effort: ReasoningEffort::High,
            }],
        );

        assert_eq!(
            jobs,
            vec![
                PlanningJob {
                    model_name: "gpt-5.4-mini".into(),
                    reasoning_effort: ReasoningEffort::Medium,
                },
                PlanningJob {
                    model_name: "gpt-5.4".into(),
                    reasoning_effort: ReasoningEffort::High,
                },
            ]
        );
    }

    #[test]
    fn planning_conversation_prompt_requires_planning_ready_block_before_final_plan() {
        let prompt = planning_conversation_prompt("Add a planning workflow");

        assert!(prompt.contains("Once intent is solidified"));
        assert!(prompt.contains(PLANNING_READY_START_TAG));
        assert!(prompt.contains("You may use AskUser multiple times across the planning session"));
        assert!(
            prompt
                .contains("Do not output a <proposed_plan> block before oat has completed PHASE 3")
        );
    }

    #[test]
    fn planner_prompt_uses_subagent_markdown_asset() {
        let prompt = planner_prompt("Add a planning workflow");

        assert!(prompt.contains("already-stabilized brief"));
        assert!(prompt.contains("Do not ask the user clarifying questions"));
        assert!(prompt.contains("Do not discuss multi-agent orchestration"));
    }

    #[test]
    fn planning_finalization_prompt_requires_targeted_validation_without_reemitting_ready() {
        let prompt = planning_finalization_prompt(
            "Add a planning workflow",
            &[(
                PlanningJob {
                    model_name: "gpt-5.4-mini".into(),
                    reasoning_effort: ReasoningEffort::Medium,
                },
                "High level description\n...".into(),
            )],
            &[],
        );

        assert!(prompt.contains("Continue in PHASE 4"));
        assert!(prompt.contains("Do a targeted read-only verification pass"));
        assert!(prompt.contains("Do not emit <planning_ready> again"));
        assert!(prompt.contains("use AskUser when the clarification can be represented"));
        assert!(prompt.contains("wrap the final answer in a <proposed_plan> block"));
    }

    #[test]
    fn extract_planning_ready_brief_returns_inner_content() {
        let text = "<planning_ready>\n# Brief\n\nDetails\n</planning_ready>";
        assert_eq!(
            extract_planning_ready_brief(text),
            Some("# Brief\n\nDetails".into())
        );
    }

    #[test]
    fn strip_planning_ready_tags_removes_internal_block() {
        let text = "Need one more thing.\n<planning_ready>\n# Brief\n</planning_ready>\nDone.";
        assert_eq!(
            strip_planning_ready_tags(text),
            "Need one more thing.\n\nDone."
        );
    }

    #[test]
    fn strip_planning_ready_tags_drops_lone_wrapper_lines() {
        assert_eq!(strip_planning_ready_tags("<planning_ready>\n"), "");
    }

    #[test]
    fn strip_proposed_plan_tags_removes_wrapper_lines() {
        let text = "<proposed_plan>\n# Plan\n\nDetails\n</proposed_plan>";
        assert_eq!(strip_proposed_plan_tags(text), "# Plan\n\nDetails");
    }

    #[test]
    fn strip_proposed_plan_tags_drops_lone_wrapper_lines() {
        assert_eq!(strip_proposed_plan_tags("<proposed_plan>\n"), "");
    }

    #[test]
    fn contains_proposed_plan_detects_wrapped_plan() {
        assert!(contains_proposed_plan(
            "<proposed_plan>\n# Title\n</proposed_plan>"
        ));
        assert!(!contains_proposed_plan("# Title"));
    }
}
