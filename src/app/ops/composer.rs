use ratatui_textarea::CursorMove;

use crate::{
    app::{AppState, EditorInput, SlashCommand},
    composer::ComposerLayout,
};

pub(crate) fn composer_has_content(state: &AppState) -> bool {
    state
        .ui
        .composer
        .composer
        .lines()
        .iter()
        .any(|line| !line.is_empty())
}

pub(crate) fn submitted_composer_text(state: &AppState) -> String {
    state
        .ui
        .composer
        .composer
        .lines()
        .join("\n")
        .trim()
        .to_owned()
}

pub(crate) fn clear_composer(state: &mut AppState) {
    set_composer_text_internal(state, "", true);
}

pub(crate) fn set_composer_text(state: &mut AppState, text: &str) {
    set_composer_text_internal(state, text, true);
}

fn set_composer_text_internal(state: &mut AppState, text: &str, reset_command_history: bool) {
    let mut composer = crate::app::ui::new_composer_with_text(text);
    composer.move_cursor(CursorMove::End);
    state.ui.composer.composer = composer;
    state.ui.invalidate_composer_layout();
    state.ui.composer.visual_column = None;
    if reset_command_history {
        state.ui.command_history.reset_navigation();
    }
    sync_command_selection(state);
}

pub(crate) fn move_composer_cursor_up(state: &mut AppState) {
    let current_cursor = state.ui.composer.composer.cursor();
    let target = {
        let Some(cursor) = state.ui.composer_layout().cursor_state(current_cursor) else {
            return;
        };

        if cursor.row_index == 0 {
            if cursor.visual_col > 0 {
                Some((cursor.row.line_index, cursor.row.start_col, None))
            } else {
                None
            }
        } else {
            let desired_col = state.ui.composer.visual_column.unwrap_or(cursor.visual_col);
            state
                .ui
                .composer_layout()
                .target_cursor_for_row(cursor.row_index - 1, desired_col)
                .map(|(row, col)| (row, col, Some(desired_col)))
        }
    };

    match target {
        Some((row, col, desired_col)) => {
            state
                .ui
                .composer
                .composer
                .move_cursor(CursorMove::Jump(row as u16, col as u16));
            state.ui.composer.visual_column = desired_col;
        }
        None => {
            state.ui.composer.visual_column = None;
        }
    }
}

pub(crate) fn move_composer_cursor_down(state: &mut AppState) {
    let current_cursor = state.ui.composer.composer.cursor();
    let target = {
        let Some(cursor) = state.ui.composer_layout().cursor_state(current_cursor) else {
            return;
        };

        if cursor.row_index + 1 >= cursor.total_rows {
            if current_cursor.1 < cursor.row.end_col {
                Some((cursor.row.line_index, cursor.row.end_col, None))
            } else {
                None
            }
        } else {
            let desired_col = state.ui.composer.visual_column.unwrap_or(cursor.visual_col);
            state
                .ui
                .composer_layout()
                .target_cursor_for_row(cursor.row_index + 1, desired_col)
                .map(|(row, col)| (row, col, Some(desired_col)))
        }
    };

    match target {
        Some((row, col, desired_col)) => {
            state
                .ui
                .composer
                .composer
                .move_cursor(CursorMove::Jump(row as u16, col as u16));
            state.ui.composer.visual_column = desired_col;
        }
        None => {
            state.ui.composer.visual_column = None;
        }
    }
}

pub(crate) fn insert_composer_newline(state: &mut AppState) {
    state.ui.command_history.reset_navigation();
    state.ui.invalidate_composer_layout();
    state.ui.composer.visual_column = None;
    state.ui.composer.composer.insert_newline();
    sync_command_selection(state);
}

pub(crate) fn apply_composer_input(state: &mut AppState, input: EditorInput) {
    state.ui.command_history.reset_navigation();
    state.ui.invalidate_composer_layout();
    state.ui.composer.visual_column = None;
    state
        .ui
        .composer
        .composer
        .input(crate::app::ui::textarea_input(&input));
    sync_command_selection(state);
}

pub(crate) fn paste_into_composer(state: &mut AppState, text: &str) {
    state.ui.command_history.reset_navigation();
    state.ui.invalidate_composer_layout();
    state.ui.composer.visual_column = None;
    state
        .ui
        .composer
        .composer
        .insert_str(crate::app::ui::normalize_pasted_line_endings(text));
    sync_command_selection(state);
}

