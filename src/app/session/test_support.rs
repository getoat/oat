use crate::{app::App, config::ReasoningEffort};

pub(crate) fn new_app(show_thinking: bool) -> App {
    App::new(show_thinking, false, "gpt-5-mini", ReasoningEffort::Medium)
}

pub(crate) fn registry_app(show_thinking: bool) -> App {
    App::new(
        show_thinking,
        false,
        "gpt-5.4-mini",
        ReasoningEffort::Medium,
    )
}
