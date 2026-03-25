use rig::completion::{
    Message as RigMessage,
    message::{AssistantContent, ToolResultContent, UserContent},
};

use crate::{
    completion_request::{estimated_history_context_tokens, estimated_message_tokens},
    model_registry,
};

pub(crate) const COMPACTION_PROMPT: &str = "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.\n\nInclude:\n- Current progress and key decisions made\n- Important context, constraints, or user preferences\n- What remains to be done (clear next steps)\n- Any critical data, examples, or references needed to continue\n- Decision complete plan, if using\n\nBe concise, structured, and focused on helping the next LLM seamlessly continue the work.";
pub(crate) const COMPACTION_SUMMARY_PREFIX: &str = "Another language model started to solve this problem and produced a summary of its thinking process. You have access to the state of the last few tools that were used by that language model, and the last few tokens of user messages to contextualise. Use this to build on the work that has already been done and avoid duplicating work. Here is the summary produced by the other language model; use the information in the summary to assist with your own analysis:\n";
pub(crate) const COMPACTION_USER_TOKEN_BUDGET: usize = 10_000;
pub(crate) const COMPACTION_TOOL_TOKEN_BUDGET: usize = 10_000;
pub(crate) const COMPACTION_NOTICE: &str = "Context compacted.";

pub(crate) fn estimated_request_tokens(history: &[RigMessage], prompt: &RigMessage) -> usize {
    (estimated_history_context_tokens(history) + estimated_message_tokens(prompt)) as usize
}

pub(crate) fn compaction_model_for_pre_turn(
    current_model_name: &str,
    history: &[RigMessage],
    history_model_name: Option<&str>,
    prompt: &RigMessage,
) -> Option<String> {
    if !should_compact_request_for_model(current_model_name, history, prompt) {
        return None;
    }

    let Some(previous_model_name) = history_model_name else {
        return Some(current_model_name.to_string());
    };
    let Some(previous_model) = model_registry::find_model(previous_model_name) else {
        return Some(current_model_name.to_string());
    };
    let Some(current_model) = model_registry::find_model(current_model_name) else {
        return Some(current_model_name.to_string());
    };
    if previous_model.context_length > current_model.context_length
        && should_compact_request_for_model(current_model_name, history, &RigMessage::user(""))
    {
        Some(previous_model_name.to_string())
    } else {
        Some(current_model_name.to_string())
    }
}

pub(crate) fn should_compact_request_for_model(
    model_name: &str,
    history: &[RigMessage],
    prompt: &RigMessage,
) -> bool {
    model_registry::find_model(model_name).is_some_and(|model| {
        model.should_compact_for_input_tokens(estimated_request_tokens(history, prompt))
    })
}

pub(crate) fn is_retryable_compaction_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    [
        "context_length_exceeded",
        "input tokens exceed",
        "maximum context",
        "too large",
        "max_output_tokens",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

pub(crate) fn drop_oldest_compaction_source_message(history: &mut Vec<RigMessage>) -> bool {
    if history.is_empty() {
        false
    } else {
        history.remove(0);
        true
    }
}

pub(crate) fn rebuild_compacted_history(history: &[RigMessage], summary: &str) -> Vec<RigMessage> {
    let user_indexes = retain_tail_indexes(
        history,
        COMPACTION_USER_TOKEN_BUDGET,
        |message| matches!(message, RigMessage::User { content } if content.iter().any(is_regular_user_content)),
    );
    let tool_indexes = retain_tail_indexes(history, COMPACTION_TOOL_TOKEN_BUDGET, |message| {
        message_contains_tool_state(message)
    });

    let mut retained = user_indexes
        .into_iter()
        .chain(tool_indexes)
        .collect::<Vec<_>>();
    retained.sort_unstable();
    retained.dedup();

    let mut rebuilt = retained
        .into_iter()
        .map(|index| history[index].clone())
        .collect::<Vec<_>>();
    rebuilt.push(RigMessage::user(format!(
        "{COMPACTION_SUMMARY_PREFIX}{summary}"
    )));
    rebuilt
}

fn retain_tail_indexes(
    history: &[RigMessage],
    token_budget: usize,
    predicate: impl Fn(&RigMessage) -> bool,
) -> Vec<usize> {
    let mut kept = Vec::new();
    let mut used_tokens = 0usize;

    for (index, message) in history.iter().enumerate().rev() {
        if !predicate(message) {
            continue;
        }

        let message_tokens = estimated_message_tokens(message) as usize;
        if !kept.is_empty() && used_tokens + message_tokens > token_budget {
            break;
        }
        kept.push(index);
        used_tokens += message_tokens;
        if used_tokens >= token_budget {
            break;
        }
    }

    kept.reverse();
    kept
}

fn is_regular_user_content(content: &UserContent) -> bool {
    !matches!(content, UserContent::ToolResult(_))
}

pub(crate) fn message_contains_tool_state(message: &RigMessage) -> bool {
    match message {
        RigMessage::Assistant { content, .. } => content
            .iter()
            .any(|content| matches!(content, AssistantContent::ToolCall(_))),
        RigMessage::User { content } => content
            .iter()
            .any(|content| matches!(content, UserContent::ToolResult(_))),
        RigMessage::System { .. } => false,
    }
}

pub(crate) fn should_compact_before_follow_up(
    model_name: &str,
    history: &[RigMessage],
    prompt: &RigMessage,
) -> bool {
    should_compact_request_for_model(model_name, history, prompt)
}

pub(crate) fn format_tool_result(tool_result: &rig::completion::message::ToolResult) -> String {
    let parts = tool_result
        .content
        .iter()
        .map(|content| match content {
            ToolResultContent::Text(text) => text.text.clone(),
            ToolResultContent::Image(_) => "[image tool result]".to_string(),
        })
        .collect::<Vec<_>>();

    if parts.is_empty() {
        String::new()
    } else {
        parts.join("\n")
    }
}
