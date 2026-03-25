use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use tokio::time::sleep;

use crate::{
    app::ApprovalMode,
    config::AppConfig,
    subagents::{SubagentActivityKind, SubagentManager, SubagentSpawnRequest, SubagentStatus},
};

use super::protocol::{PlanningJob, planner_prompt, planning_finalization_prompt, planning_jobs};

pub type PlanningSynthesisFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
pub type PlanningSynthesizer = Arc<
    dyn Fn(String, Vec<rig::completion::Message>, Option<String>) -> PlanningSynthesisFuture
        + Send
        + Sync,
>;
pub type PlanningFailureHandler = Arc<dyn Fn(String) + Send + Sync>;
pub type PlanningFinalizationHandler = Arc<dyn Fn() + Send + Sync>;

pub async fn run_planning_workflow(
    reply_id: u64,
    description: String,
    history: Vec<rig::completion::Message>,
    history_model_name: Option<String>,
    config: AppConfig,
    subagents: SubagentManager,
    on_finalization_started: PlanningFinalizationHandler,
    on_failure: PlanningFailureHandler,
    synthesize: PlanningSynthesizer,
) {
    let jobs = planning_jobs(
        &config.azure.model_name,
        config.azure.reasoning_effort,
        &config.planning.agents,
    );
    let mut successful_plans = Vec::new();
    let mut failed_models = Vec::new();

    for batch in jobs.chunks(config.subagents.max_concurrent.max(1)) {
        let batch_ids =
            spawn_planning_batch(&subagents, &config, &description, batch, &mut failed_models)
                .await;
        let (successful, failed) = collect_planning_batch_results(&subagents, batch_ids).await;
        successful_plans.extend(successful);
        failed_models.extend(failed);
    }

    if successful_plans.is_empty() {
        let message = if failed_models.is_empty() {
            "Planning failed before any planner produced output.".to_string()
        } else {
            format!(
                "Planning failed. No planner completed successfully. Failed planners: {}.",
                failed_models.join(", ")
            )
        };
        on_failure(message);
        return;
    }

    let _ = reply_id;
    on_finalization_started();
    let synth_prompt =
        planning_finalization_prompt(&description, &successful_plans, &failed_models);
    if let Err(error) = synthesize(synth_prompt, history, history_model_name).await {
        on_failure(format!("Planning synthesis failed: {error}"));
    }
}

async fn spawn_planning_batch(
    subagents: &SubagentManager,
    config: &AppConfig,
    description: &str,
    batch: &[PlanningJob],
    failed_models: &mut Vec<String>,
) -> Vec<(PlanningJob, String)> {
    let mut spawned = Vec::new();

    for job in batch {
        match spawn_planning_subagent(subagents, config, description, job.clone()).await {
            Ok(id) => spawned.push((job.clone(), id)),
            Err(_) => failed_models.push(job.model_name.clone()),
        }
    }

    spawned
}

async fn spawn_planning_subagent(
    subagents: &SubagentManager,
    config: &AppConfig,
    description: &str,
    job: PlanningJob,
) -> anyhow::Result<String> {
    let mut planner_config = config.clone();
    planner_config.azure.model_name = job.model_name.clone();
    planner_config.azure.reasoning_effort = job.reasoning_effort;
    let snapshot = subagents
        .spawn(SubagentSpawnRequest {
            prompt: planner_prompt(description),
            access_mode: crate::app::AccessMode::ReadOnly,
            activity_kind: SubagentActivityKind::Planning {
                model_name: job.model_name.clone(),
            },
            model_name_override: Some(job.model_name.clone()),
            config: planner_config,
            approval_mode: ApprovalMode::Manual,
        })
        .await?;

    Ok(snapshot.id)
}

async fn collect_planning_batch_results(
    subagents: &SubagentManager,
    batch_ids: Vec<(PlanningJob, String)>,
) -> (Vec<(PlanningJob, String)>, Vec<String>) {
    let mut pending = batch_ids;
    let mut successful = Vec::new();
    let mut failed = Vec::new();

    while !pending.is_empty() {
        let mut next_pending = Vec::new();

        for (job, id) in pending {
            match subagents.inspect(&id) {
                Ok(snapshot) => match snapshot.status {
                    SubagentStatus::Running => next_pending.push((job, id)),
                    SubagentStatus::Completed => {
                        if let Some(output) = snapshot.output {
                            successful.push((job, output));
                        } else {
                            failed.push(job.model_name);
                        }
                    }
                    SubagentStatus::Failed | SubagentStatus::Cancelled => {
                        failed.push(job.model_name)
                    }
                },
                Err(_) => failed.push(job.model_name),
            }
        }

        if next_pending.is_empty() {
            break;
        }

        pending = next_pending;
        sleep(Duration::from_millis(100)).await;
    }

    (successful, failed)
}
