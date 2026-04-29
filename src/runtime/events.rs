use crate::{
    app::{SideChannelEvent, StreamEvent},
    codex::DeviceCodeSession,
    config::CodexConfig,
    llm::CriticVerdict,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RuntimeEvent {
    MainReply {
        reply_id: u64,
        event: StreamEvent,
    },
    SideChannel {
        reply_id: u64,
        event: SideChannelEvent,
    },
    CriticFinished {
        reply_id: u64,
        result: Result<CriticVerdict, String>,
    },
    CodexLoginStarted {
        session: DeviceCodeSession,
    },
    CodexLoginCompleted {
        result: Result<CodexConfig, String>,
    },
}
