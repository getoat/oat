use ratatui::style::Color;

use crate::app::AccessMode;

pub(super) fn accent_color(mode: AccessMode, plan_active: bool) -> Color {
    if plan_active {
        return Color::Yellow;
    }

    match mode {
        AccessMode::ReadOnly => Color::Magenta,
        AccessMode::ReadWrite => Color::Cyan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_color_tracks_access_mode() {
        assert_eq!(accent_color(AccessMode::ReadOnly, false), Color::Magenta);
        assert_eq!(accent_color(AccessMode::ReadWrite, false), Color::Cyan);
    }

    #[test]
    fn accent_color_uses_dedicated_plan_state_color() {
        assert_eq!(accent_color(AccessMode::ReadOnly, true), Color::Yellow);
        assert_eq!(accent_color(AccessMode::ReadWrite, true), Color::Yellow);
    }
}
