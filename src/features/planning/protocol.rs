use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    config::{RawReasoningSetting, ReasoningEffort, ReasoningSetting},
    features::planning::state::{PlanningBrief, PlanningReply, ProposedPlan},
    model_registry,
};

pub const PLANNING_READY_START_TAG: &str = "<planning_ready>";
pub const PLANNING_READY_END_TAG: &str = "</planning_ready>";
pub const PROPOSED_PLAN_START_TAG: &str = "<proposed_plan>";
pub const PROPOSED_PLAN_END_TAG: &str = "</proposed_plan>";

const MAIN_PLANNING_PROMPT: &str = include_str!("../../../prompts/plan.md");
const SUBAGENT_PLANNING_PROMPT: &str = include_str!("../../../prompts/plan_subagent.md");

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PlanningAgentConfig {
    pub model_name: String,
    pub reasoning: ReasoningSetting,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPlanningAgentConfig {
    model_name: String,
    #[serde(flatten)]
    reasoning_fields: RawReasoningSetting,
}

impl<'de> Deserialize<'de> for PlanningAgentConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawPlanningAgentConfig::deserialize(deserializer)?;
        let model_name = raw.model_name;
        let reasoning_value = raw
            .reasoning_fields
            .resolve()
            .ok_or_else(|| serde::de::Error::missing_field("reasoning"))?;
        let reasoning =
            model_registry::parse_reasoning_setting_for_model(&model_name, &reasoning_value)
                .map_err(|error| {
                    let message = match error {
                        model_registry::ParseReasoningSettingError::UnknownModel => {
                            model_registry::unknown_model_message(
                                "planning.agents[].model_name",
                                &model_name,
                            )
                        }
                        other => other.message(
                            "planning.agents[].reasoning",
                            &model_name,
                            &reasoning_value,
                        ),
                    };
                    serde::de::Error::custom(message)
                })?;
        Ok(Self {
            model_name,
            reasoning,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlanningConfig {
    #[serde(default)]
    pub agents: Vec<PlanningAgentConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningJob {
    pub model_name: String,
    pub reasoning: ReasoningSetting,
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
        if !model.supports_reasoning(agent.reasoning) {
            continue;
        }
        if !seen.insert(agent.model_name.clone()) {
            continue;
        }

        sanitized.push(agent.clone());
    }

    sanitized
}

pub fn default_planning_reasoning(model_name: &str) -> ReasoningSetting {
    model_registry::default_reasoning_setting_for_model(model_name)
        .unwrap_or(ReasoningSetting::Gpt(ReasoningEffort::Medium))
}

pub fn planning_jobs(
    current_main_model: &str,
    current_main_reasoning: ReasoningSetting,
    agents: &[PlanningAgentConfig],
) -> Vec<PlanningJob> {
    let mut jobs = vec![PlanningJob {
        model_name: current_main_model.to_string(),
        reasoning: current_main_reasoning,
    }];
    jobs.extend(
        sanitize_planning_agents(current_main_model, agents)
            .into_iter()
            .map(|agent| PlanningJob {
                model_name: agent.model_name,
                reasoning: agent.reasoning,
            }),
    );
    jobs
}

pub fn planning_conversation_prompt(description: &str) -> String {
    planning_conversation_prompt_with_mode(description, true)
}

pub fn planning_conversation_prompt_headless(description: &str) -> String {
    planning_conversation_prompt_with_mode(description, false)
}

fn planning_conversation_prompt_with_mode(description: &str, allow_ask_user: bool) -> String {
    let ambiguity_instructions = if allow_ask_user {
        "- If any high-impact ambiguity remains, use AskUser for clarifying questions whenever the ambiguity can be expressed as meaningful multiple-choice options, and do not output a final plan yet.\n\
- You may use AskUser multiple times across the planning session. Do not switch to plain-text clarification questions just because you already used AskUser earlier.\n"
    } else {
        "- Headless mode is active. Do not ask the user follow-up questions or request interactive clarification.\n\
- If any ambiguity remains, make the most reasonable assumptions you can, state them explicitly, and continue.\n"
    };
    format!(
        concat!(
            "{prompt}\n\n",
            "## Runtime instructions\n\n",
            "- You are starting this planning session before oat runs its automatic planning phase.\n",
            "- Stay in PHASE 1 and PHASE 2 until intent is stable.\n",
            "- During Plan Mode, the task/criteria tools are unavailable and the end-of-turn critic is disabled.\n",
            "{ambiguity_instructions}",
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
        ambiguity_instructions = ambiguity_instructions,
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
    planning_finalization_prompt_with_mode(description, successful_plans, failed_models, true)
}

pub fn planning_finalization_prompt_headless(
    description: &str,
    successful_plans: &[(PlanningJob, String)],
    failed_models: &[String],
) -> String {
    planning_finalization_prompt_with_mode(description, successful_plans, failed_models, false)
}

fn planning_finalization_prompt_with_mode(
    description: &str,
    successful_plans: &[(PlanningJob, String)],
    failed_models: &[String],
    allow_ask_user: bool,
) -> String {
    let successful_sections = successful_plans
        .iter()
        .enumerate()
        .map(|(index, (job, plan))| {
            format!(
                "Planner {}: model `{}` reasoning `{}`\n{}\n",
                index + 1,
                job.model_name,
                job.reasoning.as_str(),
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

    let ambiguity_instructions = if allow_ask_user {
        "- If a single remaining high-impact ambiguity still blocks a decision-complete plan, use AskUser when the clarification can be represented as meaningful multiple-choice options; only ask in plain text when that is genuinely not practical.\n"
    } else {
        "- Headless mode is active. Do not ask the user follow-up questions or request interactive clarification.\n\
- If a remaining ambiguity blocks a decision-complete plan, choose the most reasonable assumption, state it explicitly, and continue.\n"
    };

    format!(
        concat!(
            "{prompt}\n\n",
            "## Runtime instructions\n\n",
            "- oat has already completed PHASE 3 automatically.\n",
            "- Continue in PHASE 4.\n",
            "- During Plan Mode, the task/criteria tools are unavailable and the end-of-turn critic is disabled.\n",
            "- Review the planner outputs, resolve conflicts using your judgment, and produce the final plan.\n",
            "{ambiguity_instructions}",
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
        ambiguity_instructions = ambiguity_instructions,
        description = description.trim(),
        successful_sections = successful_sections,
        failure_note = failure_note,
    )
}

pub fn accepted_plan_implementation_prompt(accepted_plan: &str) -> String {
    format!(
        concat!(
            "You are no longer in Plan Mode. The plan has been accepted for implementation.\n",
            "Do not say that you still need a developer or system transition out of plan mode.\n",
            "Before making edits, call SetCurrentTask and register acceptance criteria that reflect the accepted plan. Refine them with the criterion tools as needed while implementing.\n",
            "Use the accepted plan below as the implementation brief, explore the workspace as needed, and begin implementation now.\n\n",
            "Accepted plan:\n",
            "{accepted_plan}\n"
        ),
        accepted_plan = accepted_plan.trim()
    )
}

pub fn parse_planning_reply(text: &str) -> PlanningReply {
    if let Some(markdown) =
        extract_tagged_block(text, PLANNING_READY_START_TAG, PLANNING_READY_END_TAG)
    {
        return PlanningReply::ReadyBrief(PlanningBrief { markdown });
    }

    if let Some(markdown) =
        extract_tagged_block(text, PROPOSED_PLAN_START_TAG, PROPOSED_PLAN_END_TAG)
    {
        return PlanningReply::ProposedPlan(ProposedPlan {
            raw_block: format!("{PROPOSED_PLAN_START_TAG}\n{markdown}\n{PROPOSED_PLAN_END_TAG}"),
            markdown,
        });
    }

    PlanningReply::ConversationText(text.to_string())
}

pub fn planning_reply_visible_text(text: &str) -> String {
    match parse_planning_reply(text) {
        PlanningReply::ConversationText(text) => text,
        PlanningReply::ReadyBrief(_) => String::new(),
        PlanningReply::ProposedPlan(plan) => plan.markdown,
    }
}

pub fn pending_plain_text_is_visible(text: &str) -> bool {
    if let Some(rest) = text.strip_prefix(PLANNING_READY_START_TAG) {
        if text.contains(PLANNING_READY_END_TAG) {
            return false;
        }
        return !rest.trim().is_empty();
    }

    if let Some(rest) = text.strip_prefix(PROPOSED_PLAN_START_TAG) {
        if text.contains(PROPOSED_PLAN_END_TAG) {
            return !planning_reply_visible_text(text).trim().is_empty();
        }
        return !rest.trim().is_empty();
    }

    !planning_reply_visible_text(text).trim().is_empty()
}

fn extract_tagged_block(text: &str, start_tag: &str, end_tag: &str) -> Option<String> {
    let start = text.find(start_tag)?;
    let content_start = start + start_tag.len();
    let end = text[content_start..].find(end_tag)? + content_start;
    Some(text[content_start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct PlanningAgentsWrapper {
        #[allow(dead_code)]
        planning: PlanningConfig,
    }

    #[test]
    fn planning_agent_parse_rejects_cross_family_reasoning_value() {
        let error = toml::from_str::<PlanningAgentsWrapper>(
            r#"
                [[planning.agents]]
                model_name = "kimi-k2.5"
                reasoning = "medium"
                "#,
        )
        .expect_err("planning config should fail to parse");

        assert!(
            error
                .to_string()
                .contains("reasoning `medium` is not supported by model `kimi-k2.5`")
        );
    }

    #[test]
    fn sanitize_removes_current_main_model_invalid_entries_and_duplicates() {
        let sanitized = sanitize_planning_agents(
            "gpt-5.4-mini",
            &[
                PlanningAgentConfig {
                    model_name: "gpt-5.4-mini".into(),
                    reasoning: ReasoningSetting::Gpt(ReasoningEffort::Medium),
                },
                PlanningAgentConfig {
                    model_name: "gpt-5.4".into(),
                    reasoning: ReasoningSetting::Gpt(ReasoningEffort::High),
                },
                PlanningAgentConfig {
                    model_name: "gpt-5.4".into(),
                    reasoning: ReasoningSetting::Gpt(ReasoningEffort::Low),
                },
                PlanningAgentConfig {
                    model_name: "unknown".into(),
                    reasoning: ReasoningSetting::Gpt(ReasoningEffort::Medium),
                },
            ],
        );

        assert_eq!(
            sanitized,
            vec![PlanningAgentConfig {
                model_name: "gpt-5.4".into(),
                reasoning: ReasoningSetting::Gpt(ReasoningEffort::High),
            }]
        );
    }

    #[test]
    fn planning_jobs_always_include_main_model_first() {
        let jobs = planning_jobs(
            "gpt-5.4-mini",
            ReasoningSetting::Gpt(ReasoningEffort::Medium),
            &[PlanningAgentConfig {
                model_name: "gpt-5.4".into(),
                reasoning: ReasoningSetting::Gpt(ReasoningEffort::High),
            }],
        );

        assert_eq!(
            jobs,
            vec![
                PlanningJob {
                    model_name: "gpt-5.4-mini".into(),
                    reasoning: ReasoningSetting::Gpt(ReasoningEffort::Medium),
                },
                PlanningJob {
                    model_name: "gpt-5.4".into(),
                    reasoning: ReasoningSetting::Gpt(ReasoningEffort::High),
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
        assert!(prompt.contains("task/criteria tools are unavailable"));
        assert!(prompt.contains("critic is disabled"));
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
                    reasoning: ReasoningSetting::Gpt(ReasoningEffort::Medium),
                },
                "High level description\n...".into(),
            )],
            &[],
        );

        assert!(prompt.contains("Continue in PHASE 4"));
        assert!(prompt.contains("Do a targeted read-only verification pass"));
        assert!(prompt.contains("Do not emit <planning_ready> again"));
        assert!(prompt.contains("task/criteria tools are unavailable"));
        assert!(prompt.contains("critic is disabled"));
        assert!(prompt.contains("use AskUser when the clarification can be represented"));
        assert!(prompt.contains("wrap the final answer in a <proposed_plan> block"));
    }

    #[test]
    fn accepted_plan_implementation_prompt_requires_re_registering_task_and_criteria() {
        let prompt =
            accepted_plan_implementation_prompt("<proposed_plan>\n# Ship it\n</proposed_plan>");

        assert!(prompt.contains("You are no longer in Plan Mode"));
        assert!(prompt.contains("Before making edits, call SetCurrentTask"));
        assert!(prompt.contains("Refine them with the criterion tools"));
    }

    #[test]
    fn parse_planning_reply_returns_inner_content_for_ready_brief() {
        let text = "<planning_ready>\n# Brief\n\nDetails\n</planning_ready>";
        assert_eq!(
            parse_planning_reply(text),
            PlanningReply::ReadyBrief(PlanningBrief {
                markdown: "# Brief\n\nDetails".into(),
            })
        );
    }

    #[test]
    fn parse_planning_reply_returns_proposed_plan() {
        let text = "<proposed_plan>\n# Plan\n\nDetails\n</proposed_plan>";
        assert_eq!(
            parse_planning_reply(text),
            PlanningReply::ProposedPlan(ProposedPlan {
                markdown: "# Plan\n\nDetails".into(),
                raw_block: "<proposed_plan>\n# Plan\n\nDetails\n</proposed_plan>".into(),
            })
        );
    }

    #[test]
    fn planning_reply_visible_text_hides_ready_brief() {
        assert_eq!(
            planning_reply_visible_text("<planning_ready>\n# Brief\n</planning_ready>"),
            ""
        );
    }

    #[test]
    fn pending_plain_text_is_visible_hides_complete_ready_brief() {
        assert!(!pending_plain_text_is_visible(
            "<planning_ready>\n# Brief\n</planning_ready>"
        ));
    }

    #[test]
    fn pending_plain_text_is_visible_shows_partial_ready_brief_content_only_after_text() {
        assert!(!pending_plain_text_is_visible("<planning_ready>\n"));
        assert!(pending_plain_text_is_visible(
            "<planning_ready>\n# Brief\nStill drafting"
        ));
    }

    #[test]
    fn pending_plain_text_is_visible_shows_proposed_plan_content_once_complete() {
        assert!(pending_plain_text_is_visible(
            "<proposed_plan>\n# Plan\nShip it\n</proposed_plan>"
        ));
    }
}
