use std::collections::VecDeque;

use rig::{
    OneOrMany,
    completion::{
        Message as RigMessage,
        message::{AssistantContent, ToolCall, ToolResult, ToolResultContent, UserContent},
    },
};

use crate::{config::HistoryMode, tool_result_status::tool_result_failure_reason};

pub(crate) fn reduce_history(
    history: &[RigMessage],
    mode: HistoryMode,
    retained_steps: usize,
    finalize_last_turn: bool,
) -> Vec<RigMessage> {
    if matches!(mode, HistoryMode::Full) || history.is_empty() {
        return history.to_vec();
    }

    let mut reduced = Vec::new();
    let mut current_turn = Vec::new();

    for message in history {
        if matches!(message, RigMessage::System { .. }) {
            if !current_turn.is_empty() {
                reduced.extend(reduce_turn(&current_turn, mode, retained_steps, true));
                current_turn.clear();
            }
            reduced.push(message.clone());
            continue;
        }

        if is_regular_user_message(message) && !current_turn.is_empty() {
            reduced.extend(reduce_turn(&current_turn, mode, retained_steps, true));
            current_turn.clear();
        }

        if current_turn.is_empty() && !is_regular_user_message(message) {
            reduced.push(message.clone());
            continue;
        }

        current_turn.push(message.clone());
    }

    if !current_turn.is_empty() {
        reduced.extend(reduce_turn(
            &current_turn,
            mode,
            retained_steps,
            finalize_last_turn,
        ));
    }

    reduced
}

pub(crate) fn compact_tool_traces(history: &[RigMessage]) -> Vec<RigMessage> {
    if history.is_empty() {
        return Vec::new();
    }

    let mut reduced = Vec::new();
    let mut current_turn = Vec::new();

    for message in history {
        if matches!(message, RigMessage::System { .. }) {
            if !current_turn.is_empty() {
                reduced.extend(compact_tool_traces_in_turn(&current_turn));
                current_turn.clear();
            }
            reduced.push(message.clone());
            continue;
        }

        if is_regular_user_message(message) && !current_turn.is_empty() {
            reduced.extend(compact_tool_traces_in_turn(&current_turn));
            current_turn.clear();
        }

        if current_turn.is_empty() && !is_regular_user_message(message) {
            reduced.push(message.clone());
            continue;
        }

        current_turn.push(message.clone());
    }

    if !current_turn.is_empty() {
        reduced.extend(compact_tool_traces_in_turn(&current_turn));
    }

    reduced
}

fn reduce_turn(
    turn: &[RigMessage],
    mode: HistoryMode,
    retained_steps: usize,
    finalized: bool,
) -> Vec<RigMessage> {
    if turn.is_empty() || matches!(mode, HistoryMode::Full) {
        return turn.to_vec();
    }
    if matches!(mode, HistoryMode::TurnSummary) && !finalized {
        return turn.to_vec();
    }

    let prompt = turn[0].clone();
    let steps = split_turn_steps(&turn[1..], finalized);
    if steps.is_empty() {
        return turn.to_vec();
    }

    let raw_completed_steps = match mode {
        HistoryMode::StepSummary => retained_steps,
        HistoryMode::TurnSummary | HistoryMode::Full => 0,
    };
    let completed_count = steps.iter().filter(|step| step.completed).count();
    let summarize_completed_until = completed_count.saturating_sub(raw_completed_steps);

    let mut reduced = vec![prompt];
    let mut completed_seen = 0usize;

    for step in steps {
        let summarize = step.completed && completed_seen < summarize_completed_until;
        if step.completed {
            completed_seen += 1;
        }

        if summarize {
            reduced.extend(reduce_step_messages(&step.messages));
        } else {
            reduced.extend(step.messages);
        }
    }

    reduced
}

