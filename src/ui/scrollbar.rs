use ratatui::{Frame, layout::Rect, style::Color, text::Line, widgets::Paragraph};

pub(crate) fn scrollbar_thumb_bounds(
    track_height: usize,
    total_lines: usize,
    viewport_rows: usize,
    scroll_position: usize,
) -> (usize, usize) {
    if track_height == 0 {
        return (0, 0);
    }

    let total_lines = total_lines.max(1);
    let viewport_rows = viewport_rows.max(1).min(total_lines);
    let thumb_len =
        ((viewport_rows as f64 / total_lines as f64) * track_height as f64).round() as usize;
    let thumb_len = thumb_len.clamp(1, track_height);

    let max_scroll = total_lines.saturating_sub(viewport_rows);
    let max_thumb_start = track_height.saturating_sub(thumb_len);
    let thumb_start = if max_scroll == 0 || max_thumb_start == 0 {
        0
    } else {
        ((scroll_position.min(max_scroll) as f64 / max_scroll as f64) * max_thumb_start as f64)
            .round() as usize
    };
    (thumb_start, thumb_len)
}

pub(crate) fn render_vertical_scrollbar(
    frame: &mut Frame,
    area: Rect,
    total_lines: usize,
    viewport_rows: usize,
    scroll_position: usize,
    accent: Color,
) {
    let track_height = area.height as usize;
    let (thumb_start, thumb_len) =
        scrollbar_thumb_bounds(track_height, total_lines, viewport_rows, scroll_position);
    let lines = (0..track_height)
        .map(|row| {
            if row >= thumb_start && row < thumb_start + thumb_len {
                Line::styled(" ", ratatui::style::Style::default().bg(accent))
            } else {
                Line::default()
            }
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
}
