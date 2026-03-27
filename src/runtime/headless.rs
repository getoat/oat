use std::error::Error;

use anyhow::anyhow;

use crate::{
    StartupOptions,
    app::StreamEvent,
    config::AppConfig,
    model_registry::{self, ModelProvider},
};

use super::bootstrap::bootstrap_headless;

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

        while let Some((reply_id, event)) = runtime.stream_rx.recv().await {
            if reply_id != 1 {
                continue;
            }

            match event {
                StreamEvent::TextDelta(delta) => output.push_str(&delta),
                StreamEvent::Finished { .. } => return Ok(output),
                StreamEvent::CompactionFinished { .. } => {}
                StreamEvent::Failed(error) => {
                    return Err(anyhow!("Request failed: {error}"));
                }
                StreamEvent::Commentary(_)
                | StreamEvent::ReasoningDelta(_)
                | StreamEvent::PlanningFinalizationStarted
                | StreamEvent::ToolCall { .. }
                | StreamEvent::ToolResult { .. }
                | StreamEvent::AskUserRequested { .. }
                | StreamEvent::WriteApprovalRequested { .. }
                | StreamEvent::ShellApprovalRequested { .. } => {}
            }
        }

        Err(anyhow!("Request ended before response completed."))
    });

    runtime.task.abort();
    runtime.stats.finalize_current_session()?;
    result.map_err(Into::into)
}
