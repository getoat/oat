use crate::{app::session::SlashCommand, composer::ComposerLayout};

use super::{
    AskUserUiState, CommandRecallState, ComposerUiState, HistoryRenderCache, HistoryViewState,
    SelectionPicker, ShellApprovalUiState, StatsScreenState,
};

#[derive(Debug)]
pub struct UiState {
    pub composer: ComposerUiState,
    pub selected_command: SlashCommand,
    pub picker: Option<SelectionPicker>,
    pub stats_screen: Option<StatsScreenState>,
    pub plan_review_selected_index: usize,
    pub pending_shell_approval: Option<ShellApprovalUiState>,
    pub pending_ask_user: Option<AskUserUiState>,
    pub history_render_cache: Option<HistoryRenderCache>,
    pub history: HistoryViewState,
    pub command_history: CommandRecallState,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            composer: ComposerUiState::default(),
            selected_command: SlashCommand::NewSession,
            picker: None,
            stats_screen: None,
            plan_review_selected_index: 0,
            pending_shell_approval: None,
            pending_ask_user: None,
            history_render_cache: None,
            history: HistoryViewState::default(),
            command_history: CommandRecallState {
                limit: 20,
                ..CommandRecallState::default()
            },
        }
    }
}

impl UiState {
    pub fn composer_layout(&mut self) -> &ComposerLayout {
        if self.composer.layout_cache.is_none() {
            self.composer.layout_cache = Some(ComposerLayout::new(
                self.composer.composer.lines(),
                self.composer.wrap_width,
            ));
        }

        self.composer
            .layout_cache
            .as_ref()
            .expect("composer layout cache should be populated")
    }

    pub fn invalidate_composer_layout(&mut self) {
        self.composer.layout_cache = None;
    }
}
