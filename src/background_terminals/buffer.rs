use std::collections::VecDeque;

use crate::token_counting::TokenCounter;

#[derive(Clone, Debug)]
struct OutputChunk {
    sequence: u64,
    text: String,
    tokens: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BufferReadResult {
    pub(crate) sequence: u64,
    pub(crate) text: String,
    pub(crate) output_truncated: bool,
    pub(crate) cursor_expired: bool,
}

#[derive(Clone)]
pub(crate) struct TokenTailBuffer {
    tokenizer: Option<TokenCounter>,
    max_tokens: usize,
    chunks: VecDeque<OutputChunk>,
    retained_tokens: usize,
    next_sequence: u64,
    output_truncated: bool,
}

impl TokenTailBuffer {
    pub(crate) fn new(max_tokens: usize) -> Self {
        Self {
            tokenizer: TokenCounter::cl100k().ok(),
            max_tokens,
            chunks: VecDeque::new(),
            retained_tokens: 0,
            next_sequence: 0,
            output_truncated: false,
        }
    }

    pub(crate) fn append(&mut self, text: String) {
        if text.is_empty() {
            return;
        }

        let (text, tokens, truncated_self) = self.trim_to_budget(text);
        self.output_truncated |= truncated_self;
        if text.is_empty() || tokens == 0 {
            return;
        }

        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.retained_tokens += tokens;
        self.chunks.push_back(OutputChunk {
            sequence: self.next_sequence,
            text,
            tokens,
        });
        self.prune_front();
    }

    pub(crate) fn retained_tokens(&self) -> u64 {
        self.retained_tokens as u64
    }

    pub(crate) fn sequence(&self) -> u64 {
        self.next_sequence
    }

    pub(crate) fn output_truncated(&self) -> bool {
        self.output_truncated
    }

    pub(crate) fn read_after(&self, after_sequence: Option<u64>) -> BufferReadResult {
        let sequence = self.sequence();
        let Some(after_sequence) = after_sequence else {
            return BufferReadResult {
                sequence,
                text: self
                    .chunks
                    .iter()
                    .map(|chunk| chunk.text.as_str())
                    .collect::<String>(),
                output_truncated: self.output_truncated,
                cursor_expired: false,
            };
        };

        let earliest_sequence = self.chunks.front().map(|chunk| chunk.sequence);
        let cursor_expired = earliest_sequence
            .map(|earliest| after_sequence < earliest.saturating_sub(1))
            .unwrap_or(false);
        let text = if cursor_expired {
            self.chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<String>()
        } else {
            self.chunks
                .iter()
                .filter(|chunk| chunk.sequence > after_sequence)
                .map(|chunk| chunk.text.as_str())
                .collect::<String>()
        };

        BufferReadResult {
            sequence,
            text,
            output_truncated: self.output_truncated || cursor_expired,
            cursor_expired,
        }
    }

    fn prune_front(&mut self) {
        while self.retained_tokens > self.max_tokens {
            let Some(chunk) = self.chunks.pop_front() else {
                break;
            };
            self.retained_tokens = self.retained_tokens.saturating_sub(chunk.tokens);
            self.output_truncated = true;
        }
    }

    fn trim_to_budget(&self, text: String) -> (String, usize, bool) {
        let tokens = self.count_tokens(&text);
        if tokens <= self.max_tokens {
            return (text, tokens, false);
        }

        let Some(tokenizer) = self.tokenizer.as_ref() else {
            let trimmed = text
                .chars()
                .rev()
                .take(self.max_tokens.saturating_mul(4))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            let trimmed_tokens = self.count_tokens(&trimmed);
            return (trimmed, trimmed_tokens.min(self.max_tokens), true);
        };

        let encoded = tokenizer.encode_text(&text);
        let keep_from = encoded.len().saturating_sub(self.max_tokens);
        let trimmed = tokenizer
            .decode_tokens(encoded[keep_from..].to_vec())
            .unwrap_or_else(|_| text);
        let trimmed_tokens = self.count_tokens(&trimmed);
        (trimmed, trimmed_tokens.min(self.max_tokens), true)
    }

    fn count_tokens(&self, text: &str) -> usize {
        self.tokenizer
            .as_ref()
            .map(|tokenizer| tokenizer.count_text(text))
            .unwrap_or_else(|| text.chars().count().div_ceil(4))
    }
}

#[cfg(test)]
mod tests {
    use super::TokenTailBuffer;

    #[test]
    fn read_after_returns_only_newer_chunks() {
        let mut buffer = TokenTailBuffer::new(100);
        buffer.append("first\n".into());
        let first_sequence = buffer.sequence();
        buffer.append("second\n".into());

        let read = buffer.read_after(Some(first_sequence));
        assert_eq!(read.text, "second\n");
        assert_eq!(read.sequence, buffer.sequence());
    }

    #[test]
    fn pruning_marks_output_as_truncated() {
        let mut buffer = TokenTailBuffer::new(3);
        buffer.append("one ".into());
        buffer.append("two ".into());
        buffer.append("three ".into());

        let read = buffer.read_after(None);
        assert!(read.output_truncated);
        assert!(!read.text.is_empty());
    }
}
