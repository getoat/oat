use ratatui::{
    layout::Alignment,
    style::Style,
    text::{Line, Span},
};

pub(super) fn wrap_styled_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut wrapped = Vec::new();

    for line in lines {
        wrap_styled_line(line, width, &mut wrapped);
    }

    if wrapped.is_empty() {
        wrapped.push(Line::default());
    }

    wrapped
}

fn wrap_styled_line(line: Line<'static>, width: usize, wrapped: &mut Vec<Line<'static>>) {
    if line.spans.is_empty() {
        wrapped.push(Line {
            style: line.style,
            alignment: line.alignment,
            spans: Vec::new(),
        });
        return;
    }

    let mut current = Vec::new();
    let mut current_width = 0;

    for span in line.spans {
        for segment in split_preserving_whitespace(span.content.as_ref()) {
            push_styled_segment(
                segment,
                span.style,
                width,
                line.style,
                line.alignment,
                &mut current,
                &mut current_width,
                wrapped,
            );
        }
    }

    if !current.is_empty() {
        wrapped.push(Line {
            style: line.style,
            alignment: line.alignment,
            spans: current,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn push_styled_segment(
    segment: String,
    style: Style,
    width: usize,
    line_style: Style,
    alignment: Option<Alignment>,
    current: &mut Vec<Span<'static>>,
    current_width: &mut usize,
    wrapped: &mut Vec<Line<'static>>,
) {
    if segment.is_empty() {
        return;
    }

    let segment_width = segment.chars().count();
    if *current_width + segment_width <= width {
        current.push(Span::styled(segment, style));
        *current_width += segment_width;
        return;
    }

    if !current.is_empty() {
        wrapped.push(Line {
            style: line_style,
            alignment,
            spans: std::mem::take(current),
        });
        *current_width = 0;
    }

    if segment_width <= width {
        current.push(Span::styled(segment, style));
        *current_width = segment_width;
        return;
    }

    let mut chunk = String::new();
    for ch in segment.chars() {
        chunk.push(ch);
        if chunk.chars().count() == width {
            wrapped.push(Line {
                style: line_style,
                alignment,
                spans: vec![Span::styled(std::mem::take(&mut chunk), style)],
            });
        }
    }

    if !chunk.is_empty() {
        *current_width = chunk.chars().count();
        current.push(Span::styled(chunk, style));
    }
}

fn split_preserving_whitespace(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_is_whitespace = None;

    for ch in text.chars() {
        let is_whitespace = ch.is_whitespace();
        match current_is_whitespace {
            Some(value) if value == is_whitespace => current.push(ch),
            Some(_) => {
                segments.push(std::mem::take(&mut current));
                current.push(ch);
                current_is_whitespace = Some(is_whitespace);
            }
            None => {
                current.push(ch);
                current_is_whitespace = Some(is_whitespace);
            }
        }
    }

    if !current.is_empty() {
        segments.push(current);
    }

    segments
}

pub(super) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let paragraphs: Vec<&str> = text.split('\n').collect();

    for paragraph in paragraphs {
        if paragraph.is_empty() {
            lines.push(String::new());
        } else {
            wrap_paragraph(paragraph, width, &mut lines);
        }
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

fn wrap_paragraph(paragraph: &str, width: usize, lines: &mut Vec<String>) {
    let mut current = String::new();

    for word in paragraph.split_whitespace() {
        let word_len = word.chars().count();

        if current.is_empty() {
            if word_len <= width {
                current.push_str(word);
            } else {
                push_split_word(word, width, lines, &mut current);
            }
            continue;
        }

        let candidate_len = current.chars().count() + 1 + word_len;
        if candidate_len <= width {
            current.push(' ');
            current.push_str(word);
            continue;
        }

        lines.push(std::mem::take(&mut current));
        if word_len <= width {
            current.push_str(word);
        } else {
            push_split_word(word, width, lines, &mut current);
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
}

fn push_split_word(word: &str, width: usize, lines: &mut Vec<String>, current: &mut String) {
    let mut chunk = String::new();

    for ch in word.chars() {
        chunk.push(ch);
        if chunk.chars().count() == width {
            lines.push(std::mem::take(&mut chunk));
        }
    }

    if !chunk.is_empty() {
        current.push_str(&chunk);
    }
}
