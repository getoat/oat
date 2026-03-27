use std::{fs, path::Path};

use anyhow::{Context, Result};

use crate::features::planning::{PlanningAgentConfig, sanitize_planning_agents};

use super::{CodexConfig, ReasoningSetting};

pub(super) fn write_config_updates_at_path(
    path: &Path,
    model_name: Option<&str>,
    reasoning: Option<ReasoningSetting>,
    planning_agents: Option<&[PlanningAgentConfig]>,
    safety_model_name: Option<&str>,
    safety_reasoning: Option<ReasoningSetting>,
) -> Result<()> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let mut value: toml::Value = if raw.trim().is_empty() {
        toml::Value::Table(Default::default())
    } else {
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?
    };

    let root = value
        .as_table_mut()
        .context("config root must be a TOML table")?;
    let model = root
        .entry("model")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .context("config model value must be a TOML table")?;
    if let Some(model_name) = model_name {
        model.insert(
            "model_name".into(),
            toml::Value::String(model_name.to_string()),
        );
    }
    if let Some(reasoning) = reasoning {
        model.insert(
            "reasoning".into(),
            toml::Value::String(reasoning.as_str().to_string()),
        );
        model.remove("reasoning_effort");
    }
    let current_main_model = root
        .get("model")
        .and_then(toml::Value::as_table)
        .and_then(|model| model.get("model_name"))
        .and_then(toml::Value::as_str)
        .or_else(|| {
            root.get("azure")
                .and_then(toml::Value::as_table)
                .and_then(|azure| azure.get("model_name"))
                .and_then(toml::Value::as_str)
        })
        .unwrap_or_default()
        .to_string();
    if let Some(planning_agents) = planning_agents {
        let agents = sanitize_planning_agents(&current_main_model, planning_agents);
        let serialized = agents
            .into_iter()
            .map(toml::Value::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to serialize planning agents")?;
        let planning = root
            .entry("planning")
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .context("config planning value must be a TOML table")?;
        planning.insert("agents".into(), toml::Value::Array(serialized));
    }
    if safety_model_name.is_some() || safety_reasoning.is_some() {
        let safety = root
            .entry("safety")
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .context("config safety value must be a TOML table")?;
        if let Some(model_name) = safety_model_name {
            safety.insert(
                "model_name".into(),
                toml::Value::String(model_name.to_string()),
            );
        }
        if let Some(reasoning) = safety_reasoning {
            safety.insert(
                "reasoning".into(),
                toml::Value::String(reasoning.as_str().to_string()),
            );
            safety.remove("reasoning_effort");
        }
    }

    let formatted = toml::to_string_pretty(&value)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, formatted).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub(super) fn write_codex_auth_updates_at_path(
    path: &Path,
    codex: Option<&CodexConfig>,
) -> Result<()> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let mut value: toml::Value = if raw.trim().is_empty() {
        toml::Value::Table(Default::default())
    } else {
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?
    };

    let root = value
        .as_table_mut()
        .context("config root must be a TOML table")?;
    match codex {
        Some(codex) => {
            let codex_table = root
                .entry("codex")
                .or_insert_with(|| toml::Value::Table(Default::default()))
                .as_table_mut()
                .context("config codex value must be a TOML table")?;
            update_optional_string(
                codex_table,
                "auth_mode",
                codex.auth_mode.map(|mode| match mode {
                    super::CodexAuthMode::ApiKey => "api_key".to_string(),
                    super::CodexAuthMode::Chatgpt => "chatgpt".to_string(),
                    super::CodexAuthMode::ChatgptAuthTokens => "chatgpt_auth_tokens".to_string(),
                }),
            );
            update_optional_string(codex_table, "OPENAI_API_KEY", codex.openai_api_key.clone());
            update_optional_string(codex_table, "access_token", codex.access_token.clone());
            update_optional_string(codex_table, "refresh_token", codex.refresh_token.clone());
            update_optional_string(codex_table, "id_token", codex.id_token.clone());
            update_optional_string(codex_table, "account_id", codex.account_id.clone());
            match codex.last_refresh {
                Some(last_refresh) => {
                    codex_table.insert(
                        "last_refresh".into(),
                        toml::Value::String(last_refresh.to_rfc3339()),
                    );
                }
                None => {
                    codex_table.remove("last_refresh");
                }
            }
        }
        None => {
            if let Some(codex_table) = root.get_mut("codex").and_then(toml::Value::as_table_mut) {
                for key in [
                    "auth_mode",
                    "OPENAI_API_KEY",
                    "access_token",
                    "refresh_token",
                    "id_token",
                    "account_id",
                    "last_refresh",
                ] {
                    codex_table.remove(key);
                }
            }
        }
    }

    let formatted = toml::to_string_pretty(&value)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, formatted).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn update_optional_string(table: &mut toml::value::Table, key: &str, value: Option<String>) {
    match value {
        Some(value) => {
            table.insert(key.into(), toml::Value::String(value));
        }
        None => {
            table.remove(key);
        }
    }
}
