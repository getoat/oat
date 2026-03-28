mod approvals;
mod ask_user;
mod helpers;
mod input;
mod overlay;
mod planning;
mod queue;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
};

use crate::app::{App, ops, query};

use approvals::{pending_shell_approval_height, pending_write_approval_height};
use ask_user::pending_ask_user_height;
use helpers::{composer_content_width, render_mode, render_top_status_bar};
use input::render_input;
use overlay::render_overlay;
use planning::pending_plan_review_height;
use queue::{queued_message_strip_height, render_queued_message_strip};

use super::{history::render_history, markdown::loading_frame, theme::accent_color};

pub fn render(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    ops::composer::set_composer_wrap_width(app.state_mut(), composer_content_width(screen.width));
    let accent = accent_color(query::mode(app.state()), query::plan_active(app.state()));
    let show_top_status_bar = !query::shows_startup_banner_state(app.state());
    let input_height = if let Some(pending) = query::pending_write_approval(app.state()) {
        pending_write_approval_height(pending, screen.width)
    } else if query::has_pending_shell_approval(app.state()) {
        pending_shell_approval_height(app, screen.width)
    } else if query::has_pending_ask_user(app.state()) {
        pending_ask_user_height(app, screen.width)
    } else if query::plan_review_selection_active(app.state()) {
        pending_plan_review_height(screen.width)
    } else {
        ops::composer::composer_height(app.state_mut()).max(3)
    };
    let overlay_height = query::overlay_height(app.state(), screen.height);
    let queue_height = queued_message_strip_height(app, screen.width, screen.height);
    let mut constraints = Vec::new();
    if show_top_status_bar {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Min(1));
    if overlay_height > 0 {
        constraints.push(Constraint::Length(overlay_height));
    }
    if queue_height > 0 {
        constraints.push(Constraint::Length(queue_height));
    }
    constraints.push(Constraint::Length(input_height));
    constraints.push(Constraint::Length(1));

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(screen);

    let mut section = 0;
    if show_top_status_bar {
        render_top_status_bar(frame, app, layout[section], accent);
        section += 1;
    }
    render_history(frame, app, layout[section], accent, loading_frame(app));
    section += 1;
    if overlay_height > 0 {
        render_overlay(frame, app, layout[section], accent);
        section += 1;
    }
    if queue_height > 0 {
        render_queued_message_strip(frame, app, layout[section], accent);
        section += 1;
    }
    render_input(frame, app, layout[section], accent);
    render_mode(frame, app, layout[section + 1], accent);
}

#[cfg(test)]
mod root_tests;
#[cfg(test)]
pub(crate) mod test_support;
