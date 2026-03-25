use crate::{app::App, command_history::CommandHistoryStore};

pub(crate) fn persist_command_history_if_needed(app: &mut App, store: &CommandHistoryStore) {
    let Some(entries) = app.take_command_history_to_persist() else {
        return;
    };

    if let Err(error) = store.save(&entries) {
        app.push_error_message(format!("Failed to save input history: {error}"));
    }
}
