use crate::{
    app::{App, ops},
    command_history::CommandHistoryStore,
};

pub(crate) fn persist_command_history_if_needed(app: &mut App, store: &CommandHistoryStore) {
    let Some(entries) = ops::session::take_command_history_to_persist(app.state_mut()) else {
        return;
    };

    if let Err(error) = store.save(&entries) {
        ops::transcript::push_error_message(
            app.state_mut(),
            format!("Failed to save input history: {error}"),
        );
    }
}
