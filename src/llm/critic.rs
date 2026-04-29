//! End-of-turn critic: reruns in a fresh LLM context after a task-bearing
//! turn completes, verifies the agent actually satisfied the active
//! acceptance criteria, and reports `Done` or `NotDone { feedback }`.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::{
    llm::{EventCallback, LlmService},
    stats::StatsHook,
    task::ActiveTask,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum CriticVerdict {
    Done,
    NotDone { feedback: String },
}

#[derive(Clone, Debug, Serialize)]
struct CriticPayload<'a> {
    source_messages: &'a [String],
    task: &'a ActiveTask,
}

pub async fn run_agentic_critic(
    llm: &LlmService,
    reply_id: u64,
    task: &ActiveTask,
    max_tool_steps: usize,
    stats_hook: StatsHook,
    emit: EventCallback,
) -> Result<CriticVerdict> {
    let payload = CriticPayload {
        source_messages: &task.source_messages,
        task,
    };
    let prompt = critic_prompt_for_task(&payload)?;
    let result = llm
        .run_prompt_with_tool_step_limit(
            reply_id,
            prompt,
            Vec::new(),
            None,
            stats_hook,
            None,
            emit.clone(),
            max_tool_steps,
        )
        .await;

    match result {
        Ok(result) => {
            parse_critic_verdict(&result.output).context("failed to parse critic output as JSON")
        }
        Err(error) if LlmService::is_tool_step_limit_error(&error) => {
            let _ = emit(
                reply_id,
                crate::app::StreamEvent::TurnEnded {
                    reason: crate::app::TurnEndReason::Completed,
                    history: None,
                },
            );
            Ok(CriticVerdict::NotDone {
                feedback: format!(
                    "The critic exceeded its verification tool budget of {max_tool_steps} steps. Run the concrete verification checks yourself, narrow the task criteria if needed, and report the checked final state."
                ),
            })
        }
        Err(error) => Err(error),
    }
}

fn critic_prompt_for_task(payload: &CriticPayload<'_>) -> Result<String> {
    let payload_json = serde_json::to_string_pretty(payload)?;
    Ok(format!(
        concat!(
            "Decide whether the active task is complete.\n\n",
            "Instructions:\n",
            "1. For each acceptance criterion, inspect the current environment yourself. Prefer the criterion's verification hint exactly when it gives a command, path, URL, or observable check.\n",
            "2. Use read-only tools to inspect files, search the workspace, browse the web when relevant, and run approval-gated shell commands when needed for verification.\n",
            "3. If any criterion is unsatisfied, unverifiable, or has a verification hint too vague to check, return `not_done` with actionable feedback telling the main agent exactly what to do next.\n",
            "4. If the criteria look wrong relative to the source messages, return `not_done` and explain the correct understanding.\n",
            "5. Do not rubber-stamp. Empty outputs, missing files, failed commands, or claims unsupported by your own checks are not complete.\n",
            "6. Only return `done` when every criterion is supported by checks you performed or directly observable current state.\n\n",
            "Return JSON matching exactly one of:\n",
            "{{\"verdict\": \"done\"}}\n",
            "{{\"verdict\": \"not_done\", \"feedback\": \"...\"}}\n\n",
            "Active task JSON:\n{payload_json}\n"
        ),
        payload_json = payload_json,
    ))
}

pub fn parse_critic_verdict(raw: &str) -> Result<CriticVerdict> {
    serde_json::from_str::<CriticVerdict>(raw.trim())
        .or_else(|_| {
            let start = raw
                .find('{')
                .ok_or_else(|| anyhow!("missing JSON object"))?;
            let end = raw
                .rfind('}')
                .ok_or_else(|| anyhow!("missing JSON object"))?;
            serde_json::from_str::<CriticVerdict>(&raw[start..=end]).map_err(anyhow::Error::from)
        })
        .with_context(|| format!("raw critic output was: {}", raw.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_done() {
        let verdict = parse_critic_verdict(r#"{"verdict": "done"}"#).expect("parses");
        assert_eq!(verdict, CriticVerdict::Done);
    }

    #[test]
    fn parses_not_done() {
        let verdict =
            parse_critic_verdict(r#"{"verdict": "not_done", "feedback": "retry with sudo"}"#)
                .expect("parses");
        assert_eq!(
            verdict,
            CriticVerdict::NotDone {
                feedback: "retry with sudo".into(),
            }
        );
    }

    #[test]
    fn parses_embedded_json_with_prose_wrapping() {
        let verdict =
            parse_critic_verdict("Here is my response:\n{\"verdict\": \"done\"}\nThanks.")
                .expect("parses");
        assert_eq!(verdict, CriticVerdict::Done);
    }

    #[test]
    fn round_trips_through_serde() {
        let original = CriticVerdict::NotDone {
            feedback: "The archive is password-protected; use /app/john/run/7z2john.pl and john."
                .into(),
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed = parse_critic_verdict(&json).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn agentic_prompt_contains_task_but_not_main_turn_evidence() {
        let task = ActiveTask {
            description: "Verify the build".into(),
            criteria: vec![crate::task::AcceptanceCriterion {
                id: 1,
                text: "Tests pass".into(),
                verification_hint: "run `cargo test`".into(),
            }],
            source_messages: vec!["please make the build pass".into()],
            created_at: chrono::Utc::now(),
            next_criterion_id: 2,
        };
        let payload = CriticPayload {
            source_messages: &task.source_messages,
            task: &task,
        };

        let prompt = critic_prompt_for_task(&payload).expect("prompt");

        assert!(prompt.contains("Verify the build"));
        assert!(prompt.contains("run `cargo test`"));
        assert!(!prompt.contains("turn_evidence"));
        assert!(!prompt.contains("last_assistant_reply"));
    }
}
