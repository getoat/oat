use std::borrow::Cow;

const TOOLSET_ERROR_PREFIX: &str = "Toolset error:";
const TOOL_CALL_ERROR_PREFIX: &str = "ToolCallError:";

pub(crate) fn normalize_tool_result_text(result: &str) -> Cow<'_, str> {
    serde_json::from_str::<String>(result)
        .map(Cow::Owned)
        .unwrap_or_else(|_| Cow::Borrowed(result))
}

pub(crate) fn tool_result_is_failure_text(result: &str) -> bool {
    tool_result_failure_reason(result).is_some()
}

pub(crate) fn tool_result_failure_reason(result: &str) -> Option<String> {
    let normalized = normalize_tool_result_text(result);
    let text = normalized.trim();
    if !text.contains(TOOL_CALL_ERROR_PREFIX) {
        return None;
    }

    let text = text
        .find(TOOL_CALL_ERROR_PREFIX)
        .and_then(|index| text.get(index..))
        .unwrap_or(text);
    let reason = strip_failure_prefixes(text);
    if reason.is_empty() {
        Some("Tool call failed.".to_string())
    } else {
        Some(reason.to_string())
    }
}

fn strip_failure_prefixes(mut text: &str) -> &str {
    loop {
        let trimmed = text.trim();
        if let Some(rest) = trimmed.strip_prefix(TOOLSET_ERROR_PREFIX) {
            text = rest;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(TOOL_CALL_ERROR_PREFIX) {
            text = rest;
            continue;
        }
        return trimmed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_result_failure_reason_extracts_direct_tool_error() {
        assert_eq!(
            tool_result_failure_reason("ToolCallError: Shell command timed out after 10ms."),
            Some("Shell command timed out after 10ms.".into())
        );
    }

    #[test]
    fn tool_result_failure_reason_extracts_wrapped_tool_error() {
        assert_eq!(
            tool_result_failure_reason(
                r#""Toolset error: ToolCallError: ToolCallError: patch 1 old_text was not found in src/main.tsx""#
            ),
            Some("patch 1 old_text was not found in src/main.tsx".into())
        );
    }

    #[test]
    fn tool_result_failure_reason_ignores_non_failures() {
        assert_eq!(tool_result_failure_reason("Exit code: 0"), None);
    }
}
