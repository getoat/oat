use ratatui::{
    Frame,
    layout::Alignment,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::{
    app::{App, ModelPickerTab, SlashCommand, query},
    composer::ComposerLayout,
    ui::markdown::loading_frame,
};

use crate::ui::wrap::wrap_text;

pub(super) fn collect_line_range(line: &str, start_col: usize, end_col: usize) -> String {
    line.chars()
        .skip(start_col)
        .take(end_col.saturating_sub(start_col))
        .collect()
}

pub(super) fn composer_content_width(outer_width: u16) -> usize {
    outer_width.saturating_sub(4).max(1) as usize
}

pub(super) fn ask_user_state(
    app: &App,
) -> Option<(&crate::app::PendingAskUser, &crate::app::ui::AskUserUiState)> {
    Some((
        query::ask_user_session(app.state())?,
        query::ask_user_ui(app.state())?,
    ))
}

pub(super) fn shell_approval_state(
    app: &App,
) -> Option<(
    &crate::app::PendingShellApproval,
    &crate::app::ui::ShellApprovalUiState,
)> {
    Some((
        query::shell_approval_session(app.state())?,
        query::shell_approval_ui(app.state())?,
    ))
}

pub(super) fn render_detail_lines(lines: Vec<Line<'static>>, editing: bool) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::styled(
                if editing { "› " } else { "  " },
                Style::default().fg(Color::DarkGray),
            )];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

pub(super) fn command_block_style() -> Style {
    Style::default().bg(Color::Rgb(24, 24, 24))
}

pub(super) fn render_static_text_lines(
    text: &str,
    content_width: usize,
    style: Option<Style>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        let wrapped = if raw_line.is_empty() {
            vec![String::new()]
        } else {
            wrap_text(raw_line, content_width.max(1))
        };
        match style {
            Some(style) => lines.extend(wrapped.into_iter().map(|row| {
                let content = format!(" {row} ");
                Line::from(Span::styled(content, style))
            })),
            None => lines.extend(wrapped.into_iter().map(Line::from)),
        }
    }

    if lines.is_empty() {
        vec![Line::default()]
    } else {
        lines
    }
}

pub(super) fn render_aux_textarea_lines(
    textarea: &ratatui_textarea::TextArea<'_>,
    content_width: usize,
    accent: Color,
    show_cursor: bool,
) -> Vec<Line<'static>> {
    let show_placeholder =
        textarea.lines() == [String::new()] && !textarea.placeholder_text().is_empty();
    if show_placeholder {
        let placeholder = textarea.placeholder_text().to_owned();
        let placeholder_style = Style::default().fg(Color::DarkGray);
        let cursor_style = Style::default().bg(accent).fg(Color::Black);
        let placeholder_rows = if content_width <= 1 {
            vec![String::new()]
        } else {
            wrap_text(&placeholder, content_width.saturating_sub(1))
        };
        let mut lines = Vec::new();
        for (index, row) in placeholder_rows.into_iter().enumerate() {
            if show_cursor && index == 0 {
                let mut spans = vec![Span::styled(" ", cursor_style)];
                if !row.is_empty() {
                    spans.push(Span::styled(row, placeholder_style));
                }
                lines.push(Line::from(spans));
            } else {
                lines.push(Line::from(Span::styled(row, placeholder_style)));
            }
        }
        if lines.is_empty() {
            lines.push(Line::default());
        }
        return lines;
    }

    if textarea.lines() == [String::new()] {
        return vec![Line::from(Span::styled(
            " ",
            if show_cursor {
                Style::default().bg(accent).fg(Color::Black)
            } else {
                Style::default()
            },
        ))];
    }

    let layout = ComposerLayout::new(textarea.lines(), content_width.max(1));
    let cursor = show_cursor
        .then(|| layout.cursor_state(textarea.cursor()))
        .flatten();
    let base_style = Style::default();
    let cursor_style = Style::default().bg(accent).fg(Color::Black);
    let mut lines = Vec::new();
    for (index, row) in layout.rows().iter().enumerate() {
        let cursor_col = cursor
            .as_ref()
            .filter(|state| state.row_index == index)
            .map(|state| state.visual_col);
        lines.push(super::input::render_composer_row(
            &textarea.lines()[row.line_index],
            row.start_col,
            row.end_col,
            cursor_col,
            base_style,
            cursor_style,
        ));
    }
    if lines.is_empty() {
        vec![Line::default()]
    } else {
        lines
    }
}

