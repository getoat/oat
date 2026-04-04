use std::{future::Future, pin::Pin, sync::Arc};

use crate::{
    config::AppConfig,
    llm::{ShellApprovalController, WriteApprovalController},
    subagents::{SubagentActivityKind, SubagentManager, SubagentSpawnRequest, SubagentStatus},
    web::WebService,
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
    write_approvals: WriteApprovalController,
    shell_approvals: ShellApprovalController,
    web: WebService,
    on_finalization_started: PlanningFinalizationHandler,
    on_failure: PlanningFailureHandler,
    synthesize: PlanningSynthesizer,
) {
    let jobs = planning_jobs(
        &config.model.model_name,
        config.model.reasoning,
        &config.planning.agents,
    );
    let mut successful_plans = Vec::new();
    let mut failed_models = Vec::new();

    for batch in jobs.chunks(config.subagents.max_concurrent.max(1)) {
        let batch_ids = spawn_planning_batch(
            &subagents,
            &config,
            &description,
            batch,
            write_approvals.clone(),
            shell_approvals.clone(),
            web.clone(),
            &mut failed_models,
        )
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
    write_approvals: WriteApprovalController,
    shell_approvals: ShellApprovalController,
    web: WebService,
    failed_models: &mut Vec<String>,
) -> Vec<(PlanningJob, String)> {
    let mut spawned = Vec::new();

    for job in batch {
        match spawn_planning_subagent(
            subagents,
            config,
            description,
            job.clone(),
            write_approvals.clone(),
            shell_approvals.clone(),
            web.clone(),
        )
        .await
        {
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
    write_approvals: WriteApprovalController,
    shell_approvals: ShellApprovalController,
    web: WebService,
) -> anyhow::Result<String> {
    let mut planner_config = config.clone();
    planner_config.model.model_name = job.model_name.clone();
    planner_config.model.reasoning = job.reasoning;
    let snapshot = subagents
        .spawn(SubagentSpawnRequest {
            prompt: planner_prompt(description),
            access_mode: crate::app::AccessMode::ReadOnly,
            activity_kind: SubagentActivityKind::Planning {
                model_name: job.model_name.clone(),
            },
            model_name_override: Some(job.model_name.clone()),
            config: planner_config,
            write_approvals,
            shell_approvals,
            web,
        })
        .await?;

    Ok(snapshot.id)
}

async fn collect_planning_batch_results(
    subagents: &SubagentManager,
    batch_ids: Vec<(PlanningJob, String)>,
) -> (Vec<(PlanningJob, String)>, Vec<String>) {
    let ids = batch_ids
        .iter()
        .map(|(_, id)| id.clone())
        .collect::<Vec<_>>();
    let wait_result = match subagents.wait_all(&ids, None).await {
        Ok(result) => result,
        Err(_) => {
            return (
                Vec::new(),
                batch_ids
                    .into_iter()
                    .map(|(job, _)| job.model_name)
                    .collect::<Vec<_>>(),
            );
        }
    };
    let snapshots = wait_result
        .subagents
        .into_iter()
        .map(|snapshot| (snapshot.id.clone(), snapshot))
        .collect::<std::collections::HashMap<_, _>>();
    let mut successful = Vec::new();
    let mut failed = Vec::new();

    for (job, id) in batch_ids {
        match snapshots.get(&id) {
            Some(snapshot) => match snapshot.status {
                SubagentStatus::Completed => {
                    if let Some(output) = snapshot.output.clone() {
                        successful.push((job, output));
                    } else {
                        failed.push(job.model_name);
                    }
                }
                SubagentStatus::Failed | SubagentStatus::Cancelled | SubagentStatus::Running => {
                    failed.push(job.model_name)
                }
            },
            None => failed.push(job.model_name),
        }
    }

    (successful, failed)
}
