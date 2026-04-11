use crate::stats::{StatsReport, StatsTotals};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatsScreenTab {
    Overview,
    SessionModels,
    HistoricalModels,
}

impl StatsScreenTab {
    pub fn toggle(&mut self, direction: isize) {
        let tabs = [Self::Overview, Self::SessionModels, Self::HistoricalModels];
        let current = tabs.iter().position(|tab| *tab == *self).unwrap_or(0);
        let next = (current as isize + direction).rem_euclid(tabs.len() as isize) as usize;
        *self = tabs[next];
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatsTableRow {
    pub model_name: String,
    pub totals: StatsTotals,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatsTableViewState {
    pub selected_index: usize,
    pub scroll_top: usize,
    viewport_rows: usize,
    total_rows: usize,
}

impl StatsTableViewState {
    pub fn sync_viewport(&mut self, total_rows: usize, viewport_rows: usize) -> usize {
        self.total_rows = total_rows;
        self.viewport_rows = viewport_rows.max(1);
        self.clamp();
        self.keep_selected_visible();
        self.scroll_top
    }

    pub fn move_selection(&mut self, direction: isize) {
        if self.total_rows == 0 {
            self.selected_index = 0;
            self.scroll_top = 0;
            return;
        }

        self.selected_index = (self.selected_index as isize + direction)
            .rem_euclid(self.total_rows as isize) as usize;
        self.keep_selected_visible();
    }

    pub fn page_up(&mut self) {
        let step = self.viewport_rows.max(1);
        self.selected_index = self.selected_index.saturating_sub(step);
        self.keep_selected_visible();
    }

    pub fn page_down(&mut self) {
        let step = self.viewport_rows.max(1);
        self.selected_index = self
            .selected_index
            .saturating_add(step)
            .min(self.total_rows.saturating_sub(1));
        self.keep_selected_visible();
    }

    pub fn scroll_to_top(&mut self) {
        self.selected_index = 0;
        self.scroll_top = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        if self.total_rows == 0 {
            self.selected_index = 0;
            self.scroll_top = 0;
            return;
        }
        self.selected_index = self.total_rows - 1;
        self.keep_selected_visible();
    }

    pub fn visible_range(&self) -> std::ops::Range<usize> {
        let end = self
            .scroll_top
            .saturating_add(self.viewport_rows)
            .min(self.total_rows);
        self.scroll_top.min(end)..end
    }

    pub fn viewport_rows(&self) -> usize {
        self.viewport_rows.max(1)
    }

    fn max_start(&self) -> usize {
        self.total_rows.saturating_sub(self.viewport_rows.max(1))
    }

    fn clamp(&mut self) {
        if self.total_rows == 0 {
            self.selected_index = 0;
            self.scroll_top = 0;
            return;
        }

        self.selected_index = self.selected_index.min(self.total_rows - 1);
        self.scroll_top = self.scroll_top.min(self.max_start());
    }

    fn keep_selected_visible(&mut self) {
        self.clamp();
        if self.total_rows == 0 {
            return;
        }
        if self.selected_index < self.scroll_top {
            self.scroll_top = self.selected_index;
            return;
        }
        let bottom = self.scroll_top.saturating_add(self.viewport_rows.max(1));
        if self.selected_index >= bottom {
            self.scroll_top = self
                .selected_index
                .saturating_add(1)
                .saturating_sub(self.viewport_rows.max(1));
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatsScreenState {
    pub active_tab: StatsScreenTab,
    pub current: StatsTotals,
    pub historical: StatsTotals,
    pub historical_session_count: usize,
    pub session_models: Vec<StatsTableRow>,
    pub historical_models: Vec<StatsTableRow>,
    pub session_table: StatsTableViewState,
    pub historical_table: StatsTableViewState,
}

impl StatsScreenState {
    pub fn new(report: StatsReport) -> Self {
        Self {
            active_tab: StatsScreenTab::Overview,
            current: report.current,
            historical: report.historical,
            historical_session_count: report.historical_session_count,
            session_models: sorted_model_rows(report.current_models),
            historical_models: sorted_model_rows(report.historical_models),
            session_table: StatsTableViewState::default(),
            historical_table: StatsTableViewState::default(),
        }
    }

    pub fn active_table_mut(&mut self) -> Option<&mut StatsTableViewState> {
        match self.active_tab {
            StatsScreenTab::Overview => None,
            StatsScreenTab::SessionModels => Some(&mut self.session_table),
            StatsScreenTab::HistoricalModels => Some(&mut self.historical_table),
        }
    }

    pub fn active_table(&self) -> Option<&StatsTableViewState> {
        match self.active_tab {
            StatsScreenTab::Overview => None,
            StatsScreenTab::SessionModels => Some(&self.session_table),
            StatsScreenTab::HistoricalModels => Some(&self.historical_table),
        }
    }

    pub fn active_rows(&self) -> Option<&[StatsTableRow]> {
        match self.active_tab {
            StatsScreenTab::Overview => None,
            StatsScreenTab::SessionModels => Some(&self.session_models),
            StatsScreenTab::HistoricalModels => Some(&self.historical_models),
        }
    }
}

fn sorted_model_rows(
    models: std::collections::BTreeMap<String, StatsTotals>,
) -> Vec<StatsTableRow> {
    let mut rows = models
        .into_iter()
        .map(|(model_name, totals)| StatsTableRow { model_name, totals })
        .collect::<Vec<_>>();
    rows.sort_by(|lhs, rhs| {
        rhs.totals
            .request_count
            .cmp(&lhs.totals.request_count)
            .then_with(|| rhs.totals.tool_call_count.cmp(&lhs.totals.tool_call_count))
            .then_with(|| lhs.model_name.cmp(&rhs.model_name))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn table_view_state_keeps_selection_visible() {
        let mut state = StatsTableViewState::default();
        state.sync_viewport(10, 3);

        for _ in 0..5 {
            state.move_selection(1);
        }

        assert_eq!(state.selected_index, 5);
        assert_eq!(state.scroll_top, 3);
    }

    #[test]
    fn screen_state_sorts_rows_by_request_count_then_model_name() {
        let mut current_models = BTreeMap::new();
        current_models.insert(
            "z-model".into(),
            StatsTotals {
                request_count: 1,
                ..StatsTotals::default()
            },
        );
        current_models.insert(
            "a-model".into(),
            StatsTotals {
                request_count: 3,
                ..StatsTotals::default()
            },
        );
        current_models.insert(
            "b-model".into(),
            StatsTotals {
                request_count: 3,
                ..StatsTotals::default()
            },
        );
        let report = StatsReport {
            current: StatsTotals::default(),
            historical: StatsTotals::default(),
            current_models,
            historical_models: BTreeMap::new(),
            historical_session_count: 0,
        };

        let screen = StatsScreenState::new(report);

        assert_eq!(screen.session_models[0].model_name, "a-model");
        assert_eq!(screen.session_models[1].model_name, "b-model");
        assert_eq!(screen.session_models[2].model_name, "z-model");
    }
}
