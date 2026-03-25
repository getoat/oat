use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::{app::App, ui::wrap::wrap_text};

use super::helpers::{
    ask_user_state, composer_content_width, render_aux_textarea_lines, render_detail_lines,
};

pub(super) fn render_ask_user_prompt(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let Some((pending, ui)) = ask_user_state(app) else {
        return;
    };
    let content_width = composer_content_width(area.width);
    let prompt = Paragraph::new(ask_user_panel_lines(pending, ui, content_width, accent))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(format!(" {} ", pending.title))
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .border_style(Style::default().fg(accent)),
        );
    frame.render_widget(prompt, area);
}

pub(super) fn pending_ask_user_height(app: &App, panel_width: u16) -> u16 {
    let Some((pending, ui)) = ask_user_state(app) else {
        return 0;
    };
    let content_width = composer_content_width(panel_width);
    let mut lines = 1usize + 1 + 1;
    if ui.active_tab < pending.questions.len() {
        let question = &pending.questions[ui.active_tab];
        lines += wrap_text(&question.prompt, content_width.max(1)).len();
        lines += question.answers.len();
        lines += 2;
        let selected = &question.answers[question.selected_index];
        let detail_text = ui.detail_text(ui.active_tab);
        if ui.detail_editing || selected.is_something_else || !detail_text.trim().is_empty() {
            lines += 1;
            let detail_input = ui
                .detail_inputs
                .get(ui.active_tab)
                .expect("detail input should exist for active tab");
            lines +=
                crate::composer::ComposerLayout::new(detail_input.lines(), content_width.max(1))
                    .rows()
                    .len()
                    .max(1);
        }
        if selected.is_something_else && detail_text.trim().is_empty() {
            lines += 1;
        }
    } else {
        lines += 1;
        lines += pending.questions.len();
        for (index, _question) in pending.questions.iter().enumerate() {
            let detail_text = ui.detail_text(index);
            if !detail_text.is_empty() {
                lines += wrap_text(&format!("details: {detail_text}"), content_width.max(1)).len();
            }
        }
    }

    lines as u16 + 2
}

fn ask_user_panel_lines(
    pending: &crate::app::PendingAskUser,
    ui: &crate::app::ui::AskUserUiState,
    content_width: usize,
    accent: Color,
) -> Vec<Line<'static>> {
    let mut lines = vec![ask_user_tab_line(pending, ui, accent)];
    if ui.active_tab < pending.questions.len() {
        let question = &pending.questions[ui.active_tab];
        lines.push(Line::default());
        lines.extend(
            wrap_text(&question.prompt, content_width.max(1))
                .into_iter()
                .enumerate()
                .map(|(index, row)| {
                    let style = if index == 0 {
                        Style::default().fg(accent).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    Line::from(Span::styled(row, style))
                }),
        );
        lines.push(Line::default());

        for (index, answer) in question.answers.iter().enumerate() {
            let is_selected = index == question.selected_index;
            let marker_style = if is_selected {
                Style::default().fg(accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let answer_style = if is_selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let mut spans = vec![
                Span::styled(if is_selected { "›" } else { " " }, marker_style),
                Span::raw(" "),
                Span::styled(answer.label.clone(), answer_style),
            ];
            if answer.is_recommended {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    "Recommended",
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ));
            }
            lines.push(Line::from(spans));
        }

        let selected = &question.answers[question.selected_index];
        lines.push(Line::default());

        let detail_text = ui.detail_text(ui.active_tab);
        let show_detail =
            ui.detail_editing || selected.is_something_else || !detail_text.trim().is_empty();
        if !show_detail {
            lines.push(Line::from(Span::styled(
                if selected.is_something_else {
                    "Tab to enter required details for `Something else`."
                } else {
                    "Tab to add optional details for the selected answer."
                },
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                if ui.detail_editing {
                    "Details (editing)"
                } else {
                    "Details"
                },
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            )));
            let detail_input = ui
                .detail_inputs
                .get(ui.active_tab)
                .expect("detail input should exist for active tab");
            lines.extend(render_detail_lines(
                render_aux_textarea_lines(
                    detail_input,
                    content_width.saturating_sub(2).max(1),
                    accent,
                    ui.detail_editing,
                ),
                ui.detail_editing,
            ));
        }
        if selected.is_something_else && detail_text.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                "`Something else` requires details.",
                Style::default().fg(Color::Yellow),
            )));
        }
    } else {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "Review your answers and press Enter to submit.",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::default());

        for question in &pending.questions {
            let selected = &question.answers[question.selected_index];
            let detail_text = ui.detail_text(
                pending
                    .questions
                    .iter()
                    .position(|candidate| candidate.id == question.id)
                    .unwrap_or(0),
            );
            let complete = !selected.is_something_else || !detail_text.is_empty();
            let marker = if complete { "✓" } else { "!" };
            let marker_style = if complete {
                Style::default().fg(accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            };
            lines.push(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::raw(" "),
                Span::styled(
                    question.prompt.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    selected.label.clone(),
                    if complete {
                        Style::default()
                    } else {
                        Style::default().fg(Color::Yellow)
                    },
                ),
            ]));
            if !detail_text.is_empty() {
                lines.extend(
                    wrap_text(&format!("details: {detail_text}"), content_width.max(1))
                        .into_iter()
                        .map(|row| Line::from(Span::styled(row, Style::default().fg(Color::Gray)))),
                );
            }
        }
    }

    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "Left/Right switch questions  Up/Down switch answers  Tab edits details  Enter submits from Review",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn ask_user_tab_line(
    pending: &crate::app::PendingAskUser,
    ui: &crate::app::ui::AskUserUiState,
    accent: Color,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, question) in pending.questions.iter().enumerate() {
        if !spans.is_empty() {
            spans.push(Span::styled("  |  ", Style::default().fg(Color::DarkGray)));
        }
        let is_active = index == ui.active_tab;
        spans.push(Span::styled(
            question.id.clone(),
            if is_active {
                Style::default().fg(accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            },
        ));
    }
    if !spans.is_empty() {
        spans.push(Span::styled("  |  ", Style::default().fg(Color::DarkGray)));
    }
    let review_active = ui.active_tab == pending.questions.len();
    spans.push(Span::styled(
        "Review",
        if review_active {
            Style::default().fg(accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        },
    ));
    Line::from(spans)
}

#[cfg(test)]
mod tests;
