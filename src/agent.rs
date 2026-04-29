use crate::app::AccessMode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentRole {
    Main,
    Subagent,
    Critic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentContext {
    pub role: AgentRole,
    pub access_mode: AccessMode,
    pub model_name_override: Option<String>,
    pub reasoning_override: Option<crate::config::ReasoningSetting>,
    pub allow_full_system_access: bool,
}

impl AgentContext {
    #[cfg(test)]
    pub fn main(access_mode: AccessMode) -> Self {
        Self::main_with_full_system_access(access_mode, false)
    }

    pub fn main_with_full_system_access(
        access_mode: AccessMode,
        allow_full_system_access: bool,
    ) -> Self {
        Self {
            role: AgentRole::Main,
            access_mode,
            model_name_override: None,
            reasoning_override: None,
            allow_full_system_access,
        }
    }

    #[cfg(test)]
    pub fn subagent(access_mode: AccessMode, model_name_override: Option<String>) -> Self {
        Self::subagent_with_full_system_access(access_mode, model_name_override, false)
    }

    pub fn subagent_with_full_system_access(
        access_mode: AccessMode,
        model_name_override: Option<String>,
        allow_full_system_access: bool,
    ) -> Self {
        Self {
            role: AgentRole::Subagent,
            access_mode,
            model_name_override,
            reasoning_override: None,
            allow_full_system_access,
        }
    }

    pub fn critic_with_full_system_access(
        model_name_override: Option<String>,
        reasoning_override: Option<crate::config::ReasoningSetting>,
        allow_full_system_access: bool,
    ) -> Self {
        Self {
            role: AgentRole::Critic,
            access_mode: AccessMode::ReadOnly,
            model_name_override,
            reasoning_override,
            allow_full_system_access,
        }
    }
}
