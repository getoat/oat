use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{config::ReasoningEffort, model_registry};

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

pub fn planner_prompt(description: &str) -> String {
    format!(
        concat!(
            "You are producing a planning-only response for a coding task.\n\n",
            "Task description:\n",
            "{description}\n\n",
            "Before writing the plan, use the available read-only tools to explore the codebase deeply enough that the implementation details are already known.\n",
            "Resolve concrete specifics such as file locations, symbols, existing patterns, interfaces, and likely touchpoints before finalizing the plan.\n",
            "Do not leave discovery as future work.\n",
            "Return a plan only. Do not claim to have made changes. Keep the high-level description to a few sentences.\n",
            "Use exactly these sections and headings in this order:\n",
            "1. High level description\n",
            "2. Implementation plan\n",
            "3. Assumptions\n",
            "4. Acceptance criteria\n",
            "5. Out of scope / caveats / bewares\n\n",
            "Requirements:\n",
            "- Focus on concrete implementation steps.\n",
            "- Use bullet points for the implementation plan.\n",
            "- Write the plan as a description of intended work, not as instructions to the implementer.\n",
            "- Prefer wording like 'Will update...', 'Would add...', 'Will not change...', and 'Out of scope includes...'.\n",
            "- Avoid imperative phrasing like 'Do not change...', 'Add...', 'Use...', or 'Implement...'.\n",
            "- The implementation plan must be decision-complete and should not contain steps like 'find', 'investigate', 'look up', 'nail down', or 'determine' implementation specifics later.\n",
            "- If a detail is important to implementation, discover it now and write the concrete result into the plan.\n",
            "- Include minimal example code only if it is necessary to disambiguate the plan.\n",
            "- Keep caveats specific and practical.\n",
            "- Do not include any prose before the first heading.\n"
        ),
        description = description,
    )
}

pub fn synthesis_prompt(
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
            "Synthesize a final planning-only answer for this task.\n\n",
            "Task description:\n",
            "{description}\n\n",
            "Successful planner outputs:\n",
            "{successful_sections}\n",
            "Planner failures:\n",
            "{failure_note}\n\n",
            "After reading the planner outputs, do a targeted read-only discovery pass to verify the references you need in order to understand and reconcile the plans.\n",
            "Validate concrete things like file paths, symbol names, existing abstractions, and interface shapes when they matter to the final plan.\n",
            "Do enough discovery to ground the merged plan, but do not redo the full exploration effort of each planner and do not delegate.\n",
            "Produce a single merged plan.\n",
            "Prefer ideas that appear in multiple planner outputs.\n",
            "Retain unique steps only when they are concrete and improve the implementation.\n",
            "Do not include discovery tasks as plan steps. The final plan should read as if the relevant specifics have already been established.\n",
            "Write the final plan as a description of intended work, not as instructions.\n",
            "Prefer phrasing like 'Will update...', 'Would add...', 'Will keep...', and 'Will not change...'.\n",
            "Avoid imperative phrasing such as 'Do not change...', 'Add...', 'Use...', or 'Implement...'.\n",
            "Do not mention vote counts or describe the synthesis process.\n",
            "Keep the high-level description to a few sentences.\n",
            "Use exactly these sections and headings in this order:\n",
            "1. High level description\n",
            "2. Implementation plan\n",
            "3. Assumptions\n",
            "4. Acceptance criteria\n",
            "5. Out of scope / caveats / bewares\n"
        ),
        description = description,
        successful_sections = successful_sections,
        failure_note = failure_note,
    )
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
    fn planner_prompt_requires_discovery_before_planning() {
        let prompt = planner_prompt("Add a planning workflow");

        assert!(
            prompt.contains("use the available read-only tools to explore the codebase deeply")
        );
        assert!(prompt.contains("Do not leave discovery as future work"));
        assert!(prompt.contains("should not contain steps like 'find'"));
        assert!(prompt.contains("Write the plan as a description of intended work"));
    }

    #[test]
    fn synthesis_prompt_requires_targeted_validation_without_redoing_planner_work() {
        let prompt = synthesis_prompt(
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

        assert!(prompt.contains("do a targeted read-only discovery pass"));
        assert!(prompt.contains("do not redo the full exploration effort of each planner"));
        assert!(prompt.contains("Do not include discovery tasks as plan steps"));
        assert!(prompt.contains("Write the final plan as a description of intended work"));
    }
}
