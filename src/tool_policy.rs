use std::path::Path;

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::token_counting::TokenCounter;

const DEFAULT_EXCLUDED_DIRECTORY_NAMES: &[&str] =
    &["node_modules", "dist", "build", "coverage", "out", "target"];
const DEFAULT_EXCLUDED_FILE_NAMES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
    "npm-shrinkwrap.json",
];
const DEFAULT_EXCLUDED_FILE_SUFFIXES: &[&str] = &[".tsbuildinfo"];
const DEFAULT_TOOL_OUTPUT_MAX_TOKENS: usize = 10_000;

#[derive(Clone, Debug)]
pub struct SearchPathPolicy {
    allow_globs: GlobSet,
    allow_prefixes: Vec<String>,
}

#[derive(Clone)]
pub struct ToolOutputPolicy {
    max_output_tokens: usize,
    tokenizer: TokenCounter,
}

impl SearchPathPolicy {
    pub fn new(allow_patterns: &[String]) -> Result<Self> {
        let mut builder = GlobSetBuilder::new();
        let mut allow_prefixes = Vec::with_capacity(allow_patterns.len());

        for pattern in allow_patterns {
            let normalized = normalize_pattern(pattern);
            builder.add(Glob::new(&normalized).with_context(|| {
                format!("invalid tools.search_include_patterns entry `{pattern}`")
            })?);
            if let Some(prefix) = literal_pattern_prefix(&normalized) {
                allow_prefixes.push(prefix.to_string());
            }
        }

        Ok(Self {
            allow_globs: builder
                .build()
                .context("failed to build search include globs")?,
            allow_prefixes,
        })
    }

    pub fn validate_patterns(allow_patterns: &[String]) -> Result<()> {
        let _ = Self::new(allow_patterns)?;
        Ok(())
    }

    pub fn should_include(&self, relative_path: &Path, is_dir: bool) -> bool {
        let normalized = normalize_relative_path(relative_path);
        if normalized.is_empty() {
            return true;
        }
        if self.is_explicitly_allowed(&normalized, is_dir) {
            return true;
        }
        if is_dir && self.has_allowed_descendant(&normalized) {
            return true;
        }

        !is_default_excluded(&normalized, is_dir)
    }

    pub fn excluded_message(path: &str) -> String {
        format!(
            "{path} is excluded by the default search filters (hidden files/directories, gitignored paths, and common generated/package artifacts)"
        )
    }

    fn is_explicitly_allowed(&self, normalized_path: &str, is_dir: bool) -> bool {
        self.allow_globs.is_match(normalized_path)
            || (is_dir && self.allow_globs.is_match(format!("{normalized_path}/")))
    }

    fn has_allowed_descendant(&self, normalized_dir: &str) -> bool {
        self.allow_prefixes.iter().any(|prefix| {
            prefix == normalized_dir
                || prefix
                    .strip_prefix(normalized_dir)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
    }
}

impl ToolOutputPolicy {
    pub fn new(max_output_tokens: usize) -> Result<Self> {
        Ok(Self {
            max_output_tokens,
            tokenizer: TokenCounter::cl100k()
                .context("failed to initialize tool output tokenizer")?,
        })
    }

    pub fn truncate(&self, tool_name: &str, output: &str) -> String {
        let tokens = self.tokenizer.encode_text(output);
        if tokens.len() <= self.max_output_tokens {
            return output.to_string();
        }

        let kept = tokens[..self.max_output_tokens].to_vec();
        let truncated_output = self
            .tokenizer
            .decode_tokens(kept)
            .unwrap_or_else(|_| fallback_decode(output, self.max_output_tokens));

        format!(
            concat!(
                "[tool output truncated]\n",
                "`{tool_name}` returned approximately {actual} tokens; only the first {limit} tokens are shown below.\n",
                "Request the next chunk or narrow the request with a more specific path, pattern, offset, or line range.\n\n",
                "{truncated_output}"
            ),
            tool_name = tool_name,
            actual = tokens.len(),
            limit = self.max_output_tokens,
            truncated_output = truncated_output,
        )
    }
}

pub fn default_tool_output_max_tokens() -> usize {
    DEFAULT_TOOL_OUTPUT_MAX_TOKENS
}

fn is_default_excluded(normalized_path: &str, is_dir: bool) -> bool {
    let components = normalized_path
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    if components.is_empty() {
        return false;
    }

    if components
        .iter()
        .any(|component| component.starts_with('.') && *component != "." && *component != "..")
    {
        return true;
    }

    if components.iter().any(|component| {
        DEFAULT_EXCLUDED_DIRECTORY_NAMES
            .iter()
            .any(|excluded| component.eq_ignore_ascii_case(excluded))
    }) {
        return true;
    }

    if !is_dir {
        let file_name = components.last().copied().unwrap_or_default();
        if DEFAULT_EXCLUDED_FILE_NAMES
            .iter()
            .any(|excluded| file_name.eq_ignore_ascii_case(excluded))
        {
            return true;
        }
        if DEFAULT_EXCLUDED_FILE_SUFFIXES
            .iter()
            .any(|suffix| file_name.ends_with(suffix))
        {
            return true;
        }
    }

    false
}

fn normalize_relative_path(path: &Path) -> String {
    path.iter()
        .map(|component| component.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_pattern(pattern: &str) -> String {
    pattern
        .trim()
        .trim_start_matches("./")
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

fn literal_pattern_prefix(pattern: &str) -> Option<&str> {
    let wildcard_index = pattern.find(['*', '?', '[', '{']);
    let prefix = match wildcard_index {
        Some(index) => pattern[..index].trim_end_matches('/'),
        None => pattern.trim_end_matches('/'),
    };
    if prefix.is_empty() {
        None
    } else {
        Some(prefix)
    }
}

fn fallback_decode(output: &str, max_output_tokens: usize) -> String {
    output
        .chars()
        .take(max_output_tokens.saturating_mul(4))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn policy_excludes_hidden_and_generated_paths_by_default() {
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");

        assert!(!policy.should_include(Path::new(".env"), false));
        assert!(!policy.should_include(Path::new("node_modules/react/index.js"), false));
        assert!(!policy.should_include(Path::new("dist/assets/app.js"), false));
        assert!(!policy.should_include(Path::new("package-lock.json"), false));
        assert!(policy.should_include(Path::new("src/main.tsx"), false));
    }

    #[test]
    fn policy_can_allow_hidden_directory_patterns() {
        let policy = SearchPathPolicy::new(&[".research/**".into()]).expect("policy builds");

        assert!(policy.should_include(Path::new(".research"), true));
        assert!(policy.should_include(Path::new(".research/notes.txt"), false));
    }

    #[test]
    fn tool_output_policy_truncates_large_outputs() {
        let policy = ToolOutputPolicy::new(16).expect("policy builds");
        let output =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(8);

        let truncated = policy.truncate("Grep", &output);

        assert!(truncated.contains("[tool output truncated]"));
        assert!(truncated.contains("`Grep` returned approximately"));
        assert!(truncated.contains("Request the next chunk"));
    }

    #[test]
    fn tool_output_policy_leaves_small_outputs_unchanged() {
        let policy = ToolOutputPolicy::new(10_000).expect("policy builds");
        let output = "small output";

        assert_eq!(policy.truncate("ReadFile", output), output);
    }
}
