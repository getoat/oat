use std::{fs, path::Path};

use anyhow::{Context, Result};

use crate::features::planning::{PlanningAgentConfig, sanitize_planning_agents};

use super::ReasoningSetting;

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
    let azure = root
        .entry("azure")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .context("config azure value must be a TOML table")?;
    if let Some(model_name) = model_name {
        azure.insert(
            "model_name".into(),
            toml::Value::String(model_name.to_string()),
        );
    }
    if let Some(reasoning) = reasoning {
        azure.insert(
            "reasoning".into(),
            toml::Value::String(reasoning.as_str().to_string()),
        );
        azure.remove("reasoning_effort");
    }
    let current_main_model = azure
        .get("model_name")
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(planning_agents) = planning_agents {
        let agents = sanitize_planning_agents(&current_main_model, planning_agents);
        let serialized = agents
            .into_iter()
            .map(toml::Value::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to serialize planning agents")?;
        let _ = azure;
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