pub(super) fn model_picker_tab_line(active_tab: ModelPickerTab, accent: Color) -> Line<'static> {
    let normal_style = if active_tab == ModelPickerTab::NormalAgent {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let planning_style = if active_tab == ModelPickerTab::PlanningAgents {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let safety_style = if active_tab == ModelPickerTab::SafetyModel {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let memory_style = if active_tab == ModelPickerTab::MemoryModel {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    Line::from(vec![
        Span::styled("Normal agent", normal_style),
        Span::styled("  |  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Planning agents", planning_style),
        Span::styled("  |  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Safety model", safety_style),
        Span::styled("  |  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Memory model", memory_style),
    ])
}

pub(super) fn mode_status_label(
    mode: crate::app::AccessMode,
    approval_mode: crate::app::ApprovalMode,
    plan_active: bool,
) -> &'static str {
    if plan_active {
        return "Plan";
    }

    match (mode, approval_mode) {
        (crate::app::AccessMode::ReadWrite, crate::app::ApprovalMode::Disabled) => "Write (!)",
        _ => mode.label(),
    }
}

pub(super) fn render_mode(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    accent: Color,
) {
    let mode_label = mode_status_label(
        query::mode(app.state()),
        query::approval_mode(app.state()),
        query::plan_active(app.state()),
    );
    let session_stats = query::session_stats(app.state());
    let context_percent = query::next_request_context_percent_state(app.state());
    let model_name = display_model_name(query::model_name(app.state()));

    let mut spans = vec![Span::styled(
        mode_label,
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )];
    if query::pending_write_approval(app.state()).is_none()
        && !query::has_pending_shell_approval(app.state())
        && query::history_is_pinned(app.state())
    {
        spans.push(Span::raw("  "));
        spans.push(Span::styled("Pinned", Style::default().fg(Color::Gray)));
    }
    spans.push(Span::raw(format!(
        "  {} • {}  in {}  out {}  ctx {}  ${:.6}",
        model_name,
        query::reasoning(app.state()).as_str(),
        format_compact_tokens(session_stats.input_tokens),
        format_compact_tokens(session_stats.output_tokens),
        format!("{context_percent}%"),
        session_stats.estimated_cost_usd(),
    )));
    let active_terminal_count = query::active_background_terminal_count(app.state());
    if active_terminal_count > 0 {
        let label = if active_terminal_count == 1 {
            "1 terminal active".to_string()
        } else {
            format!("{active_terminal_count} terminals active")
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(label, Style::default().fg(Color::Cyan)));
    }

    if let Some(pending) = query::pending_write_approval(app.state()) {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!(
                "Approval pending: {}{}",
                pending.tool_name,
                pending
                    .source_label
                    .as_ref()
                    .map(|source| format!(" from {source}"))
                    .unwrap_or_default()
            ),
            Style::default().fg(Color::Yellow),
        ));
    } else if let Some((pending, _)) = shell_approval_state(app) {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("Approval pending: {} risk shell", pending.risk.label()),
            Style::default().fg(Color::Yellow),
        ));
    } else if matches!(query::mode(app.state()), crate::app::AccessMode::ReadOnly) {
        spans.push(Span::styled(
            "  Tab switches to write mode for edits and higher-risk shell commands",
            Style::default().fg(Color::Gray),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub(super) fn render_top_status_bar(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    accent: Color,
) {
    let title = query::session_title(app.state())
        .map(str::to_string)
        .or_else(|| query::session_title_pending(app.state()).then(|| loading_frame(app).into()))
        .unwrap_or_default();
    let bar = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(accent))
        .title_bottom(
            Line::from(Span::styled(
                format!(" {title} "),
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
        );
    frame.render_widget(bar, area);
}

pub(super) fn command_palette_line(
    command: SlashCommand,
    selected: Option<SlashCommand>,
    accent: Color,
) -> Line<'static> {
    let is_selected = Some(command) == selected;
    let marker = if is_selected { "›" } else { " " };
    let name_style = if is_selected {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };

    let mut spans = vec![
        Span::styled(marker, name_style),
        Span::raw(" "),
        Span::styled(command.canonical_name(), name_style),
        Span::raw("  "),
        Span::styled(command.description(), Style::default().fg(Color::Gray)),
    ];
    if !command.aliases().is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("aliases: {}", command.aliases().join(", ")),
            Style::default().fg(Color::DarkGray),
        ));
    }

    Line::from(spans)
}

