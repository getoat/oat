use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

use bytes::Bytes;
use serde_json::Value;

pub(crate) const OAT_INTERACTION_SCOPE_HEADER: &str = "x-oat-interaction-scope";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResponsesHostedToolEvent {
    WebSearchStarted { id: String, detail: String },
    WebSearchCompleted { id: String, detail: String },
}

type HostedToolEventSink = Arc<dyn Fn(ResponsesHostedToolEvent) + Send + Sync>;

static OBSERVERS: LazyLock<Mutex<HashMap<String, Arc<ResponsesSearchObserver>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Default)]
struct ObserverState {
    buffer: String,
}

pub(crate) struct ResponsesSearchObserverGuard {
    scope: String,
}

impl ResponsesSearchObserverGuard {
    pub(crate) fn register(scope: String, sink: HostedToolEventSink) -> Self {
        OBSERVERS
            .lock()
            .expect("responses search observer lock")
            .insert(scope.clone(), Arc::new(ResponsesSearchObserver::new(sink)));
        Self { scope }
    }
}

impl Drop for ResponsesSearchObserverGuard {
    fn drop(&mut self) {
        OBSERVERS
            .lock()
            .expect("responses search observer lock")
            .remove(&self.scope);
    }
}

pub(crate) fn observer_for_scope(scope: &str) -> Option<Arc<ResponsesSearchObserver>> {
    OBSERVERS
        .lock()
        .expect("responses search observer lock")
        .get(scope)
        .cloned()
}

pub(crate) struct ResponsesSearchObserver {
    sink: HostedToolEventSink,
    state: Mutex<ObserverState>,
}

impl ResponsesSearchObserver {
    fn new(sink: HostedToolEventSink) -> Self {
        Self {
            sink,
            state: Mutex::new(ObserverState::default()),
        }
    }

    pub(crate) fn observe_chunk(&self, chunk: &Bytes) {
        let text = String::from_utf8_lossy(chunk)
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let mut state = self.state.lock().expect("responses search observer state");
        state.buffer.push_str(&text);

        while let Some(delimiter) = state.buffer.find("\n\n") {
            let raw_event = state.buffer[..delimiter].to_string();
            state.buffer.drain(..delimiter + 2);
            drop(state);
            self.process_raw_event(&raw_event);
            state = self.state.lock().expect("responses search observer state");
        }
    }

    fn process_raw_event(&self, raw_event: &str) {
        let data = raw_event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            return;
        }

        let Ok(payload) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        let Some(kind) = payload.get("type").and_then(Value::as_str) else {
            return;
        };
        let Some(item) = payload.get("item").and_then(Value::as_object) else {
            return;
        };
        if item.get("type").and_then(Value::as_str) != Some("web_search_call") {
            return;
        }

        let id = item
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| item.get("call_id").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            return;
        }

        let detail = web_search_detail(item);
        let event = match kind {
            "response.output_item.added" => {
                ResponsesHostedToolEvent::WebSearchStarted { id, detail }
            }
            "response.output_item.done" => {
                ResponsesHostedToolEvent::WebSearchCompleted { id, detail }
            }
            _ => return,
        };
        (self.sink)(event);
    }
}

fn web_search_detail(item: &serde_json::Map<String, Value>) -> String {
    let action = item.get("action").and_then(Value::as_object);
    let fallback_query = item
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let detail = action.map(web_search_action_detail).unwrap_or_default();
    if detail.is_empty() {
        fallback_query
    } else {
        detail
    }
}

fn web_search_action_detail(action: &serde_json::Map<String, Value>) -> String {
    match action.get("type").and_then(Value::as_str) {
        Some("search") => {
            let query = action
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !query.is_empty() {
                return query;
            }

            let Some(queries) = action.get("queries").and_then(Value::as_array) else {
                return String::new();
            };
            let first = queries
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if first.is_empty() {
                String::new()
            } else if queries.len() > 1 {
                format!("{first} ...")
            } else {
                first
            }
        }
        Some("open_page") => action
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        Some("find_in_page") => {
            let pattern = action.get("pattern").and_then(Value::as_str);
            let url = action.get("url").and_then(Value::as_str);
            match (pattern, url) {
                (Some(pattern), Some(url)) => format!("'{pattern}' in {url}"),
                (Some(pattern), None) => format!("'{pattern}'"),
                (None, Some(url)) => url.to_string(),
                (None, None) => String::new(),
            }
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_events(chunks: &[&str]) -> Vec<ResponsesHostedToolEvent> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        let observer = ResponsesSearchObserver::new(Arc::new(move |event| {
            sink_events
                .lock()
                .expect("captured events lock")
                .push(event);
        }));
        for chunk in chunks {
            observer.observe_chunk(&Bytes::from(chunk.to_string()));
        }
        events.lock().expect("captured events lock").clone()
    }

    #[test]
    fn emits_start_and_completion_for_web_search_call() {
        let events = capture_events(&[
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"ws_1\",\"type\":\"web_search_call\",\"action\":{\"type\":\"search\",\"query\":\"latest rust news\"}}}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"ws_1\",\"type\":\"web_search_call\",\"action\":{\"type\":\"search\",\"query\":\"latest rust news\"}}}\n\n",
        ]);

        assert_eq!(
            events,
            vec![
                ResponsesHostedToolEvent::WebSearchStarted {
                    id: "ws_1".into(),
                    detail: "latest rust news".into(),
                },
                ResponsesHostedToolEvent::WebSearchCompleted {
                    id: "ws_1".into(),
                    detail: "latest rust news".into(),
                },
            ]
        );
    }

    #[test]
    fn ignores_non_search_output_items() {
        let events = capture_events(&[
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"name\":\"List\"}}}\n\n",
        ]);

        assert!(events.is_empty());
    }

    #[test]
    fn handles_split_sse_chunks() {
        let events = capture_events(&[
            "data: {\"type\":\"response.output_item.added\",\"item\":",
            "{\"id\":\"ws_1\",\"type\":\"web_search_call\",\"action\":{\"type\":\"search\",\"queries\":[\"one\",\"two\"]}}}\n\n",
        ]);

        assert_eq!(
            events,
            vec![ResponsesHostedToolEvent::WebSearchStarted {
                id: "ws_1".into(),
                detail: "one ...".into(),
            }]
        );
    }
}
