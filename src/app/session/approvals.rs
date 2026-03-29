use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
pub enum CommandRisk {
    Low,
    Medium,
    High,
}

impl CommandRisk {
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum WriteApprovalDecision {
    AllowOnce,
    AllowAllSession,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWriteApproval {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: String,
    pub summary: String,
    pub target: Option<String>,
    pub source_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingShellApproval {
    pub request_id: String,
    pub risk: CommandRisk,
    pub risk_explanation: String,
    pub command: String,
    pub working_directory: String,
    pub reason: String,
    pub source_label: Option<String>,
}

impl PendingShellApproval {
    pub fn new(
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
        source_label: Option<String>,
    ) -> Self {
        Self {
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
            source_label,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum ShellApprovalDecision {
    AllowOnce,
    AllowPattern(String),
    AllowAllRisk,
    Deny(Option<String>),
}

pub fn default_shell_approval_pattern(command: &str) -> String {
    let first_line = command.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return command.trim().to_string();
    }

    let tokens = shell_command_prefix_tokens(first_line);
    let word_tokens = tokens
        .iter()
        .copied()
        .filter(|token| !is_shell_redirection_token(token))
        .collect::<Vec<_>>();
    let has_extra_shell_syntax = command.lines().nth(1).is_some()
        || tokens.iter().any(|token| is_shell_redirection_token(token));

    match word_tokens.as_slice() {
        [] => first_line.to_string(),
        [single] if has_extra_shell_syntax => format!("{single} *"),
        [single] => (*single).to_string(),
        many => format!("{} *", many[..many.len() - 1].join(" ")),
    }
}

fn shell_command_prefix_tokens(line: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < line.len() {
        let ch = line[index..]
            .chars()
            .next()
            .expect("valid char boundary while tokenizing shell command");
        if ch.is_whitespace() {
            index += ch.len_utf8();
            continue;
        }
        if starts_with_shell_control_operator(&line[index..]) {
            break;
        }

        let start = index;
        let mut in_single_quotes = false;
        let mut in_double_quotes = false;
        let mut escaped = false;

        while index < line.len() {
            let ch = line[index..]
                .chars()
                .next()
                .expect("valid char boundary while scanning shell token");

            if escaped {
                escaped = false;
                index += ch.len_utf8();
                continue;
            }

            if !in_single_quotes && ch == '\\' {
                escaped = true;
                index += ch.len_utf8();
                continue;
            }

            if !in_double_quotes && ch == '\'' {
                in_single_quotes = !in_single_quotes;
                index += ch.len_utf8();
                continue;
            }

            if !in_single_quotes && ch == '"' {
                in_double_quotes = !in_double_quotes;
                index += ch.len_utf8();
                continue;
            }

            if !in_single_quotes && !in_double_quotes {
                if ch.is_whitespace() || starts_with_shell_control_operator(&line[index..]) {
                    break;
                }
            }

            index += ch.len_utf8();
        }

        tokens.push(&line[start..index]);

        if starts_with_shell_control_operator(&line[index..]) {
            break;
        }
    }

    tokens
}

fn starts_with_shell_control_operator(input: &str) -> bool {
    input.starts_with("&&")
        || input.starts_with("||")
        || input.starts_with('|')
        || input.starts_with(';')
        || input.starts_with('&')
}

fn is_shell_redirection_token(token: &str) -> bool {
    let trimmed = token.trim_start_matches(|ch: char| ch.is_ascii_digit());
    trimmed.starts_with('<') || trimmed.starts_with('>')
}
