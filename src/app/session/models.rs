use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::token_counting::count_text_tokens;
use crate::{
    app::HostedToolKind,
    ask_user::AskUserRequest,
    config::{ReasoningEffort, ReasoningSetting},
    model_registry,
    task::ActiveTask,
    todo::TodoSnapshot,
    tools::TaskUpdate,
};

use super::CommandRisk;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlashCommand {
    NewSession,
    Resume,
    Btw,
    Compact,
    Memory,
    Stats,
    Model,
    Effort,
    Login,
    Logout,
    Terminals,
    Terminal,
    KillTerminal,
    Plan,
    Quit,
}

impl SlashCommand {
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::NewSession => "/new",
            Self::Resume => "/resume",
            Self::Btw => "/btw",
            Self::Compact => "/compact",
            Self::Memory => "/memory",
            Self::Stats => "/stats",
            Self::Model => "/model",
            Self::Effort => "/effort",
            Self::Login => "/login",
            Self::Logout => "/logout",
            Self::Terminals => "/terminals",
            Self::Terminal => "/terminal",
            Self::KillTerminal => "/kill-terminal",
            Self::Plan => "/plan",
            Self::Quit => "/quit",
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::NewSession => &["/clear"],
            Self::Resume => &["/sessions"],
            Self::Btw => &[],
            Self::Compact => &[],
            Self::Memory => &[],
            Self::Stats => &["/status"],
            Self::Model => &["/models"],
            Self::Effort => &["/reasoning", "/thinking"],
            Self::Login => &[],
            Self::Logout => &[],
            Self::Terminals => &[],
            Self::Terminal => &[],
            Self::KillTerminal => &[],
            Self::Plan => &[],
            Self::Quit => &["/exit"],
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::NewSession => "Start a new session",
            Self::Resume => "Resume a previous session",
            Self::Btw => "Ask a side question without affecting history",
            Self::Compact => "Compact the internal model history",
            Self::Memory => "Search and manage long-term memories",
            Self::Stats => "Show session and historical usage stats",
            Self::Model => "Select the model and reasoning setting",
            Self::Effort => "Set reasoning for the current model",
            Self::Login => "Authenticate Codex device login",
            Self::Logout => "Clear stored Codex credentials",
            Self::Terminals => "List background terminals",
            Self::Terminal => "Inspect a background terminal",
            Self::KillTerminal => "Stop a background terminal",
            Self::Plan => "Start an interactive planning session",
            Self::Quit => "Exit the app",
        }
    }

    pub fn all_names(self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.canonical_name()).chain(self.aliases().iter().copied())
    }

    pub fn matches_exact(self, query: &str) -> bool {
        self.all_names()
            .any(|name| name.eq_ignore_ascii_case(query))
    }

    fn matches_prefix(self, query: &str) -> bool {
        let query = query.to_ascii_lowercase();
        self.all_names()
            .any(|name| name.to_ascii_lowercase().starts_with(&query))
    }

    pub fn filtered(query: &str) -> Vec<Self> {
        COMMANDS
            .into_iter()
            .filter(|command| command.matches_prefix(query))
            .collect()
    }
}

const COMMANDS: [SlashCommand; 15] = [
    SlashCommand::NewSession,
    SlashCommand::Resume,
    SlashCommand::Btw,
    SlashCommand::Compact,
    SlashCommand::Memory,
    SlashCommand::Stats,
    SlashCommand::Model,
    SlashCommand::Effort,
    SlashCommand::Login,
    SlashCommand::Logout,
    SlashCommand::Terminals,
    SlashCommand::Terminal,
    SlashCommand::KillTerminal,
    SlashCommand::Plan,
    SlashCommand::Quit,
];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum AccessMode {
    ReadOnly,
    ReadWrite,
}

