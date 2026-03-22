use crate::app::AccessMode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentRole {
    Main,
    Subagent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentContext {
    pub role: AgentRole,
    pub access_mode: AccessMode,
    pub model_name_override: Option<String>,
}

impl AgentContext {
    pub fn main(access_mode: AccessMode) -> Self {
        Self {
            role: AgentRole::Main,
            access_mode,
            model_name_override: None,
        }
    }

    pub fn subagent(access_mode: AccessMode, model_name_override: Option<String>) -> Self {
        Self {
            role: AgentRole::Subagent,
            access_mode,
            model_name_override,
        }
    }
}
