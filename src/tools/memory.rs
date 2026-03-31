use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use serde_json::json;

use crate::memory::MemoryService;

use super::common::ToolExecError;

pub(crate) const SEARCH_MEMORIES_TOOL_NAME: &str = "SearchMemories";
pub(crate) const GET_MEMORY_TOOL_NAME: &str = "GetMemory";
const DEFAULT_MEMORY_SEARCH_LIMIT: usize = 5;
const MAX_MEMORY_SEARCH_LIMIT: usize = 10;

#[derive(Clone)]
pub(crate) struct SearchMemoriesTool {
    memory: MemoryService,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchMemoriesArgs {
    query: String,
    #[serde(default)]
    include_candidates: bool,
    #[serde(default = "default_memory_search_limit")]
    limit: usize,
}

impl SearchMemoriesTool {
    pub(crate) fn new(memory: MemoryService) -> Self {
        Self { memory }
    }
}

impl Tool for SearchMemoriesTool {
    const NAME: &'static str = SEARCH_MEMORIES_TOOL_NAME;
    type Error = ToolExecError;
    type Args = SearchMemoriesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Search long-term memory for relevant repo, module, or user-preference context. Use this when prior work, previous decisions, or stable user preferences may help answer the current request.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keywords or a short natural-language query describing the memory to find."
                    },
                    "include_candidates": {
                        "type": "boolean",
                        "description": "Set true to include lower-confidence candidate memories awaiting review."
                    },
                    "limit": {
                        "type": "integer",
                        "description": format!("Maximum number of results to return. Must be between 1 and {MAX_MEMORY_SEARCH_LIMIT}.")
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let query = args.query.trim();
        if query.is_empty() {
            return Err(ToolExecError::new("query must not be empty"));
        }
        if args.limit == 0 || args.limit > MAX_MEMORY_SEARCH_LIMIT {
            return Err(ToolExecError::new(format!(
                "limit must be between 1 and {MAX_MEMORY_SEARCH_LIMIT}"
            )));
        }

        self.memory
            .search_text(query, args.include_candidates, args.limit)
            .map_err(|error| ToolExecError::new(error.to_string()))
    }
}

#[derive(Clone)]
pub(crate) struct GetMemoryTool {
    memory: MemoryService,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GetMemoryArgs {
    id: String,
}

impl GetMemoryTool {
    pub(crate) fn new(memory: MemoryService) -> Self {
        Self { memory }
    }
}

impl Tool for GetMemoryTool {
    const NAME: &'static str = GET_MEMORY_TOOL_NAME;
    type Error = ToolExecError;
    type Args = GetMemoryArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Fetch the full text of a specific long-term memory record by id."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Full memory id to retrieve."
                    }
                },
                "required": ["id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let id = args.id.trim();
        if id.is_empty() {
            return Err(ToolExecError::new("id must not be empty"));
        }

        self.memory
            .get_text(id)
            .map_err(|error| ToolExecError::new(error.to_string()))
    }
}

const fn default_memory_search_limit() -> usize {
    DEFAULT_MEMORY_SEARCH_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::MemoryConfig,
        memory::{
            MemoryEvidenceRef, MemoryKind, MemoryRecord, MemoryScope, MemorySource, MemoryStatus,
        },
    };
    use chrono::Utc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use uuid::Uuid;

    fn test_memory_service() -> MemoryService {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("timestamp")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "oat-memory-tool-test-{}-{nanos}",
            std::process::id()
        ));
        let store = std::env::temp_dir().join(format!(
            "oat-memory-tool-store-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join(".git")).expect("create fake repo root");
        let service = MemoryService::with_storage_dir(MemoryConfig::default(), root.clone(), store)
            .expect("memory");
        service
            .insert_test_record(MemoryRecord {
                id: Uuid::now_v7().to_string(),
                scope: MemoryScope::Global,
                repo_fingerprint: None,
                subject_key: "global:terse-summaries".into(),
                kind: MemoryKind::Preference,
                title: "Summary style".into(),
                summary: "Prefer terse summaries.".into(),
                details: None,
                tags: vec!["terse".into(), "summary".into()],
                evidence: vec![MemoryEvidenceRef {
                    session_id: Some("session-1".into()),
                    prompt: Some("Please prefer terse summaries".into()),
                    files: Vec::new(),
                }],
                source: MemorySource::ExplicitUser,
                confidence: 0.95,
                status: MemoryStatus::Active,
                supersedes: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .expect("seed memory");
        service
    }

    #[tokio::test]
    async fn search_memories_returns_matches() {
        let tool = SearchMemoriesTool::new(test_memory_service());
        let output = tool
            .call(SearchMemoriesArgs {
                query: "terse".into(),
                include_candidates: false,
                limit: 5,
            })
            .await
            .expect("search succeeds");

        assert!(output.contains("Memory results"));
        assert!(output.contains("User preference"));
    }

    #[tokio::test]
    async fn get_memory_requires_non_empty_id() {
        let tool = GetMemoryTool::new(test_memory_service());
        let error = tool
            .call(GetMemoryArgs { id: "  ".into() })
            .await
            .expect_err("empty ids must fail");

        assert!(error.to_string().contains("must not be empty"));
    }
}
