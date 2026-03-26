use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::app::{App, ModelPickerTab, SelectionPicker, query};

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
            normal_selected_index,
            planning_selected_index,
            safety_selected_index,
        } => {
            let mut lines = vec![model_picker_tab_line(*active_tab, accent)];
            let row_budget = visible_rows.saturating_sub(1);
            match active_tab {
                ModelPickerTab::NormalAgent => {
                    lines.extend(
                        crate::model_registry::models()
                            .iter()
                            .take(row_budget)
                            .enumerate()
                            .map(|(index, model)| {
                                selection_picker_line(
                                    index == *normal_selected_index,
                                    model.name,
                                    model_picker_detail(model),
                                    accent,
                                )
                            }),
                    );
                }
                ModelPickerTab::PlanningAgents => {
                    lines.extend(
                        crate::model_registry::models()
                            .iter()
                            .filter(|model| model.name != query::model_name(app.state()))
                            .take(row_budget)
                            .enumerate()
                            .map(|(index, model)| {
                                let planning_agent = query::planning_agents(app.state())
                                    .iter()
                                    .find(|agent| agent.model_name == model.name);
                                let detail = planning_agent
                                    .map(|agent| {
                                        format!("selected  reasoning: {}", agent.reasoning.as_str())
                                    })
                                    .unwrap_or_else(|| {
                                        "not selected  Space toggles  Enter sets reasoning".into()
                                    });
                                selection_picker_line(
                                    index == *planning_selected_index,
                                    model.name,
                                    detail,
                                    accent,
                                )
                            }),
                    );
                }
                ModelPickerTab::SafetyModel => {
                    lines.extend(
                        crate::model_registry::models()
                            .iter()
                            .take(row_budget)
                            .enumerate()
                            .map(|(index, model)| {
                                let detail = if query::safety_model_name(app.state()) == model.name
                                {
                                    format!(
                                        "selected  reasoning: {}",
                                        query::safety_reasoning(app.state()).as_str()
                                    )
                                } else {
                                    "Enter sets reasoning".into()
                                };
                                selection_picker_line(
                                    index == *safety_selected_index,
                                    model.name,
                                    detail,
                                    accent,
                                )
                            }),
                    );
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

#[cfg(test)]
mod tests;
