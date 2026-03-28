use std::error::Error;

use anyhow::anyhow;

use crate::{
    StartupOptions,
    config::AppConfig,
    model_registry::{self, ModelProvider},
};

use super::{RuntimeEvent, bootstrap::bootstrap_headless};

pub(crate) fn run_headless(
    config: AppConfig,
    startup: StartupOptions,
    prompt: String,
) -> Result<String, Box<dyn Error>> {
    let headless_codex_without_auth = matches!(
        model_registry::find_model(&config.model.model_name).map(|model| model.provider),
        Some(ModelProvider::Codex)
    ) && !config
        .codex
        .as_ref()
        .is_some_and(|codex| codex.is_authenticated());
    if headless_codex_without_auth {
        return Err(anyhow!(
            "Headless Codex requests require authenticating from the TUI first with `/login`."
        )
        .into());
    }

    let mut runtime = bootstrap_headless(&config, startup, prompt)?;

    let result = runtime.runtime.block_on(async {
        let mut output = String::new();

        while let Some(runtime_event) = runtime.stream_rx.recv().await {
            match runtime_event {
                RuntimeEvent::MainReply { reply_id, event } => {
                    if reply_id != 1 {
                        continue;
                    }
                    match event {
                        crate::app::StreamEvent::SessionTitleGenerated(_) => {}
                        crate::app::StreamEvent::TextDelta(delta) => output.push_str(&delta),
                        crate::app::StreamEvent::TurnEnded { reason, .. } => {
                            if reason == crate::app::TurnEndReason::Completed {
                                return Ok(output);
                            }
                        }
                        crate::app::StreamEvent::CompactionFinished { .. } => {}
                        crate::app::StreamEvent::Failed(error) => {
                            return Err(anyhow!("Request failed: {error}"));
                        }
                        crate::app::StreamEvent::Commentary(_)
                        | crate::app::StreamEvent::ReasoningDelta(_)
                        | crate::app::StreamEvent::PlanningFinalizationStarted
                        | crate::app::StreamEvent::ToolCall { .. }
                        | crate::app::StreamEvent::ToolResult { .. }
                        | crate::app::StreamEvent::TodoSnapshot(_)
                        | crate::app::StreamEvent::AskUserRequested { .. }
                        | crate::app::StreamEvent::WriteApprovalRequested { .. }
                        | crate::app::StreamEvent::ShellApprovalRequested { .. } => {}
                    }
                }
                RuntimeEvent::SideChannel { .. } => {}
            }
        }

        Err(anyhow!("Request ended before response completed."))
    });

    runtime.task.abort();
    runtime.stats.finalize_current_session()?;
    result.map_err(Into::into)
}
