#[derive(Debug, Default)]
pub struct CommandRecallState {
    pub entries: Vec<String>,
    pub browsing_index: Option<usize>,
    pub draft: Option<String>,
    pub limit: usize,
    pub dirty: bool,
}

impl CommandRecallState {
    pub fn restore(&mut self, mut entries: Vec<String>, limit: usize) {
        self.limit = limit;
        self.browsing_index = None;
        self.draft = None;
        self.dirty = false;
        self.entries.clear();
        self.entries.append(&mut entries);
        self.trim_to_limit();
    }

    pub fn record(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }

        if self.entries.last().is_some_and(|entry| entry == text) {
            self.browsing_index = None;
            self.draft = None;
            return;
        }

        self.entries.push(text.to_string());
        self.trim_to_limit();
        self.browsing_index = None;
        self.draft = None;
        self.dirty = true;
    }

    pub fn previous(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }

        match self.browsing_index {
            Some(index) if index > 0 => self.browsing_index = Some(index - 1),
            Some(_) => {}
            None => {
                self.draft = Some(current.to_string());
                self.browsing_index = Some(self.entries.len() - 1);
            }
        }

        self.browsing_index.map(|index| self.entries[index].clone())
    }

    pub fn next(&mut self) -> Option<String> {
        match self.browsing_index {
            None => None,
            Some(index) if index + 1 < self.entries.len() => {
                self.browsing_index = Some(index + 1);
                self.browsing_index.map(|index| self.entries[index].clone())
            }
            Some(_) => {
                self.browsing_index = None;
                Some(self.draft.take().unwrap_or_default())
            }
        }
    }

    pub fn reset_navigation(&mut self) {
        self.browsing_index = None;
        self.draft = None;
    }

    pub fn take_dirty_entries(&mut self) -> Option<Vec<String>> {
        if !self.dirty {
            return None;
        }

        self.dirty = false;
        Some(self.entries.clone())
    }

    fn trim_to_limit(&mut self) {
        self.entries.retain(|entry| !entry.trim().is_empty());
        self.entries.dedup();
        if self.entries.len() > self.limit {
            self.entries.drain(..self.entries.len() - self.limit);
        }
    }
}
