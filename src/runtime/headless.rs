use std::error::Error;

use anyhow::anyhow;

use crate::{StartupOptions, app::StreamEvent, config::AppConfig};

use super::bootstrap::bootstrap_headless;

pub(crate) fn run_headless(
    config: AppConfig,
    startup: StartupOptions,
    prompt: String,
) -> Result<String, Box<dyn Error>> {
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
