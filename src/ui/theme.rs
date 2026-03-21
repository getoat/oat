use ratatui::style::Color;

use crate::app::AccessMode;

pub(super) fn accent_color(mode: AccessMode) -> Color {
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
        assert_eq!(accent_color(AccessMode::ReadOnly), Color::Magenta);
        assert_eq!(accent_color(AccessMode::ReadWrite), Color::Cyan);
    }
}
