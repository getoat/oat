#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlashCommand {
    NewSession,
    Compact,
    Stats,
    Model,
    Effort,
    Plan,
    Quit,
}

impl SlashCommand {
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::NewSession => "/new",
            Self::Compact => "/compact",
            Self::Stats => "/stats",
            Self::Model => "/model",
            Self::Effort => "/effort",
            Self::Plan => "/plan",
            Self::Quit => "/quit",
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::NewSession => &["/clear"],
            Self::Compact => &[],
            Self::Stats => &["/status"],
            Self::Model => &["/models"],
            Self::Effort => &["/reasoning", "/thinking"],
            Self::Plan => &[],
            Self::Quit => &["/exit"],
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::NewSession => "Start a new session",
            Self::Compact => "Compact the internal model history",
            Self::Stats => "Show session and historical usage stats",
            Self::Model => "Select the model and reasoning effort",
            Self::Effort => "Set reasoning effort for the current model",
            Self::Plan => "Start an interactive planning session",
            Self::Quit => "Exit the app",
        }
    }

    pub fn usage(self) -> Option<&'static str> {
        match self {
            Self::Model => Some("/model"),
            Self::Compact => Some("/compact"),
            Self::Effort => Some("/effort <minimal|low|medium|high|xhigh>"),
            Self::Plan => Some("/plan"),
            Self::NewSession | Self::Stats | Self::Quit => None,
        }
    }

    pub fn all_names(self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.canonical_name()).chain(self.aliases().iter().copied())
    }

    pub fn display_name(self) -> String {
        let aliases = self.aliases();
        if aliases.is_empty() {
            self.canonical_name().to_string()
        } else {
            format!("{} ({})", self.canonical_name(), aliases.join(", "))
        }
    }

    fn matches_prefix(self, query: &str) -> bool {
        let query = query.to_ascii_lowercase();
        self.all_names()
            .any(|name| name.to_ascii_lowercase().starts_with(&query))
    }

    pub fn matches_exact(self, query: &str) -> bool {
        let query = query.to_ascii_lowercase();
        self.all_names()
            .any(|name| name.eq_ignore_ascii_case(&query))
    }

    pub fn filtered(query: &str) -> Vec<Self> {
        COMMANDS
            .into_iter()
            .filter(|command| command.matches_prefix(query))
            .collect()
    }

    pub fn parse(query: &str) -> Option<Self> {
        COMMANDS
            .into_iter()
            .find(|command| command.matches_exact(query))
    }
}

const COMMANDS: [SlashCommand; 7] = [
    SlashCommand::NewSession,
    SlashCommand::Compact,
    SlashCommand::Stats,
    SlashCommand::Model,
    SlashCommand::Effort,
    SlashCommand::Plan,
    SlashCommand::Quit,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalMode {
    Manual,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionHistoryMessage {
    pub payload: serde_json::Value,
    pub estimated_tokens: u64,
}

impl SessionHistoryMessage {
    pub fn system(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            payload: json!({
                "role": "system",
                "content": text,
            }),
            estimated_tokens: ESTIMATED_MESSAGE_OVERHEAD_TOKENS + count_text_tokens(&text),
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::text_message("user", text.into())
    }

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
    TextDelta(String),
    ReasoningDelta(String),
    Commentary(String),
    ToolCall {
        name: String,
        arguments: String,
    },
    ToolResult {
        name: String,
        output: String,
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
    Finished {
        history: Option<Vec<SessionHistoryMessage>>,
    },
    Failed(String),
}
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    ask_user::AskUserRequest, config::ReasoningEffort, model_registry,
    token_counting::count_text_tokens,
};

use super::CommandRisk;

const ESTIMATED_MESSAGE_OVERHEAD_TOKENS: u64 = 4;
const ESTIMATED_CONTENT_OVERHEAD_TOKENS: u64 = 2;

pub(crate) fn compatible_reasoning_effort(
    model_name: &str,
    current: ReasoningEffort,
) -> ReasoningEffort {
    if let Some(model) = model_registry::find_model(model_name) {
        if model.supports_reasoning(current) {
            current
        } else {
            model
                .supported_reasoning_levels
                .iter()
                .find(|level| **level == ReasoningEffort::Medium)
                .copied()
                .or_else(|| model.supported_reasoning_levels.first().copied())
                .unwrap_or(current)
        }
    } else {
        current
    }
}