pub(super) fn selection_picker_line(
    is_selected: bool,
    label: impl Into<String>,
    detail: impl Into<String>,
    accent: Color,
) -> Line<'static> {
    selection_picker_line_with_label_width(is_selected, label, detail, 0, accent)
}

pub(super) fn selection_picker_line_with_label_width(
    is_selected: bool,
    label: impl Into<String>,
    detail: impl Into<String>,
    label_width: usize,
    accent: Color,
) -> Line<'static> {
    let marker = if is_selected { ">" } else { " " };
    let label = label.into();
    let name_style = if is_selected {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    let label = if label_width == 0 {
        label
    } else {
        format!("{label:<label_width$}")
    };

    Line::from(vec![
        Span::styled(marker, name_style),
        Span::raw(" "),
        Span::styled(label, name_style),
        Span::raw("  "),
        Span::styled(detail.into(), Style::default().fg(Color::Gray)),
    ])
}

pub(super) fn model_picker_header_line(name_width: usize) -> Line<'static> {
    let detail = format!(
        "{:<14} {:>7} {:>7} {:>8} {:>7}",
        "provider", "ctx", "$in", "$cache", "$out"
    );
    let prefix = " ".repeat(2 + name_width + 2);
    Line::from(Span::styled(
        format!("{prefix}{detail}"),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))
}

pub(super) fn model_picker_detail(model: &crate::model_registry::ModelInfo) -> String {
    let standard = format!(
        "{:<14} {:>7} {:>7} {:>8} {:>7}",
        model.provider.display_name(),
        model
            .display_context_length()
            .map(str::to_string)
            .unwrap_or_else(|| format_context_length(model.context_length)),
        format_price(model.pricing.input_per_million_tokens),
        format_price(model.pricing.cache_read_per_million_tokens),
        format_price(model.pricing.output_per_million_tokens),
    );

    if let Some(long_context) = model.long_context_pricing {
        format!(
            "{standard}  >{}: $in {}  $cache {}  $out {}",
            format_context_length(long_context.input_tokens_threshold),
            format_price(long_context.pricing.input_per_million_tokens),
            format_price(long_context.pricing.cache_read_per_million_tokens),
            format_price(long_context.pricing.output_per_million_tokens),
        )
    } else {
        standard
    }
}

pub(super) fn display_model_name(model_name: &str) -> String {
    crate::codex::display_name(model_name)
}

pub(super) fn format_context_length(context_length: usize) -> String {
    if context_length >= 1_000_000 {
        format!("{:.2}M", context_length as f64 / 1_000_000.0)
    } else if context_length >= 1_000 {
        format!("{}K", context_length / 1_000)
    } else {
        context_length.to_string()
    }
}

pub(super) fn format_price(price: f64) -> String {
    if price == 0.0 {
        "0".to_string()
    } else if price < 0.1 {
        format!("{price:.3}")
    } else {
        format!("{price:.2}")
    }
}

pub(super) fn format_compact_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.2}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests;
