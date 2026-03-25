use super::App;

impl App {
    pub fn push_agent_message(&mut self, text: impl Into<String>) {
        self.reducer_context().push_agent_message(text);
    }

    pub fn push_error_message(&mut self, text: impl Into<String>) {
        self.reducer_context().push_error_message(text);
    }
}
