use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    app::{App, StatsScreenState, StatsScreenTab, StatsTableRow},
    stats::StatsTotals,
    ui::scrollbar::render_vertical_scrollbar,
};

use super::helpers::{format_compact_tokens, format_price, tab_line};

const MODEL_COLUMN_WIDTH: u16 = 14;
const REQUESTS_COLUMN_WIDTH: u16 = 4;
const TOOLS_COLUMN_WIDTH: u16 = 5;
const TOKEN_COLUMN_WIDTH: u16 = 7;
const DURATION_COLUMN_WIDTH: u16 = 8;
const THROUGHPUT_COLUMN_WIDTH: u16 = 7;
const COST_COLUMN_WIDTH: u16 = 9;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModelTableColumn {
    Model,
    Requests,
    Tools,
    Input,
    Cached,
    Output,
    Thinking,
    AvgTtfb,
    AvgTime,
    TokensPerSecond,
    Cost,
}

impl ModelTableColumn {
    fn header(self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::Requests => "Req",
            Self::Tools => "Tools",
            Self::Input => "In",
            Self::Cached => "Cached",
            Self::Output => "Out",
            Self::Thinking => "Think",
            Self::AvgTtfb => "TTFB",
            Self::AvgTime => "Wait",
            Self::TokensPerSecond => "Tok/s",
            Self::Cost => "Cost",
        }
    }

    fn constraint(self) -> Constraint {
        match self {
            Self::Model => Constraint::Min(MODEL_COLUMN_WIDTH),
            Self::Requests => Constraint::Length(REQUESTS_COLUMN_WIDTH),
            Self::Tools => Constraint::Length(TOOLS_COLUMN_WIDTH),
            Self::Input | Self::Cached | Self::Output | Self::Thinking => {
                Constraint::Length(TOKEN_COLUMN_WIDTH)
            }
            Self::AvgTtfb | Self::AvgTime => Constraint::Length(DURATION_COLUMN_WIDTH),
            Self::TokensPerSecond => Constraint::Length(THROUGHPUT_COLUMN_WIDTH),
            Self::Cost => Constraint::Length(COST_COLUMN_WIDTH),
        }
    }

    fn estimated_width(self) -> u16 {
        match self.constraint() {
            Constraint::Min(width) | Constraint::Length(width) => width,
            _ => 0,
        }
    }

    fn cell(self, row: &StatsTableRow) -> String {
        match self {
            Self::Model => row.model_name.clone(),
            Self::Requests => row.totals.request_count.to_string(),
            Self::Tools => row.totals.tool_call_count.to_string(),
            Self::Input => format_compact_tokens(row.totals.input_tokens),
            Self::Cached => format_compact_tokens(row.totals.cached_input_tokens),
            Self::Output => format_compact_tokens(row.totals.output_tokens),
            Self::Thinking => format_thinking_tokens(row.totals),
            Self::AvgTtfb => format_duration_millis(row.totals.average_ttfb_millis()),
            Self::AvgTime => format_duration_millis(row.totals.average_total_request_millis()),
            Self::TokensPerSecond => format_tokens_per_second(row.totals.tokens_per_second()),
            Self::Cost => format_cost(row.totals),
        }
    }
}

pub(super) fn render_stats_screen(frame: &mut Frame, app: &mut App, area: Rect, accent: Color) {
    let screen = {
        let Some(screen) = app.state_mut().ui.stats_screen.as_mut() else {
            return;
        };
        sync_active_stats_viewport(screen, area);
        screen.clone()
    };

    let block = Block::default()
        .title(" Stats ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    frame.render_widget(stats_tab_line(screen.active_tab, accent), layout[0]);

    match screen.active_tab {
        StatsScreenTab::Overview => render_overview_table(frame, &screen, layout[1], accent),
        StatsScreenTab::SessionModels | StatsScreenTab::HistoricalModels => {
            render_model_table(frame, &screen, layout[1], accent)
        }
    }
}

pub(super) fn render_stats_footer(frame: &mut Frame, area: Rect, accent: Color) {
    let spans = vec![
        Span::styled(
            "Esc",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" close  "),
        Span::styled(
            "←/→",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" tabs  "),
        Span::styled(
            "↑/↓",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" rows  "),
        Span::styled(
            "PgUp/PgDn",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" page  "),
        Span::styled(
            "Home/End",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" jump  "),
        Span::styled(
            "~",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" estimated think  "),
        Span::styled(
            "*",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" partial coverage"),
    ];
    let footer = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(accent)),
    );
    frame.render_widget(footer, area);
}

