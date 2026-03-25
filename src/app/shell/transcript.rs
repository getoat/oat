use super::*;

impl AppShell {
    pub fn push_agent_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::Agent, text, MessageStyle::Plain);
    }

    pub(crate) fn push_user_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::User, text, MessageStyle::Plain);
    }

    pub fn push_error_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::Agent, text, MessageStyle::Error);
    }

    pub(crate) fn push_agent_error(&mut self, text: impl Into<String>) {
        self.push_error_message(text);
    }

    pub(crate) fn push_agent_commentary(&mut self, text: impl Into<String>) {
        let text = text.into();
        if let Some(pending) = self.session.pending_reply.as_mut() {
            pending.reset_active_stream_segment();
            pending.commentary_messages.push(text.clone());
            pending.has_visible_content = true;
        }
        self.push_message(Speaker::Agent, text, MessageStyle::Commentary);
    }

    pub(crate) fn push_tool_call(&mut self, name: String, parameter: String) {
        if let Some(pending) = self.session.pending_reply.as_mut() {
            pending.reset_active_stream_segment();
            pending.has_visible_content = true;
        }
        self.session
            .entries
            .push(TranscriptEntry::ToolCall(ToolCall {
                preview: mutation_preview(&name, &parameter, &self.session.workspace_root),
                name,
                parameter,
            }));
        self.bump_transcript_revision();
    }

    pub(crate) fn push_tool_result(&mut self, name: String, output: String) {
        if let Some(pending) = self.session.pending_reply.as_mut() {
            pending.reset_active_stream_segment();
            if self.session.show_tool_output {
                pending.has_visible_content = true;
            }
        }
        self.session
            .entries
            .push(TranscriptEntry::ToolResult(ToolResultEntry {
                name,
                output,
            }));
        self.bump_transcript_revision();
    }

    pub(crate) fn upsert_subagent_status(
        &mut self,
        id: String,
        kind: SubagentStatusKind,
        display_label: String,
        state: SubagentDisplayState,
        status_text: String,
    ) {
        if let Some(TranscriptEntry::SubagentStatus(entry)) = self.session.entries.iter_mut().find(
            |entry| matches!(entry, TranscriptEntry::SubagentStatus(status) if status.id == id),
        ) {
            entry.kind = kind;
            entry.display_label = display_label;
            entry.state = state;
            entry.status_text = status_text;
            self.bump_transcript_revision();
            return;
        }

        self.session
            .entries
            .push(TranscriptEntry::SubagentStatus(SubagentStatusEntry {
                id,
                kind,
                display_label,
                state,
                status_text,
                latest_tool_name: None,
            }));
        self.bump_transcript_revision();
    }

    pub(crate) fn set_subagent_latest_tool(&mut self, id: String, latest_tool_name: String) {
        if let Some(TranscriptEntry::SubagentStatus(entry)) = self.session.entries.iter_mut().find(
            |entry| matches!(entry, TranscriptEntry::SubagentStatus(status) if status.id == id),
        ) {
            entry.latest_tool_name = Some(latest_tool_name);
            self.bump_transcript_revision();
            return;
        }

        self.session
            .entries
            .push(TranscriptEntry::SubagentStatus(SubagentStatusEntry {
                display_label: id.clone(),
                id,
                kind: SubagentStatusKind::Subagent,
                state: SubagentDisplayState::Running,
                status_text: "running".into(),
                latest_tool_name: Some(latest_tool_name),
            }));
        self.bump_transcript_revision();
    }

    pub(crate) fn append_pending_stream_message(&mut self, delta: &str, style: MessageStyle) {
        if delta.is_empty() || self.session.pending_reply.is_none() || style == MessageStyle::Error
        {
            return;
        }

        let existing_index = {
            let pending = self
                .session
                .pending_reply
                .as_mut()
                .expect("pending reply checked above");
            let crossed_style_boundary = match style {
                MessageStyle::Plain => pending.reasoning_entry_index.is_some(),
                MessageStyle::Commentary => true,
                MessageStyle::Thinking => pending.text_entry_index.is_some(),
                MessageStyle::Error => false,
            };
            if crossed_style_boundary {
                pending.reset_active_stream_segment();
            }
            match style {
                MessageStyle::Plain => pending.text_entry_index,
                MessageStyle::Commentary => None,
                MessageStyle::Thinking => pending.reasoning_entry_index,
                MessageStyle::Error => None,
            }
        };

        let Some(existing_index) = existing_index else {
            let mut pending_text = delta.to_string();
            {
                let pending = self
                    .session
                    .pending_reply
                    .as_mut()
                    .expect("pending reply checked above");
                match style {
                    MessageStyle::Plain => {
                        pending.plain_text.push_str(delta);
                        pending.staged_plain_text.push_str(delta);
                        if !pending_stream_text_is_visible(style, &pending.staged_plain_text) {
                            return;
                        }
                        pending_text = std::mem::take(&mut pending.staged_plain_text);
                    }
                    MessageStyle::Thinking => {
                        pending.reasoning_text.push_str(delta);
                        pending.staged_reasoning_text.push_str(delta);
                        if !pending_stream_text_is_visible(style, &pending.staged_reasoning_text) {
                            return;
                        }
                        pending_text = std::mem::take(&mut pending.staged_reasoning_text);
                    }
                    MessageStyle::Commentary => {
                        if !pending_stream_text_is_visible(style, delta) {
                            return;
                        }
                    }
                    MessageStyle::Error => return,
                }
            }

            self.push_message(Speaker::Agent, pending_text, style);
            let index = self.session.entries.len() - 1;
            let pending = self
                .session
                .pending_reply
                .as_mut()
                .expect("pending reply checked above");
            pending.has_visible_content = true;
            match style {
                MessageStyle::Plain => pending.text_entry_index = Some(index),
                MessageStyle::Commentary => {}
                MessageStyle::Thinking => pending.reasoning_entry_index = Some(index),
                MessageStyle::Error => {}
            }
            return;
        };

        if let Some(TranscriptEntry::Message(message)) =
            self.session.entries.get_mut(existing_index)
        {
            message.text.push_str(delta);
            if style == MessageStyle::Plain
                && let Some(pending) = self.session.pending_reply.as_mut()
            {
                pending.plain_text.push_str(delta);
            }
            self.bump_transcript_revision();
        }
    }

    fn push_message(&mut self, speaker: Speaker, text: impl Into<String>, style: MessageStyle) {
        self.session
            .entries
            .push(TranscriptEntry::Message(ChatMessage {
                speaker,
                text: text.into(),
                style,
            }));
        self.bump_transcript_revision();
    }

    fn bump_transcript_revision(&mut self) {
        self.session.transcript_revision = self.session.transcript_revision.wrapping_add(1);
        self.ui.history_render_cache = None;
    }
}
