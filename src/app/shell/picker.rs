use super::App;

impl App {
    #[cfg(test)]
    pub(crate) fn open_model_picker(&mut self) {
        self.reducer_context().open_model_picker();
    }

    pub(crate) fn open_reasoning_picker(&mut self) {
        self.reducer_context().open_reasoning_picker();
    }

    #[cfg(test)]
    pub(crate) fn sync_command_selection(&mut self) {
        self.reducer_context().sync_command_selection();
    }
}
