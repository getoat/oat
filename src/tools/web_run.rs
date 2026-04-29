use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::web::{OpenPageChunk, ResponseLength, SearchResults, WebService};

use super::common::ToolExecError;

#[derive(Clone)]
pub struct WebRunTool {
    web: WebService,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseLengthArg {
    Short,
    Medium,
    Long,
}

impl Default for ResponseLengthArg {
    fn default() -> Self {
        Self::Medium
    }
}

impl From<ResponseLengthArg> for ResponseLength {
    fn from(value: ResponseLengthArg) -> Self {
        match value {
            ResponseLengthArg::Short => ResponseLength::Short,
            ResponseLengthArg::Medium => ResponseLength::Medium,
            ResponseLengthArg::Long => ResponseLength::Long,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchQueryInvocation {
    pub q: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenInvocation {
    pub ref_id: String,
    #[serde(default)]
    pub lineno: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct FindInvocation {
    pub ref_id: String,
    pub pattern: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct WebRunArgs {
    #[serde(default)]
    pub search_query: Vec<SearchQueryInvocation>,
    #[serde(default)]
    pub open: Vec<OpenInvocation>,
    #[serde(default)]
    pub find: Vec<FindInvocation>,
    #[serde(default)]
    pub response_length: ResponseLengthArg,
}

#[derive(Debug, Serialize)]
struct WebRunOutput {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    search_query: Vec<SearchResults>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    open: Vec<OpenPageChunk>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    find: Vec<crate::web::FindResults>,
}

impl WebRunTool {
    pub fn new(web: WebService) -> Self {
        Self { web }
    }
}

impl Tool for WebRunTool {
    const NAME: &'static str = "WebRun";
    type Error = ToolExecError;
    type Args = WebRunArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Local web browsing tool equivalent to web.run. Use search_query when you need to discover URLs from a search engine. Use open when you already know a URL or have a ref_id from search_query; open returns a cached, line-numbered page chunk and supports lineno for continuation without refetching. Use find to search within a previously opened page or a URL. Prefer this tool over hosted web_search when you need reliable web retrieval.".to_string(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "search_query": {
                        "type": "array",
                        "description": "Search the web for one or more queries.",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["q"],
                            "properties": {
                                "q": {
                                    "type": "string",
                                    "description": "Search query text."
                                }
                            }
                        }
                    },
                    "open": {
                        "type": "array",
                        "description": "Open a known URL or a ref_id from search_query and return a cached chunk of its content.",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["ref_id"],
                            "properties": {
                                "ref_id": {
                                    "type": "string",
                                    "description": "A search result ref_id or a fully-qualified HTTP/HTTPS URL."
                                },
                                "lineno": {
                                    "type": "integer",
                                    "minimum": 1,
                                    "description": "1-based line number to start from. Omit for the beginning of the page."
                                }
                            }
                        }
                    },
                    "find": {
                        "type": "array",
                        "description": "Search within a previously opened page or a known URL.",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["ref_id", "pattern"],
                            "properties": {
                                "ref_id": {
                                    "type": "string",
                                    "description": "A search result ref_id or a fully-qualified HTTP/HTTPS URL."
                                },
                                "pattern": {
                                    "type": "string",
                                    "description": "Plain-text substring to search for."
                                }
                            }
                        }
                    },
                    "response_length": {
                        "type": "string",
                        "enum": ["short", "medium", "long"],
                        "description": "Controls how many search results to return for search_query."
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.search_query.is_empty() && args.open.is_empty() && args.find.is_empty() {
            return Err(ToolExecError::new(
                "provide at least one of search_query, open, or find",
            ));
        }

        let response_length = ResponseLength::from(args.response_length);
        let mut output = WebRunOutput {
            search_query: Vec::new(),
            open: Vec::new(),
            find: Vec::new(),
        };

        for query in args.search_query {
            output.search_query.push(
                self.web
                    .search_query(&query.q, response_length)
                    .await
                    .map_err(|error| ToolExecError::new(error.to_string()))?,
            );
        }

        for open in args.open {
            output.open.push(
                self.web
                    .open(&open.ref_id, open.lineno)
                    .await
                    .map_err(|error| ToolExecError::new(error.to_string()))?,
            );
        }

        for find in args.find {
            output.find.push(
                self.web
                    .find(&find.ref_id, &find.pattern)
                    .await
                    .map_err(|error| ToolExecError::new(error.to_string()))?,
            );
        }

        serde_json::to_string(&output).map_err(|error| ToolExecError::new(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn rejects_empty_requests() {
        let tool = WebRunTool::new(
            WebService::new_for_tests(256, Duration::from_secs(60), 16 * 1024).expect("service"),
        );

        let error = tool
            .call(WebRunArgs::default())
            .await
            .expect_err("must fail");

        assert!(error.to_string().contains("provide at least one"));
    }
}
