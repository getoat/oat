use crate::{
    app::{SideChannelEvent, StreamEvent},
    codex::DeviceCodeSession,
    config::CodexConfig,
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
    CodexLoginStarted {
        session: DeviceCodeSession,
    },
    CodexLoginCompleted {
        result: Result<CodexConfig, String>,
    },
}
