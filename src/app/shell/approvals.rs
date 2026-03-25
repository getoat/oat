use super::App;
use crate::app::{PendingShellApproval, ui::ShellApprovalUiState};

impl App {
    pub(crate) fn shell_approval_session(&self) -> Option<&PendingShellApproval> {
        self.session.pending_shell_approvals.front()
    }

    pub(crate) fn shell_approval_ui(&self) -> Option<&ShellApprovalUiState> {
        self.ui.pending_shell_approval.as_ref()
    }
}