fn sync_active_stats_viewport(screen: &mut StatsScreenState, area: Rect) {
    let inner_height = area.height.saturating_sub(2);
    let visible_rows = inner_height.saturating_sub(1).max(1) as usize;
    let total_rows = screen.active_rows().map(|rows| rows.len()).unwrap_or(0);
    if let Some(table) = screen.active_table_mut() {
        table.sync_viewport(total_rows, visible_rows);
    }
}

fn stats_tab_line(active_tab: StatsScreenTab, accent: Color) -> Paragraph<'static> {
    Paragraph::new(tab_line(
        &[
            ("Overview", active_tab == StatsScreenTab::Overview),
            (
                "Session Models",
                active_tab == StatsScreenTab::SessionModels,
            ),
            (
                "Historical Models",
                active_tab == StatsScreenTab::HistoricalModels,
            ),
        ],
        accent,
    ))
}

fn render_overview_table(frame: &mut Frame, screen: &StatsScreenState, area: Rect, accent: Color) {
    let historical_header = format!("Historical ({})", screen.historical_session_count);
    let header = Row::new(vec![
        Cell::from("Metric"),
        Cell::from("Current Session"),
        Cell::from(historical_header),
    ])
    .style(Style::default().fg(accent).add_modifier(Modifier::BOLD));

    let rows = vec![
        overview_row(
            "Requests",
            screen.current.request_count.to_string(),
            screen.historical.request_count.to_string(),
        ),
        overview_row(
            "In flight",
            screen.current.open_request_count().to_string(),
            screen.historical.open_request_count().to_string(),
        ),
        overview_row(
            "Completed",
            screen.current.completed_request_count.to_string(),
            screen.historical.completed_request_count.to_string(),
        ),
        overview_row(
            "Failed",
            screen.current.failed_request_count.to_string(),
            screen.historical.failed_request_count.to_string(),
        ),
        overview_row(
            "Interrupted",
            screen.current.interrupted_request_count.to_string(),
            screen.historical.interrupted_request_count.to_string(),
        ),
        overview_row(
            "No usage",
            screen.current.requests_without_usage().to_string(),
            screen.historical.requests_without_usage().to_string(),
        ),
        overview_row(
            "Tool calls",
            screen.current.tool_call_count.to_string(),
            screen.historical.tool_call_count.to_string(),
        ),
        overview_row(
            "Input tokens",
            format_compact_tokens(screen.current.input_tokens),
            format_compact_tokens(screen.historical.input_tokens),
        ),
        overview_row(
            "Cached input",
            format_compact_tokens(screen.current.cached_input_tokens),
            format_compact_tokens(screen.historical.cached_input_tokens),
        ),
        overview_row(
            "Output tokens",
            format_compact_tokens(screen.current.output_tokens),
            format_compact_tokens(screen.historical.output_tokens),
        ),
        overview_row(
            "Thinking tokens",
            format_thinking_tokens(screen.current),
            format_thinking_tokens(screen.historical),
        ),
        overview_row(
            "Avg TTFB",
            format_duration_millis(screen.current.average_ttfb_millis()),
            format_duration_millis(screen.historical.average_ttfb_millis()),
        ),
        overview_row(
            "TTFB samples",
            screen.current.ttfb_recorded_request_count.to_string(),
            screen.historical.ttfb_recorded_request_count.to_string(),
        ),
        overview_row(
            "Avg AI wait",
            format_duration_millis(screen.current.average_total_request_millis()),
            format_duration_millis(screen.historical.average_total_request_millis()),
        ),
        overview_row(
            "Tokens/sec",
            format_tokens_per_second(screen.current.tokens_per_second()),
            format_tokens_per_second(screen.historical.tokens_per_second()),
        ),
        overview_row(
            "Estimated cost",
            format_cost(screen.current),
            format_cost(screen.historical),
        ),
    ];

    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Percentage(41),
            Constraint::Percentage(41),
        ],
    )
    .header(header)
    .column_spacing(1);
    frame.render_widget(table, area);
}

fn overview_row(metric: &str, current: String, historical: String) -> Row<'static> {
    Row::new(vec![
        Cell::from(metric.to_string()),
        Cell::from(current),
        Cell::from(historical),
    ])
}