pub(crate) fn record_submitted_input(state: &mut AppState, text: &str) {
    state.ui.command_history.record(text);
}

pub(crate) fn should_recall_previous_input(state: &mut AppState) -> bool {
    let current_cursor = state.ui.composer.composer.cursor();
    state
        .ui
        .composer_layout()
        .cursor_state(current_cursor)
        .is_some_and(|cursor| cursor.row_index == 0 && cursor.visual_col == 0)
}

pub(crate) fn should_recall_next_input(state: &mut AppState) -> bool {
    let current_cursor = state.ui.composer.composer.cursor();
    state
        .ui
        .composer_layout()
        .cursor_state(current_cursor)
        .is_some_and(|cursor| {
            cursor.row_index + 1 >= cursor.total_rows && current_cursor.1 == cursor.row.end_col
        })
}

pub(crate) fn recall_previous_input(state: &mut AppState) -> bool {
    let current = state.ui.composer.composer.lines().join("\n");
    let Some(previous) = state.ui.command_history.previous(&current) else {
        return false;
    };
    set_composer_text_internal(state, &previous, false);
    true
}

pub(crate) fn recall_next_input(state: &mut AppState) -> bool {
    let Some(next) = state.ui.command_history.next() else {
        return false;
    };
    set_composer_text_internal(state, &next, false);
    true
}

pub(crate) fn command_query(state: &AppState) -> Option<&str> {
    let [line] = state.ui.composer.composer.lines() else {
        return None;
    };

    line.starts_with('/').then_some(line.as_str())
}

pub(crate) fn command_name(state: &AppState) -> Option<&str> {
    command_query(state)
        .map(crate::app::ui::split_command_query)
        .map(|(name, _)| name)
}

pub(crate) fn command_arguments(state: &AppState) -> Option<&str> {
    command_query(state)
        .map(crate::app::ui::split_command_query)
        .map(|(_, args)| args)
}

pub(crate) fn filtered_commands(state: &AppState) -> Vec<SlashCommand> {
    command_name(state)
        .map(SlashCommand::filtered)
        .unwrap_or_default()
}

pub(crate) fn selected_command(state: &AppState) -> Option<SlashCommand> {
    let commands = filtered_commands(state);
    commands
        .contains(&state.ui.selected_command)
        .then_some(state.ui.selected_command)
        .or_else(|| commands.first().copied())
}

pub(crate) fn move_command_selection_up(state: &mut AppState) {
    move_command_selection(state, -1);
}

pub(crate) fn move_command_selection_down(state: &mut AppState) {
    move_command_selection(state, 1);
}

fn move_command_selection(state: &mut AppState, direction: isize) {
    let commands = filtered_commands(state);
    if commands.is_empty() {
        return;
    }

    let current_index = commands
        .iter()
        .position(|command| *command == state.ui.selected_command)
        .unwrap_or(0);
    let next_index = (current_index as isize + direction).rem_euclid(commands.len() as isize);
    state.ui.selected_command = commands[next_index as usize];
}

pub(crate) fn sync_command_selection(state: &mut AppState) {
    let commands = filtered_commands(state);
    if let Some(command) = commands.first().copied()
        && !commands.contains(&state.ui.selected_command)
    {
        state.ui.selected_command = command;
    }
}

pub(crate) fn set_composer_wrap_width(state: &mut AppState, width: usize) {
    let width = width.max(1);
    if state.ui.composer.wrap_width != width {
        state.ui.composer.wrap_width = width;
        state.ui.invalidate_composer_layout();
        state.ui.composer.visual_column = None;
    }
}

pub(crate) fn composer_height(state: &mut AppState) -> u16 {
    state.ui.composer_layout().height().saturating_add(2) as u16
}

pub(crate) fn composer_layout(state: &mut AppState) -> &ComposerLayout {
    state.ui.composer_layout()
}

#[cfg(test)]
pub(crate) fn set_composer_cursor(state: &mut AppState, row: u16, col: u16) {
    state
        .ui
        .composer
        .composer
        .move_cursor(CursorMove::Jump(row, col));
}
