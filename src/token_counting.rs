use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow};
use serde_json::Value;
use tiktoken_rs::{CoreBPE, cl100k_base};

const FALLBACK_TEXT_CHARS_PER_TOKEN: usize = 4;
const FALLBACK_BINARY_BYTES_PER_TOKEN: usize = 4;

static CL100K_TOKENIZER: OnceLock<std::result::Result<Arc<CoreBPE>, String>> = OnceLock::new();

#[derive(Clone)]
pub struct TokenCounter {
    tokenizer: Arc<CoreBPE>,
}

impl TokenCounter {
    pub fn cl100k() -> Result<Self> {
        let tokenizer = CL100K_TOKENIZER
            .get_or_init(|| {
                cl100k_base()
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map(Arc::clone)
            .map_err(|error| anyhow!("{error}"))?;
        Ok(Self { tokenizer })
    }

    pub fn count_text(&self, text: &str) -> usize {
        if text.is_empty() {
            0
        } else {
            self.tokenizer.encode_ordinary(text).len()
        }
    }

    pub fn count_json(&self, value: &Value) -> usize {
        serde_json::to_string(value)
            .map(|json| self.count_text(&json))
            .unwrap_or(0)
    }

    pub fn encode_text(&self, text: &str) -> Vec<u32> {
        self.tokenizer.encode_ordinary(text)
    }

    pub fn decode_tokens(&self, tokens: Vec<u32>) -> Result<String> {
        self.tokenizer
            .decode(tokens)
            .map_err(|error| anyhow!(error.to_string()))
    }
}

pub fn count_text_tokens(text: &str) -> u64 {
    TokenCounter::cl100k()
        .map(|counter| counter.count_text(text) as u64)
        .unwrap_or_else(|_| fallback_text_tokens(text))
}

pub fn count_json_tokens(value: &Value) -> u64 {
    TokenCounter::cl100k()
        .map(|counter| counter.count_json(value) as u64)
        .unwrap_or_else(|_| {
            serde_json::to_string(value)
                .map(|json| fallback_text_tokens(&json))
                .unwrap_or(0)
        })
}

pub fn estimate_binary_tokens(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        0
    } else {
        bytes.len().div_ceil(FALLBACK_BINARY_BYTES_PER_TOKEN) as u64
    }
}

fn fallback_text_tokens(text: &str) -> u64 {
    if text.is_empty() {
        0
    } else {
        text.chars().count().div_ceil(FALLBACK_TEXT_CHARS_PER_TOKEN) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tokenizer_counts_text() {
        assert!(count_text_tokens("hello world") > 0);
    }

    #[test]
    fn tokenizer_counts_json() {
        assert!(count_json_tokens(&json!({ "path": "src/lib.rs" })) > 0);
    }

    #[test]
    fn binary_estimate_is_non_zero_for_non_empty_payloads() {
        assert_eq!(estimate_binary_tokens(&[]), 0);
        assert_eq!(estimate_binary_tokens(&[0_u8; 4]), 1);
    }
}
