use std::sync::Arc;

use anyhow::Result;
use tokio::sync::oneshot;

use crate::{
    agent::AgentContext,
    app::StreamEvent,
    llm::{CompletionCapture, LlmService, PromptRunResult, WriteApprovalController},
};

use super::{
    SUBAGENT_FAILURE_LOG_DIR_RELATIVE_PATH, SUBAGENT_FAILURE_LOG_SCHEMA_VERSION,
    SubagentFailureLog, SubagentManager, SubagentSnapshot, SubagentSpawnRequest, SubagentUiEvent,
    failures::{SubagentExecutionFailure, default_subagent_failure_log_dir, unix_timestamp_ms},
    normalize_subagent_failure, persist_subagent_failure_log,
};

impl SubagentManager {
    pub async fn spawn(&self, request: SubagentSpawnRequest) -> Result<SubagentSnapshot> {
        let id = self.register_running(
            request.access_mode,
            request
                .model_name_override
                .clone()
                .unwrap_or_else(|| request.config.model.model_name.clone()),
        )?;

        let _ = self.inner.ui_tx.send(SubagentUiEvent::Spawned {
            id: id.clone(),
            access_mode: request.access_mode,
            activity_kind: request.activity_kind.clone(),
        });

        let manager = self.clone();
        let spawned_id = id.clone();
        let request_for_failure = request.clone();
        let (start_tx, start_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            if start_rx.await.is_err() {
                return;
            }
            let result = manager
                .run_spawned_subagent(spawned_id.clone(), request)
                .await;

            match result {
                Ok(output) => manager.mark_completed(&spawned_id, output),
                Err(error) => manager.fail_subagent(&spawned_id, &request_for_failure, error),
            }
        });
        self.insert_task_handle(&id, handle);
        let _ = start_tx.send(());

        self.inspect(&id)
    }

    async fn run_spawned_subagent(
        &self,
        id: String,
        request: SubagentSpawnRequest,
    ) -> std::result::Result<String, SubagentExecutionFailure> {
        let context =
            AgentContext::subagent(request.access_mode, request.model_name_override.clone());
        let service = LlmService::from_config(
            &request.config,
            context,
            WriteApprovalController::new(request.approval_mode),
            None,
            Some(self.clone()),
        )
        .map_err(|error| SubagentExecutionFailure {
            raw_error: error.to_string(),
            failing_request: None,
        })?;
        let stats_hook = self.inner.stats.hook_for_model(
            request
                .model_name_override
                .clone()
                .unwrap_or_else(|| request.config.model.model_name.clone()),
        );
        let subagent_id = id.clone();
        let callback_manager = self.clone();
        let callback = Arc::new(move |_reply_id: u64, event: StreamEvent| {
            callback_manager.handle_stream_event(&subagent_id, event);
            true
        });
        let capture = CompletionCapture::new();

        let result = service
            .run_prompt(
                1,
                request.prompt,
                Vec::new(),
                None,
                stats_hook,
                Some(capture.clone()),
                callback,
            )
            .await;

        match result {
            Ok(PromptRunResult { output, .. }) => Ok(output),
            Err(error) => Err(SubagentExecutionFailure {
                raw_error: error.to_string(),
                failing_request: capture.snapshot(),
            }),
        }
    }

    fn fail_subagent(
        &self,
        id: &str,
        request: &SubagentSpawnRequest,
        failure: SubagentExecutionFailure,
    ) {
        let normalized_error = normalize_subagent_failure(&failure.raw_error);
        {
            let mut state = self.inner.state.lock().expect("subagent state lock");
            let Some(record) = state.records.get_mut(id) else {
                return;
            };
            if record.status != super::SubagentStatus::Running {
                return;
            }

            record.status = super::SubagentStatus::Failed;
            record.output = None;
            record.error = Some(normalized_error.clone());
            record.failure_log_path = None;
            record.last_activity_at = tokio::time::Instant::now();
            state.tasks.remove(id);
            self.bump_generation(&mut state);
        }

        let failure_log_path = persist_subagent_failure_log(
            default_subagent_failure_log_dir(SUBAGENT_FAILURE_LOG_DIR_RELATIVE_PATH).as_deref(),
            &SubagentFailureLog {
                schema_version: SUBAGENT_FAILURE_LOG_SCHEMA_VERSION,
                subagent_id: id.to_string(),
                failed_at_unix_ms: unix_timestamp_ms(),
                model_name: request
                    .model_name_override
                    .clone()
                    .unwrap_or_else(|| request.config.model.model_name.clone()),
                access_mode: request.access_mode.label().to_ascii_lowercase(),
                prompt: request.prompt.clone(),
                raw_error: failure.raw_error,
                normalized_error: normalized_error.clone(),
                failing_request: failure.failing_request,
            },
        )
        .ok()
        .flatten()
        .map(|path| path.display().to_string());

        {
            let mut state = self.inner.state.lock().expect("subagent state lock");
            if let Some(record) = state.records.get_mut(id)
                && record.status == super::SubagentStatus::Failed
            {
                record.failure_log_path = failure_log_path.clone();
            }
        }
        let _ = self.inner.ui_tx.send(SubagentUiEvent::Failed {
            id: id.to_string(),
            error: normalized_error,
            log_path: failure_log_path,
        });
    }
}