fn render_model_table(frame: &mut Frame, screen: &StatsScreenState, area: Rect, accent: Color) {
    let Some(rows) = screen.active_rows() else {
        return;
    };
    let Some(table_state) = screen.active_table() else {
        return;
    };

    if rows.is_empty() {
        let empty = Paragraph::new("No model stats recorded yet.");
        frame.render_widget(empty, area);
        return;
    }

    let show_scrollbar = area.width > 2 && rows.len() > table_state.viewport_rows();
    let layout = if show_scrollbar {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1)])
            .split(area)
    };

    let table_area = layout[0];
    let columns = visible_model_columns(table_area.width);
    let constraints = columns
        .iter()
        .map(|column| column.constraint())
        .collect::<Vec<_>>();
    let header = Row::new(
        columns
            .iter()
            .map(|column| Cell::from(column.header()))
            .collect::<Vec<_>>(),
    )
    .style(Style::default().fg(accent).add_modifier(Modifier::BOLD));

    let visible_rows = table_state
        .visible_range()
        .map(|index| {
            let is_selected = index == table_state.selected_index;
            let row = &rows[index];
            let style = if is_selected {
                Style::default().bg(accent).fg(Color::Black)
            } else {
                Style::default()
            };
            Row::new(
                columns
                    .iter()
                    .map(|column| Cell::from(column.cell(row)))
                    .collect::<Vec<_>>(),
            )
            .style(style)
        })
        .collect::<Vec<_>>();

    let table = Table::new(visible_rows, constraints)
        .header(header)
        .column_spacing(1);
    frame.render_widget(table, table_area);

    if show_scrollbar {
        render_vertical_scrollbar(
            frame,
            layout[1],
            rows.len(),
            table_state.viewport_rows(),
            table_state.scroll_top,
            accent,
        );
    }
}

fn visible_model_columns(width: u16) -> Vec<ModelTableColumn> {
    let mut columns = vec![
        ModelTableColumn::Model,
        ModelTableColumn::Requests,
        ModelTableColumn::Tools,
        ModelTableColumn::Input,
        ModelTableColumn::Cached,
        ModelTableColumn::Output,
        ModelTableColumn::Thinking,
        ModelTableColumn::AvgTtfb,
        ModelTableColumn::AvgTime,
        ModelTableColumn::TokensPerSecond,
        ModelTableColumn::Cost,
    ];
    let drop_order = [
        ModelTableColumn::Cost,
        ModelTableColumn::TokensPerSecond,
        ModelTableColumn::AvgTime,
        ModelTableColumn::AvgTtfb,
        ModelTableColumn::Cached,
    ];

    while estimated_table_width(&columns) > width {
        let Some(to_remove) = drop_order
            .iter()
            .copied()
            .find(|candidate| columns.contains(candidate))
        else {
            break;
        };
        columns.retain(|column| *column != to_remove);
    }

    columns
}

fn estimated_table_width(columns: &[ModelTableColumn]) -> u16 {
    if columns.is_empty() {
        return 0;
    }
    let base = columns
        .iter()
        .map(|column| column.estimated_width())
        .sum::<u16>();
    base + columns.len().saturating_sub(1) as u16
}

fn format_thinking_tokens(totals: StatsTotals) -> String {
    let Some(tokens) = totals.thinking_tokens_value() else {
        return "n/a".to_string();
    };
    let mut rendered = String::new();
    if totals.thinking_tokens_estimated() {
        rendered.push('~');
    }
    rendered.push_str(&format_compact_tokens(tokens));
    if totals.thinking_tokens_partial() {
        rendered.push('*');
    }
    rendered
}

fn format_duration_millis(duration: Option<f64>) -> String {
    let Some(duration) = duration else {
        return "n/a".to_string();
    };
    if duration < 1_000.0 {
        format!("{duration:.0}ms")
    } else {
        format!("{:.1}s", duration / 1_000.0)
    }
}

fn format_tokens_per_second(tokens_per_second: Option<f64>) -> String {
    let Some(tokens_per_second) = tokens_per_second else {
        return "n/a".to_string();
    };
    if tokens_per_second >= 1_000.0 {
        format!("{:.1}K", tokens_per_second / 1_000.0)
    } else {
        format!("{tokens_per_second:.0}")
    }
}

fn format_cost(totals: StatsTotals) -> String {
    format!("${}", format_price(totals.estimated_cost_usd()))
}

#[cfg(test)]
mod tests;