fn compact_tool_traces_in_turn(turn: &[RigMessage]) -> Vec<RigMessage> {
    if turn.is_empty() {
        return Vec::new();
    }

    let prompt = turn[0].clone();
    let steps = split_turn_steps(&turn[1..], true);
    if steps.is_empty() {
        return turn.to_vec();
    }

    let mut reduced = vec![prompt];
    for step in steps {
        reduced.extend(reduce_step_messages(&step.messages));
    }
    reduced
}

#[derive(Clone)]
struct TurnStep {
    messages: Vec<RigMessage>,
    completed: bool,
}

fn split_turn_steps(messages: &[RigMessage], finalized: bool) -> Vec<TurnStep> {
    let mut steps = Vec::new();
    let mut current = Vec::new();
    let mut saw_tool_result = false;

    for message in messages {
        if matches!(message, RigMessage::Assistant { .. }) && !current.is_empty() && saw_tool_result
        {
            steps.push(TurnStep {
                messages: current,
                completed: true,
            });
            current = Vec::new();
            saw_tool_result = false;
        }

        if is_regular_user_message(message) && !current.is_empty() {
            steps.push(TurnStep {
                messages: current,
                completed: true,
            });
            current = Vec::new();
            saw_tool_result = false;
        }

        current.push(message.clone());
        if is_tool_result_message(message) {
            saw_tool_result = true;
        }
    }

    if !current.is_empty() {
        steps.push(TurnStep {
            messages: current,
            completed: finalized || saw_tool_result,
        });
    }

    steps
}

fn reduce_step_messages(messages: &[RigMessage]) -> Vec<RigMessage> {
    let mut reduced = Vec::new();
    let mut pending = VecDeque::<PendingSummary>::new();

    for message in messages {
        match message {
            RigMessage::Assistant { content, .. } => {
                for part in content.iter() {
                    match part {
                        AssistantContent::ToolCall(tool_call) => {
                            pending.push_back(PendingSummary::from_tool_call(tool_call));
                        }
                        _ => {
                            flush_pending(&mut reduced, &mut pending);
                            reduced.push(assistant_message_from_content(part.clone()));
                        }
                    }
                }
            }
            RigMessage::User { content } => {
                for part in content.iter() {
                    match part {
                        UserContent::ToolResult(tool_result) => {
                            if let Some(summary) = pop_matching_pending(&mut pending, tool_result) {
                                if let Some(reason) =
                                    tool_result_failure_reason_for_tool_result(tool_result)
                                {
                                    if let Some(message) =
                                        summarize_failed_tool_trace(&summary, &reason)
                                    {
                                        reduced.push(RigMessage::assistant(message));
                                    }
                                } else {
                                    if let Some(message) =
                                        summarize_tool_trace(&summary, tool_result)
                                    {
                                        reduced.push(RigMessage::assistant(message));
                                    }
                                }
                            } else {
                                reduced.push(user_message_from_content(UserContent::ToolResult(
                                    tool_result.clone(),
                                )));
                            }
                        }
                        _ => {
                            flush_pending(&mut reduced, &mut pending);
                            reduced.push(user_message_from_content(part.clone()));
                        }
                    }
                }
            }
            RigMessage::System { .. } => {
                flush_pending(&mut reduced, &mut pending);
                reduced.push(message.clone());
            }
        }
    }

    flush_pending(&mut reduced, &mut pending);
    reduced
}

fn pop_matching_pending(
    pending: &mut VecDeque<PendingSummary>,
    tool_result: &ToolResult,
) -> Option<PendingSummary> {
    if let Some(call_id) = tool_result.call_id.as_deref()
        && let Some(index) = pending
            .iter()
            .position(|summary| summary.call_id.as_deref() == Some(call_id))
    {
        return pending.remove(index);
    }

    if let Some(index) = pending
        .iter()
        .position(|summary| summary.id == tool_result.id)
    {
        return pending.remove(index);
    }

    pending.pop_front()
}

fn assistant_message_from_content(content: AssistantContent) -> RigMessage {
    RigMessage::Assistant {
        id: None,
        content: OneOrMany::one(content),
    }
}

