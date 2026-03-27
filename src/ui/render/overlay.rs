use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::app::{
    App, ModelPickerEntry, ModelPickerTab, SelectionPicker, display_entries_for_tab, query,
};

use super::helpers::{
    command_palette_line, model_picker_detail, model_picker_tab_line, selection_picker_line,
};

pub(super) fn render_overlay(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    if let Some(picker) = query::selection_picker(app.state()) {
        render_selection_picker(frame, app, picker, area, accent);
    } else {
        render_command_palette(frame, app, area, accent);
    }
}

fn render_command_palette(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let visible_rows = area.height.saturating_sub(2) as usize;
    let commands = query::filtered_commands(app.state());
    let selected = query::selected_command(app.state());
    let lines = if commands.is_empty() {
        vec![Line::from(Span::styled(
            "No matching commands",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        commands
            .into_iter()
            .take(visible_rows)
            .map(|command| command_palette_line(command, selected, accent))
            .collect()
    };

    let palette = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Commands ")
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .border_style(Style::default().fg(accent)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(palette, area);
}

fn render_selection_picker(
    frame: &mut Frame,
    app: &App,
    picker: &SelectionPicker,
    area: Rect,
    accent: Color,
) {
    let visible_rows = area.height.saturating_sub(2) as usize;
    let (title, lines) = match picker {
        SelectionPicker::Model {
            active_tab,
            normal_selected_model,
            planning_selected_model,
            safety_selected_model,
        } => {
            let mut lines = vec![model_picker_tab_line(*active_tab, accent)];
            let row_budget = visible_rows.saturating_sub(1);
            let selected_model = match active_tab {
                ModelPickerTab::NormalAgent => normal_selected_model.as_str(),
                ModelPickerTab::PlanningAgents => planning_selected_model.as_str(),
                ModelPickerTab::SafetyModel => safety_selected_model.as_str(),
            };
            let entries = display_entries_for_tab(*active_tab, query::model_name(app.state()));
            for entry in visible_model_entries(&entries, row_budget, selected_model) {
                match entry {
                    ModelPickerEntry::ProviderHeading(provider) => {
                        lines.push(Line::from(vec![Span::styled(
                            provider.display_name(),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::BOLD),
                        )]))
                    }
                    ModelPickerEntry::Model(model) => {
                        let detail = match active_tab {
                            ModelPickerTab::NormalAgent => model_picker_detail(model),
                            ModelPickerTab::PlanningAgents => query::planning_agents(app.state())
                                .iter()
                                .find(|agent| agent.model_name == model.name)
                                .map(|agent| {
                                    format!("selected  reasoning: {}", agent.reasoning.as_str())
                                })
                                .unwrap_or_else(|| {
                                    "not selected  Space toggles  Enter sets reasoning".into()
                                }),
                            ModelPickerTab::SafetyModel => {
                                if query::safety_model_name(app.state()) == model.name {
                                    format!(
                                        "selected  reasoning: {}",
                                        query::safety_reasoning(app.state()).as_str()
                                    )
                                } else {
                                    "Enter sets reasoning".into()
                                }
                            }
                        };
                        lines.push(selection_picker_line(
                            model.name == selected_model,
                            model.name,
                            detail,
                            accent,
                        ));
                    }
                }
            }
            (" Models ", lines)
        }
        SelectionPicker::Reasoning {
            model_name,
            options,
            selected_index,
            target,
        } => {
            let lines: Vec<Line<'static>> = options
                .iter()
                .take(visible_rows)
                .enumerate()
                .map(|(index, level)| {
                    selection_picker_line(
                        index == *selected_index,
                        level.as_str(),
                        match target {
                            crate::app::ReasoningPickerTarget::NormalAgent => {
                                format!("for {}", model_name)
                            }
                            crate::app::ReasoningPickerTarget::PlanningAgent => {
                                format!("for planning with {}", model_name)
                            }
                            crate::app::ReasoningPickerTarget::SafetyModel => {
                                format!("for safety classification with {}", model_name)
                            }
                        },
                        accent,
                    )
                })
                .collect();
            let title = match target {
                crate::app::ReasoningPickerTarget::NormalAgent => " Reasoning ",
                crate::app::ReasoningPickerTarget::PlanningAgent => " Planning Reasoning ",
                crate::app::ReasoningPickerTarget::SafetyModel => " Safety Reasoning ",
            };
            (title, lines)
        }
    };

    let picker = Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .border_style(Style::default().fg(accent)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(picker, area);
}

fn visible_model_entries<'a>(
    entries: &'a [ModelPickerEntry],
    row_budget: usize,
    selected_model: &str,
) -> &'a [ModelPickerEntry] {
    if row_budget == 0 {
        return &entries[..0];
    }

    if entries.len() <= row_budget {
        return entries;
    }

    let selected_index = entries
        .iter()
        .position(
            |entry| matches!(entry, ModelPickerEntry::Model(model) if model.name == selected_model),
        )
        .or_else(|| entries.iter().position(ModelPickerEntry::is_model))
        .unwrap_or(0);
    let provider_heading_index = entries[..=selected_index]
        .iter()
        .rposition(|entry| matches!(entry, ModelPickerEntry::ProviderHeading(_)))
        .unwrap_or(0);

    let mut start = if selected_index.saturating_sub(provider_heading_index) < row_budget {
        provider_heading_index
    } else {
        selected_index + 1 - row_budget
    };
    let mut end = (start + row_budget).min(entries.len());
    if end - start < row_budget && end == entries.len() {
        start = end.saturating_sub(row_budget);
    }
    end = (start + row_budget).min(entries.len());
    &entries[start..end]
}

#[cfg(test)]
mod tests;
