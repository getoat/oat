use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const HISTORY_PATH_RELATIVE: &str = ".config/oat/command_history.json";

#[derive(Debug, Clone)]
pub(crate) struct CommandHistoryStore {
    path: Option<PathBuf>,
    limit: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct PersistedCommandHistory {
    entries: Vec<String>,
}

impl CommandHistoryStore {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            path: default_history_path(),
            limit,
        }
    }

    #[cfg(test)]
    fn with_path(path: impl Into<PathBuf>, limit: usize) -> Self {
        Self {
            path: Some(path.into()),
            limit,
        }
    }

    pub(crate) fn load(&self) -> Result<Vec<String>> {
        let Some(path) = self.path.as_deref() else {
            return Ok(Vec::new());
        };

        if !path.exists() {
            return Ok(Vec::new());
        }

        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Vec::new());
        }

        let persisted: PersistedCommandHistory = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(trim_entries(persisted.entries, self.limit))
    }

    pub(crate) fn save(&self, entries: &[String]) -> Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let persisted = PersistedCommandHistory {
            entries: trim_entries(entries.to_vec(), self.limit),
        };
        let raw = serde_json::to_string_pretty(&persisted)
            .with_context(|| format!("failed to serialize {}", path.display()))?;
        fs::write(path, raw).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }
}

fn trim_entries(mut entries: Vec<String>, limit: usize) -> Vec<String> {
    entries.retain(|entry| !entry.trim().is_empty());
    if entries.len() > limit {
        entries.drain(..entries.len() - limit);
    }
    entries
}

fn default_history_path() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(HISTORY_PATH_RELATIVE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "oat-command-history-{name}-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("timestamp")
                .as_nanos()
        ))
    }

    #[test]
    fn load_returns_empty_when_history_file_is_missing() {
        let path = unique_temp_path("missing");
        let store = CommandHistoryStore::with_path(&path, 20);

        assert_eq!(store.load().expect("history loads"), Vec::<String>::new());
    }

    #[test]
    fn save_and_load_round_trip_with_limit() {
        let path = unique_temp_path("round-trip");
        let store = CommandHistoryStore::with_path(&path, 3);
        let entries = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
            "fourth".to_string(),
        ];

        store.save(&entries).expect("history saves");

        let loaded = store.load().expect("history loads");
        assert_eq!(loaded, vec!["second", "third", "fourth"]);

        fs::remove_file(path).expect("remove temp history");
    }

    #[test]
    fn save_discards_blank_entries() {
        let path = unique_temp_path("blanks");
        let store = CommandHistoryStore::with_path(&path, 5);
        let entries = vec![
            "".to_string(),
            "   ".to_string(),
            "keep".to_string(),
            "\n".to_string(),
        ];

        store.save(&entries).expect("history saves");

        let loaded = store.load().expect("history loads");
        assert_eq!(loaded, vec!["keep"]);

        fs::remove_file(path).expect("remove temp history");
    }
}