fn user_message_from_content(content: UserContent) -> RigMessage {
    RigMessage::User {
        content: OneOrMany::one(content),
    }
}

fn flush_pending(reduced: &mut Vec<RigMessage>, pending: &mut VecDeque<PendingSummary>) {
    while let Some(summary) = pending.pop_front() {
        reduced.push(summary.raw_tool_call);
    }
}

#[derive(Clone)]
struct PendingSummary {
    raw_tool_call: RigMessage,
    summary_kind: SummaryKind,
    tool_name: String,
    id: String,
    call_id: Option<String>,
}

impl PendingSummary {
    fn from_tool_call(tool_call: &ToolCall) -> Self {
        let args = &tool_call.function.arguments;
        let summary_kind = match tool_call.function.name.as_str() {
            "ReadFile" => SummaryKind::ReadFile {
                filename: args
                    .get("filename")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
                    .to_string(),
                offset: args
                    .get("offset")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                limit: args
                    .get("limit")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
            },
            "ReadFiles" => {
                let files = args
                    .get("files")
                    .and_then(|value| value.as_array())
                    .map(|entries| {
                        entries
                            .iter()
                            .filter_map(|entry| {
                                Some((
                                    entry.get("filename")?.as_str()?.to_string(),
                                    entry.get("offset")?.as_u64()?,
                                    entry.get("limit")?.as_u64()?,
                                ))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if files.is_empty() {
                    SummaryKind::Generic {
                        tool_name: tool_call.function.name.clone(),
                    }
                } else {
                    SummaryKind::ReadFiles { files }
                }
            }
            "WriteFile" => SummaryKind::WriteFile {
                filename: args
                    .get("filename")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
                    .to_string(),
                line_count: args
                    .get("content")
                    .and_then(|value| value.as_str())
                    .map(line_count)
                    .unwrap_or(0),
                intent: args
                    .get("intent")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            },
            "ApplyPatches" => SummaryKind::ApplyPatches {
                filename: args
                    .get("filename")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
                    .to_string(),
                patch_count: args
                    .get("patches")
                    .and_then(|value| value.as_array())
                    .map(|patches| patches.len())
                    .unwrap_or(0),
                intent: args
                    .get("intent")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            },
            "List" => SummaryKind::List {
                dir: args
                    .get("dir")
                    .and_then(|value| value.as_str())
                    .unwrap_or(".")
                    .to_string(),
                recursive: args
                    .get("recursive")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
            },
            "Grep" => SummaryKind::Grep {
                pattern: args
                    .get("pattern")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
                    .to_string(),
                path: args
                    .get("path")
                    .and_then(|value| value.as_str())
                    .unwrap_or(".")
                    .to_string(),
            },
            "RunShellScript" => SummaryKind::RunShellScript {
                cwd: args
                    .get("cwd")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                intent: args
                    .get("intent")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            },
            "StartBackgroundTerminal" => SummaryKind::StartBackgroundTerminal {
                label: args
                    .get("label")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                cwd: args
                    .get("cwd")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            },
            "ListBackgroundTerminals" => SummaryKind::ListBackgroundTerminals,
            "InspectBackgroundTerminal" => SummaryKind::InspectBackgroundTerminal {
                id: args
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
                    .to_string(),
            },
            "KillBackgroundTerminal" => SummaryKind::KillBackgroundTerminal {
                id: args
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
                    .to_string(),
            },
            "DeletePath" => SummaryKind::DeletePath {
                path: args
                    .get("path")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
                    .to_string(),
            },
            "SearchMemories" | "GetMemory" | "Todo" | "Commentary" => SummaryKind::Ephemeral,
            _ => SummaryKind::Generic {
                tool_name: tool_call.function.name.clone(),
            },
        };

        Self {
            raw_tool_call: assistant_message_from_content(AssistantContent::ToolCall(
                tool_call.clone(),
            )),
            summary_kind,
            tool_name: tool_call.function.name.clone(),
            id: tool_call.id.clone(),
            call_id: tool_call.call_id.clone(),
        }
    }
}

#[derive(Clone)]
enum SummaryKind {
    ReadFile {
        filename: String,
        offset: u64,
        limit: u64,
    },
    ReadFiles {
        files: Vec<(String, u64, u64)>,
    },
    WriteFile {
        filename: String,
        line_count: usize,
        intent: Option<String>,
    },
    ApplyPatches {
        filename: String,
        patch_count: usize,
        intent: Option<String>,
    },
    List {
        dir: String,
        recursive: bool,
    },
    Grep {
        pattern: String,
        path: String,
    },
    RunShellScript {
        cwd: Option<String>,
        intent: Option<String>,
    },
    StartBackgroundTerminal {
        label: Option<String>,
        cwd: Option<String>,
    },
    ListBackgroundTerminals,
    InspectBackgroundTerminal {
        id: String,
    },
    KillBackgroundTerminal {
        id: String,
    },
    DeletePath {
        path: String,
    },
    Generic {
        tool_name: String,
    },
    Ephemeral,
}

fn summarize_tool_trace(summary: &PendingSummary, _tool_result: &ToolResult) -> Option<String> {
    match &summary.summary_kind {
        SummaryKind::ReadFile {
            filename,
            offset,
            limit,
        } => Some(format!(
            "Read `{filename}` lines {}-{}.",
            offset + 1,
            offset + limit
        )),
        SummaryKind::ReadFiles { files } => Some(format!(
            "Read {}.",
            files
                .iter()
                .map(|(filename, offset, limit)| format!(
                    "`{filename}` lines {}-{}",
                    offset + 1,
                    offset + limit
                ))
                .collect::<Vec<_>>()
                .join("; ")
        )),
        SummaryKind::WriteFile {
            filename,
            line_count,
            intent,
        } => Some(match intent {
            Some(intent) if !intent.trim().is_empty() => {
                format!("Wrote `{filename}` ({line_count} lines). Intent: {intent}")
            }
            _ => format!("Wrote `{filename}` ({line_count} lines)."),
        }),
        SummaryKind::ApplyPatches {
            filename,
            patch_count,
            intent,
        } => Some(match intent {
            Some(intent) if !intent.trim().is_empty() => {
                format!("Updated `{filename}` with {patch_count} patch(es). Intent: {intent}")
            }
            _ => format!("Updated `{filename}` with {patch_count} patch(es)."),
        }),
        SummaryKind::List { dir, recursive } => Some(if *recursive {
            format!("Listed `{dir}` recursively.")
        } else {
            format!("Listed `{dir}`.")
        }),
        SummaryKind::Grep { pattern, path } => Some(format!(
            "Searched `{path}` for `/{}/`.",
            truncate_summary_text(pattern, 48)
        )),
        SummaryKind::RunShellScript { cwd, intent } => Some(match (cwd, intent) {
            (Some(cwd), Some(intent)) if !intent.trim().is_empty() => {
                format!("Ran a shell command in `{cwd}`. Intent: {intent}")
            }
            (Some(cwd), _) => format!("Ran a shell command in `{cwd}`."),
            (None, Some(intent)) if !intent.trim().is_empty() => {
                format!("Ran a shell command. Intent: {intent}")
            }
            _ => "Ran a shell command.".to_string(),
        }),
        SummaryKind::StartBackgroundTerminal { label, cwd } => Some(match (label, cwd) {
            (Some(label), Some(cwd)) if !label.trim().is_empty() => {
                format!("Started background terminal `{label}` in `{cwd}`.")
            }
            (Some(label), _) if !label.trim().is_empty() => {
                format!("Started background terminal `{label}`.")
            }
            (None, Some(cwd)) => format!("Started a background terminal in `{cwd}`."),
            _ => "Started a background terminal.".to_string(),
        }),
        SummaryKind::ListBackgroundTerminals => Some("Listed background terminals.".to_string()),
        SummaryKind::InspectBackgroundTerminal { id } => {
            Some(format!("Inspected background terminal `{id}`."))
        }
        SummaryKind::KillBackgroundTerminal { id } => {
            Some(format!("Stopped background terminal `{id}`."))
        }
        SummaryKind::DeletePath { path } => Some(format!("Deleted `{path}`.")),
        SummaryKind::Generic { tool_name } => Some(format!("Used `{tool_name}`.")),
        SummaryKind::Ephemeral => None,
    }
}

fn summarize_failed_tool_trace(summary: &PendingSummary, reason: &str) -> Option<String> {
    let reason = truncate_summary_text(reason, 160);
    match &summary.summary_kind {
        SummaryKind::ReadFile { filename, .. } => {
            Some(format!("Failed to read `{filename}`: {reason}"))
        }
        SummaryKind::ReadFiles { files } => Some(format!(
            "Failed to read {}: {reason}",
            files
                .iter()
                .map(|(filename, _, _)| format!("`{filename}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        SummaryKind::WriteFile { filename, .. } => {
            Some(format!("Failed to write `{filename}`: {reason}"))
        }
        SummaryKind::ApplyPatches { filename, .. } => {
            Some(format!("Failed to update `{filename}`: {reason}"))
        }
        SummaryKind::List { dir, .. } => Some(format!("Failed to list `{dir}`: {reason}")),
        SummaryKind::Grep { pattern, path } => Some(format!(
            "Failed to search `{path}` for `/{}/`: {reason}",
            truncate_summary_text(pattern, 48)
        )),
        SummaryKind::RunShellScript { cwd, .. } => Some(match cwd {
            Some(cwd) => format!("Shell command failed in `{cwd}`: {reason}"),
            None => format!("Shell command failed: {reason}"),
        }),
        SummaryKind::StartBackgroundTerminal { label, cwd } => Some(match (label, cwd) {
            (Some(label), _) if !label.trim().is_empty() => {
                format!("Failed to start background terminal `{label}`: {reason}")
            }
            (None, Some(cwd)) => {
                format!("Failed to start a background terminal in `{cwd}`: {reason}")
            }
            _ => format!("Failed to start a background terminal: {reason}"),
        }),
        SummaryKind::ListBackgroundTerminals => {
            Some(format!("Failed to list background terminals: {reason}"))
        }
        SummaryKind::InspectBackgroundTerminal { id } => Some(format!(
            "Failed to inspect background terminal `{id}`: {reason}"
        )),
        SummaryKind::KillBackgroundTerminal { id } => Some(format!(
            "Failed to stop background terminal `{id}`: {reason}"
        )),
        SummaryKind::DeletePath { path } => Some(format!("Failed to delete `{path}`: {reason}")),
        SummaryKind::Generic { tool_name } => Some(format!("`{tool_name}` failed: {reason}")),
        SummaryKind::Ephemeral => Some(format!("`{}` failed: {reason}", summary.tool_name)),
    }
}

fn line_count(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
    }
}

fn truncate_summary_text(text: &str, max_chars: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return text.to_string();
    }

    let keep = max_chars.saturating_sub(3);
    format!("{}...", chars.into_iter().take(keep).collect::<String>())
}

fn tool_result_failure_reason_for_tool_result(tool_result: &ToolResult) -> Option<String> {
    let text = tool_result
        .content
        .iter()
        .filter_map(|content| match content {
            ToolResultContent::Text(text) => Some(text.text.as_str()),
            ToolResultContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    tool_result_failure_reason(&text)
}

fn is_regular_user_message(message: &RigMessage) -> bool {
    matches!(
        message,
        RigMessage::User { content }
            if content.iter().any(|part| !matches!(part, UserContent::ToolResult(_)))
    )
}

fn is_tool_result_message(message: &RigMessage) -> bool {
    matches!(
        message,
        RigMessage::User { content }
            if content.iter().any(|part| matches!(part, UserContent::ToolResult(_)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::message::Text;
    use serde_json::json;

    fn tool_call(name: &str, arguments: serde_json::Value) -> RigMessage {
        RigMessage::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                id: "tool-1".into(),
                call_id: Some("call-1".into()),
                function: rig::completion::message::ToolFunction::new(name.into(), arguments),
                signature: None,
                additional_params: None,
            })),
        }
    }

    fn tool_result(text: &str) -> RigMessage {
        RigMessage::User {
            content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: "tool-1".into(),
                call_id: Some("call-1".into()),
                content: OneOrMany::one(ToolResultContent::Text(Text { text: text.into() })),
            })),
        }
    }

    #[test]
    fn step_summary_reduces_older_steps_only() {
        let history = vec![
            RigMessage::user("prompt"),
            tool_call(
                "ReadFile",
                json!({"filename":"src/main.rs","offset":0,"limit":10}),
            ),
            tool_result("1 | hello"),
            RigMessage::assistant("working"),
            tool_call(
                "ReadFile",
                json!({"filename":"src/lib.rs","offset":0,"limit":10}),
            ),
            tool_result("1 | world"),
            RigMessage::assistant("current"),
        ];

        let reduced = reduce_history(&history, HistoryMode::StepSummary, 1, false);
        assert!(
            reduced
                .iter()
                .any(|message| message == &RigMessage::assistant("Read `src/main.rs` lines 1-10."))
        );
        assert!(reduced.iter().any(|message| message
            == &tool_call(
                "ReadFile",
                json!({"filename":"src/lib.rs","offset":0,"limit":10})
            )));
    }

    #[test]
    fn compact_tool_traces_summarizes_successful_latest_step_tools() {
        let history = vec![
            RigMessage::user("prompt"),
            tool_call(
                "ReadFile",
                json!({"filename":"src/main.rs","offset":0,"limit":10}),
            ),
            tool_result("1 | hello"),
            RigMessage::assistant("done"),
        ];

        let reduced = compact_tool_traces(&history);

        assert_eq!(
            reduced,
            vec![
                RigMessage::user("prompt"),
                RigMessage::assistant("Read `src/main.rs` lines 1-10."),
                RigMessage::assistant("done"),
            ]
        );
    }

    #[test]
    fn compact_tool_traces_drops_todo_traces_entirely() {
        let history = vec![
            RigMessage::user("prompt"),
            tool_call(
                "Todo",
                json!({"operation":"update","tasks":[{"description":"Inspect workspace","status":"in_progress"}]}),
            ),
            tool_result("{\"has_list\":true}"),
            RigMessage::assistant("done"),
        ];

        let reduced = compact_tool_traces(&history);

        assert_eq!(
            reduced,
            vec![RigMessage::user("prompt"), RigMessage::assistant("done"),]
        );
    }

    #[test]
    fn compact_tool_traces_handles_multi_item_assistant_tool_calls() {
        let history = vec![
            RigMessage::user("prompt"),
            RigMessage::Assistant {
                id: None,
                content: OneOrMany::many(vec![
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-1".into(),
                        call_id: Some("call-1".into()),
                        function: rig::completion::message::ToolFunction::new(
                            "WriteFile".into(),
                            json!({"filename":"src/App.tsx","content":"one\ntwo","intent":"Create app"}),
                        ),
                        signature: None,
                        additional_params: None,
                    }),
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-2".into(),
                        call_id: Some("call-2".into()),
                        function: rig::completion::message::ToolFunction::new(
                            "Todo".into(),
                            json!({"operation":"update","tasks":[]}),
                        ),
                        signature: None,
                        additional_params: None,
                    }),
                ])
                .expect("multiple assistant content items"),
            },
            RigMessage::User {
                content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                    id: "tool-1".into(),
                    call_id: Some("call-1".into()),
                    content: OneOrMany::one(ToolResultContent::Text(Text {
                        text: "ok".into(),
                    })),
                })),
            },
            RigMessage::User {
                content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                    id: "tool-2".into(),
                    call_id: Some("call-2".into()),
                    content: OneOrMany::one(ToolResultContent::Text(Text {
                        text: "{\"has_list\":true}".into(),
                    })),
                })),
            },
            RigMessage::assistant("done"),
        ];

        let reduced = compact_tool_traces(&history);

        assert_eq!(
            reduced,
            vec![
                RigMessage::user("prompt"),
                RigMessage::assistant("Wrote `src/App.tsx` (2 lines). Intent: Create app"),
                RigMessage::assistant("done"),
            ]
        );
    }

    #[test]
    fn compact_tool_traces_drops_memory_lookup_traces() {
        let history = vec![
            RigMessage::user("prompt"),
            tool_call("SearchMemories", json!({"query":"tailwind", "limit":5})),
            tool_result("Memory results for `tailwind`..."),
            tool_call("GetMemory", json!({"id":"019d79c1"})),
            tool_result("Full memory text"),
            RigMessage::assistant("done"),
        ];

        let reduced = compact_tool_traces(&history);

        assert_eq!(
            reduced,
            vec![RigMessage::user("prompt"), RigMessage::assistant("done"),]
        );
    }

    #[test]
    fn compact_tool_traces_summarizes_failed_apply_patch_attempts() {
        let history = vec![
            RigMessage::user("prompt"),
            tool_call(
                "ApplyPatches",
                json!({
                    "filename":"src/main.tsx",
                    "patches":[{"old_text":"old","new_text":"new"}],
                    "intent":"Update main file"
                }),
            ),
            tool_result(
                "Toolset error: ToolCallError: ToolCallError: patch 1 old_text was not found in src/main.tsx",
            ),
            RigMessage::assistant("done"),
        ];

        let reduced = compact_tool_traces(&history);

        assert_eq!(
            reduced,
            vec![
                RigMessage::user("prompt"),
                RigMessage::assistant(
                    "Failed to update `src/main.tsx`: patch 1 old_text was not found in src/main.tsx"
                ),
                RigMessage::assistant("done"),
            ]
        );
    }

    #[test]
    fn compact_tool_traces_matches_out_of_order_results_by_call_id() {
        let history = vec![
            RigMessage::user("prompt"),
            RigMessage::Assistant {
                id: None,
                content: OneOrMany::many(vec![
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-1".into(),
                        call_id: Some("call-1".into()),
                        function: rig::completion::message::ToolFunction::new(
                            "ReadFile".into(),
                            json!({"filename":"src/one.rs","offset":0,"limit":5}),
                        ),
                        signature: None,
                        additional_params: None,
                    }),
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-2".into(),
                        call_id: Some("call-2".into()),
                        function: rig::completion::message::ToolFunction::new(
                            "ReadFile".into(),
                            json!({"filename":"src/two.rs","offset":10,"limit":5}),
                        ),
                        signature: None,
                        additional_params: None,
                    }),
                ])
                .expect("multiple assistant content items"),
            },
            RigMessage::User {
                content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                    id: "tool-2".into(),
                    call_id: Some("call-2".into()),
                    content: OneOrMany::one(ToolResultContent::Text(Text { text: "ok".into() })),
                })),
            },
            RigMessage::User {
                content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                    id: "tool-1".into(),
                    call_id: Some("call-1".into()),
                    content: OneOrMany::one(ToolResultContent::Text(Text { text: "ok".into() })),
                })),
            },
            RigMessage::assistant("done"),
        ];

        let reduced = compact_tool_traces(&history);

        assert_eq!(
            reduced,
            vec![
                RigMessage::user("prompt"),
                RigMessage::assistant("Read `src/two.rs` lines 11-15."),
                RigMessage::assistant("Read `src/one.rs` lines 1-5."),
                RigMessage::assistant("done"),
            ]
        );
    }
}