impl AccessMode {
    pub fn toggle(&mut self) {
        *self = match self {
            Self::ReadOnly => Self::ReadWrite,
            Self::ReadWrite => Self::ReadOnly,
        };
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read-only",
            Self::ReadWrite => "Write",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum SessionProfile {
    Normal,
    Planning,
    Subagent,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum ApprovalMode {
    Manual,
    Disabled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum Speaker {
    User,
    Agent,
}

impl Speaker {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "you",
            Self::Agent => "oat",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorKey {
    Char(char),
    F(u8),
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    Tab,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Esc,
    Null,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorInput {
    pub key: EditorKey,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionHistoryMessage {
    pub payload: serde_json::Value,
    pub estimated_tokens: u64,
}

impl SessionHistoryMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self::text_message("user", text.into())
    }

    #[cfg(test)]
    pub fn assistant(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            payload: json!({
                "role": "assistant",
                "id": null,
                "content": [
                    {
                        "type": "text",
                        "text": text,
                    }
                ],
            }),
            estimated_tokens: estimated_text_content_message_tokens(&text),
        }
    }

    fn text_message(role: &str, text: String) -> Self {
        Self {
            payload: json!({
                "role": role,
                "content": [
                    {
                        "type": "text",
                        "text": text,
                    }
                ],
            }),
            estimated_tokens: estimated_text_content_message_tokens(&text),
        }
    }
}

fn estimated_text_content_message_tokens(text: &str) -> u64 {
    ESTIMATED_MESSAGE_OVERHEAD_TOKENS + ESTIMATED_CONTENT_OVERHEAD_TOKENS + count_text_tokens(text)
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    SessionTitleGenerated(String),
    TextDelta(String),
    ReasoningDelta(String),
    Commentary(String),
    ToolCall {
        name: String,
        arguments: String,
    },
    HostedToolStarted {
        id: String,
        kind: HostedToolKind,
        detail: String,
    },
    HostedToolCompleted {
        id: String,
        kind: HostedToolKind,
        detail: String,
    },
    ToolResult {
        name: String,
        output: String,
    },
    TodoSnapshot(TodoSnapshot),
    TaskUpdated {
        update: TaskUpdate,
        snapshot: Option<ActiveTask>,
    },
    AskUserRequested {
        request_id: String,
        request: AskUserRequest,
    },
    WriteApprovalRequested {
        request_id: String,
        tool_name: String,
        arguments: String,
    },
    ShellApprovalRequested {
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
    },
    PlanningFinalizationStarted,
    CompactionFinished {
        history: Vec<SessionHistoryMessage>,
        model_name: String,
    },
    TurnEnded {
        reason: TurnEndReason,
        history: Option<Vec<SessionHistoryMessage>>,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SideChannelEvent {
    Finished { output: String },
    Failed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnEndReason {
    Completed,
    InterruptedAtStepBoundary,
}

const ESTIMATED_MESSAGE_OVERHEAD_TOKENS: u64 = 4;
const ESTIMATED_CONTENT_OVERHEAD_TOKENS: u64 = 2;

pub(crate) fn compatible_reasoning_setting(
    model_name: &str,
    current: ReasoningSetting,
) -> ReasoningSetting {
    if let Some(model) = model_registry::find_model(model_name) {
        if model.supports_reasoning(current) {
            current
        } else {
            model
                .supported_reasoning_settings
                .iter()
                .find(|setting| **setting == ReasoningSetting::Gpt(ReasoningEffort::Medium))
                .copied()
                .or_else(|| model.supported_reasoning_settings.first().copied())
                .unwrap_or(current)
        }
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatible_reasoning_setting_preserves_supported_level() {
        assert_eq!(
            compatible_reasoning_setting("gpt-5.4", ReasoningSetting::Gpt(ReasoningEffort::High)),
            ReasoningSetting::Gpt(ReasoningEffort::High)
        );
    }

    #[test]
    fn compatible_reasoning_setting_downgrades_to_medium_when_needed() {
        assert_eq!(
            compatible_reasoning_setting(
                "gpt-5.4-mini",
                ReasoningSetting::Gpt(ReasoningEffort::Minimal),
            ),
            ReasoningSetting::Gpt(ReasoningEffort::Medium)
        );
    }
}
