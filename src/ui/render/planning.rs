use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::{
    app::{App, query},
    ui::wrap::wrap_text,
};

use super::helpers::composer_content_width;

pub(super) fn render_plan_review_prompt(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let selected_index = query::selected_plan_review_index(app.state()).unwrap_or(0);
    let selected_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
    let unselected_style = Style::default().fg(Color::Gray);
    let lines = vec![
        Line::from(Span::styled(
            "A proposed plan is ready.",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                if selected_index == 0 {
                    "› [1]"
                } else {
                    "  [1]"
                },
                if selected_index == 0 {
                    selected_style
                } else {
                    unselected_style
                },
            ),
            Span::styled(
                " Accept this plan and begin implementation",
                if selected_index == 0 {
                    selected_style
                } else {
                    Style::default()
                },
            ),
        ]),
        Line::from(vec![
            Span::styled(
                if selected_index == 1 {
                    "› [2]"
                } else {
                    "  [2]"
                },
                if selected_index == 1 {
                    selected_style
                } else {
                    unselected_style
                },
            ),
            Span::styled(
                " Suggest changes to the plan",
                if selected_index == 1 {
                    selected_style
                } else {
                    Style::default()
                },
            ),
        ]),
    ];

    let prompt = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .title(" Plan Ready ")
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .border_style(Style::default().fg(accent)),
    );
    frame.render_widget(prompt, area);
}

pub(super) fn pending_plan_review_height(panel_width: u16) -> u16 {
    let content_width = composer_content_width(panel_width);
    let title_lines = wrap_text("A proposed plan is ready.", content_width.max(1)).len();
    let option_one_lines = wrap_text(
        "[1] Accept this plan and begin implementation",
        content_width.max(1),
    )
    .len();
    let option_two_lines = wrap_text("[2] Suggest changes to the plan", content_width.max(1)).len();

    (title_lines + option_one_lines + option_two_lines + 2) as u16
}

#[cfg(test)]
mod tests;
