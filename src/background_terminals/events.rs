use crate::app::ActivityDisplayState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BackgroundTerminalUiEvent {
    Spawned {
        id: String,
        label: String,
        cwd: String,
        pid: Option<u32>,
    },
    StateChanged {
        id: String,
        label: String,
        state: ActivityDisplayState,
        status_text: String,
        detail_text: Option<String>,
    },
}
