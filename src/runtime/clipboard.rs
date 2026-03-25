use base64::{Engine as _, engine::general_purpose::STANDARD};

pub(crate) fn osc52_copy_sequence(text: &str) -> String {
    format!("\u{1b}]52;c;{}\u{7}", STANDARD.encode(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_sequence_base64_encodes_selection_text() {
        assert_eq!(
            osc52_copy_sequence("copy me"),
            "\u{1b}]52;c;Y29weSBtZQ==\u{7}"
        );
    }
}
