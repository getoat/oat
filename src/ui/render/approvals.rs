use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::{app::App, composer::ComposerLayout, ui::wrap::wrap_text};

use super::helpers::{
    command_block_style, composer_content_width, render_aux_textarea_lines, render_detail_lines,
    render_static_text_lines, shell_approval_state,
};

pub(super) fn render_write_approval_prompt(
    frame: &mut Frame,
    pending: &crate::app::PendingWriteApproval,
    area: Rect,
    accent: Color,
) {
    let mut lines = Vec::new();
    if let Some(source_label) = &pending.source_label {
        lines.push(Line::from(vec![
            Span::styled(
                "Source:",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(source_label.clone(), Style::default().fg(Color::Yellow)),
        ]));
    }
    lines.push(Line::from(Span::styled(
        pending.summary.clone(),
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )));

    lines.push(Line::from(vec![
        Span::styled(
            "[a]",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" allow once"),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "[s]",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" allow all this session"),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "[d]",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" deny"),
    ]));

    let prompt = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .title(" Write Approval Required ")
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(prompt, area);
}

pub(super) fn pending_write_approval_height(
    pending: &crate::app::PendingWriteApproval,
    panel_width: u16,
) -> u16 {
    let content_width = composer_content_width(panel_width);
    let source_lines = pending
        .source_label
        .as_ref()
        .map(|source| wrap_text(&format!("Source: {source}"), content_width.max(1)).len())
        .unwrap_or(0);
    let summary_lines = wrap_text(&pending.summary, content_width.max(1)).len();
    (source_lines + summary_lines + 3 + 2) as u16
}

pub(super) fn render_shell_approval_prompt(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    accent: Color,
) {
    let Some((pending, ui)) = shell_approval_state(app) else {
        return;
    };
    let content_width = composer_content_width(area.width);
    let selected_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
    let unselected_style = Style::default().fg(Color::Gray);
    let mut lines = Vec::new();
    if let Some(source_label) = &pending.source_label {
        lines.push(Line::from(vec![
            Span::styled(
                "Source:",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(source_label.clone(), Style::default().fg(Color::Yellow)),
        ]));
    }
    lines.push(Line::from(Span::styled(
        format!(
            "Risk: {}; {}",
            pending.risk.label(),
            pending.risk_explanation
        ),
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::raw(format!("Reason: {}", pending.reason))));
    lines.push(Line::from(Span::raw(format!(
        "Working directory: {}",
        pending.working_directory
    ))));
    lines.push(Line::from(Span::raw("Command:")));
    lines.extend(render_detail_lines(
        render_static_text_lines(
            &pending.command,
            content_width.saturating_sub(4).max(1),
            Some(command_block_style()),
        ),
        false,
    ));

    let option_style = |selected: bool| {
        if selected {
            selected_style
        } else {
            Style::default()
        }
    };
    let marker_style = |selected: bool| {
        if selected {
            selected_style
        } else {
            unselected_style
        }
    };

    lines.push(Line::from(vec![
        Span::styled(
            if ui.selected_index == 0 { "›" } else { " " },
            marker_style(ui.selected_index == 0),
        ),
        Span::raw(" "),
        Span::styled("Approve once", option_style(ui.selected_index == 0)),
    ]));

    lines.push(Line::from(vec![
        Span::styled(
            if ui.selected_index == 1 { "›" } else { " " },
            marker_style(ui.selected_index == 1),
        ),
        Span::raw(" "),
        Span::styled(
            "Approve commands starting with",
            option_style(ui.selected_index == 1),
        ),
    ]));
    lines.extend(render_detail_lines(
        render_aux_textarea_lines(
            &ui.pattern_input,
            content_width.saturating_sub(2).max(1),
            accent,
            ui.selected_index == 1,
        ),
        ui.selected_index == 1,
    ));

    lines.push(Line::from(vec![
        Span::styled(
            if ui.selected_index == 2 { "›" } else { " " },
            marker_style(ui.selected_index == 2),
        ),
        Span::raw(" "),
        Span::styled(
            format!("Approve all {} risk commands", pending.risk.as_str()),
            option_style(ui.selected_index == 2),
        ),
    ]));

    lines.push(Line::from(vec![
        Span::styled(
            if ui.selected_index == 3 { "›" } else { " " },
            marker_style(ui.selected_index == 3),
        ),
        Span::raw(" "),
        Span::styled("Deny", option_style(ui.selected_index == 3)),
    ]));

    let deny_text = ui.deny_input.lines().join("\n");
    if ui.edit_mode == Some(crate::app::ShellApprovalEditMode::Deny) || !deny_text.trim().is_empty()
    {
        lines.push(Line::from(Span::styled(
            if ui.edit_mode == Some(crate::app::ShellApprovalEditMode::Deny) {
                "Details (editing)"
            } else {
                "Details"
            },
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )));
        lines.extend(render_detail_lines(
            render_aux_textarea_lines(
                &ui.deny_input,
                content_width.saturating_sub(2).max(1),
                accent,
                ui.edit_mode == Some(crate::app::ShellApprovalEditMode::Deny),
            ),
            ui.edit_mode == Some(crate::app::ShellApprovalEditMode::Deny),
        ));
    } else if ui.selected_index == 3 {
        lines.push(Line::from(Span::styled(
            "Tab to add optional deny details.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::default());
    let hint = if ui.selected_index == 1 {
        "Use * as a wildcard. Up/Down switch options  Enter submits selected option"
    } else if ui.selected_index == 3 {
        "Tab edits deny details  Up/Down switch options  Enter submits selected option"
    } else {
        "Up/Down switch options  Enter submits selected option"
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().fg(Color::DarkGray),
    )));

    let prompt = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .title(" Shell Approval Required ")
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(prompt, area);
}

pub(super) fn pending_shell_approval_height(app: &App, panel_width: u16) -> u16 {
    let Some((pending, ui)) = shell_approval_state(app) else {
        return 0;
    };
    let content_width = composer_content_width(panel_width);
    let source_lines = pending
        .source_label
        .as_ref()
        .map(|source| wrap_text(&format!("Source: {source}"), content_width.max(1)).len())
        .unwrap_or(0);
    let base_lines = source_lines
        + wrap_text(
            &format!(
                "Risk: {}; {}",
                pending.risk.label(),
                pending.risk_explanation
            ),
            content_width.max(1),
        )
        .len()
        + wrap_text(&format!("Reason: {}", pending.reason), content_width.max(1)).len()
        + wrap_text(
            &format!("Working directory: {}", pending.working_directory),
            content_width.max(1),
        )
        .len()
        + 1
        + render_static_text_lines(
            &pending.command,
            content_width.saturating_sub(4).max(1),
            Some(command_block_style()),
        )
        .len()
        + 4;
    let pattern_lines = ComposerLayout::new(
        ui.pattern_input.lines(),
        content_width.saturating_sub(2).max(1),
    )
    .rows()
    .len()
    .max(1);
    let deny_text = ui.deny_input.lines().join("\n");
    let deny_lines = if ui.edit_mode == Some(crate::app::ShellApprovalEditMode::Deny)
        || !deny_text.trim().is_empty()
    {
        1 + ComposerLayout::new(
            ui.deny_input.lines(),
            content_width.saturating_sub(2).max(1),
        )
        .rows()
        .len()
        .max(1)
    } else if ui.selected_index == 3 {
        1
    } else {
        0
    };
    (base_lines + pattern_lines + deny_lines + 4) as u16
}

#[cfg(test)]
mod tests;
