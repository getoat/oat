use crate::app::{SideChannelEvent, StreamEvent};

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
}
