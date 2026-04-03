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
                        | crate::app::StreamEvent::HostedToolStarted { .. }
                        | crate::app::StreamEvent::HostedToolCompleted { .. }
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

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use anyhow::{Context, Result, anyhow};

    use crate::{
        StartupOptions,
        app::{HostedToolKind, StreamEvent, TurnEndReason},
        config::AppConfig,
        runtime::{RuntimeEvent, bootstrap::bootstrap_headless},
    };

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("timestamp")
            .as_nanos();
        env::temp_dir().join(format!(
            "oat-headless-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            let original = env::var_os(key);
            // SAFETY: test-only scoped environment mutation, restored on drop.
            unsafe { env::set_var(key, value) };
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                // SAFETY: restoring the process environment to its prior value.
                unsafe { env::set_var(self.key, value) };
            } else {
                // SAFETY: removing the temporary test override.
                unsafe { env::remove_var(self.key) };
            }
        }
    }

    fn write_live_search_config(temp_root: &Path) -> Result<PathBuf> {
        fs::create_dir_all(temp_root)
            .with_context(|| format!("failed to create {}", temp_root.display()))?;
        let source =
            fs::read_to_string("config.toml").context("failed to read repo config.toml")?;
        let mut value: toml::Value =
            toml::from_str(&source).context("failed to parse repo config.toml")?;
        let root = value
            .as_table_mut()
            .ok_or_else(|| anyhow!("expected config root table"))?;

        let memory = root
            .entry("memory")
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .ok_or_else(|| anyhow!("expected [memory] table"))?;
        memory.insert("enabled".into(), toml::Value::Boolean(false));

        let model = root
            .entry("model")
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .ok_or_else(|| anyhow!("expected [model] table"))?;
        model.insert(
            "model_name".into(),
            toml::Value::String("gpt-5.4-mini".into()),
        );
        model.insert("reasoning".into(), toml::Value::String("medium".into()));

        let tools = root
            .entry("tools")
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .ok_or_else(|| anyhow!("expected [tools] table"))?;
        let web_search = tools
            .entry("web_search")
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .ok_or_else(|| anyhow!("expected [tools.web_search] table"))?;
        web_search.insert("mode".into(), toml::Value::String("live".into()));

        let config_path = temp_root.join("config.toml");
        fs::write(
            &config_path,
            toml::to_string(&value).context("failed to serialize temp config")?,
        )
        .with_context(|| format!("failed to write {}", config_path.display()))?;

        Ok(config_path)
    }

    #[test]
    #[ignore = "manual live test requiring provider credentials and network access"]
    fn live_responses_search_emits_hosted_tool_events() -> Result<()> {
        let temp_root = unique_temp_dir("live-search");
        let config_path = write_live_search_config(&temp_root)?;
        let _home = EnvVarGuard::set("HOME", &temp_root);
        let config = AppConfig::load_from_path(&config_path)?;

        let mut runtime = bootstrap_headless(
            &config,
            StartupOptions::default(),
            concat!(
                "Use web search before answering. After the search completes, ",
                "reply with one short sentence confirming you finished searching."
            )
            .to_string(),
        )?;

        let outcome = runtime.runtime.block_on(async {
            let mut saw_search_start = false;
            let mut saw_search_complete = false;
            let mut final_text = String::new();
            let timeout = tokio::time::sleep(Duration::from_secs(90));
            tokio::pin!(timeout);

            loop {
                tokio::select! {
                    _ = &mut timeout => {
                        break Err(anyhow!(
                            "timed out waiting for live search event. partial response: {final_text}"
                        ));
                    }
                    maybe_event = runtime.stream_rx.recv() => {
                        match maybe_event {
                            Some(RuntimeEvent::MainReply { reply_id: 1, event }) => match event {
                                StreamEvent::HostedToolStarted {
                                    kind: HostedToolKind::WebSearch,
                                    detail,
                                    ..
                                } => {
                                    saw_search_start = true;
                                    let _ = detail;
                                }
                                StreamEvent::HostedToolCompleted {
                                    kind: HostedToolKind::WebSearch,
                                    ..
                                } => {
                                    saw_search_complete = true;
                                }
                                StreamEvent::TextDelta(delta) => final_text.push_str(&delta),
                                StreamEvent::TurnEnded {
                                    reason: TurnEndReason::Completed,
                                    ..
                                } => {
                                    break Ok((saw_search_start, saw_search_complete, final_text));
                                }
                                StreamEvent::Failed(error) => {
                                    break Err(anyhow!("request failed during live search smoke: {error}"));
                                }
                                _ => {}
                            },
                            Some(_) => {}
                            None => {
                                break Err(anyhow!(
                                    "event stream ended before completion. partial response: {final_text}"
                                ));
                            }
                        }
                    }
                }
            }
        });

        runtime.task.abort();
        runtime
            .stats
            .finalize_current_session()
            .context("failed to finalize stats after live search smoke")?;

        let (saw_search_start, saw_search_complete, final_text) = outcome?;
        assert!(
            saw_search_start,
            "no hosted web search start event observed. final response: {final_text}"
        );
        assert!(
            saw_search_complete,
            "no hosted web search completion event observed. final response: {final_text}"
        );
        assert!(
            !final_text.trim().is_empty(),
            "final response was empty after hosted web search. final response: {final_text}"
        );

        Ok(())
    }
}
