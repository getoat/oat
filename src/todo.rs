use serde::{Deserialize, Serialize};

pub const MAX_TODO_TASKS: usize = 10;
pub const MAX_TODO_DESCRIPTION_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum TodoStatus {
    #[serde(rename = "todo")]
    Todo,
    #[serde(rename = "in progress")]
    InProgress,
    #[serde(rename = "done")]
    Done,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TodoTask {
    pub description: String,
    pub status: TodoStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TodoSnapshot {
    pub has_list: bool,
    pub tasks: Vec<TodoTask>,
}

impl TodoSnapshot {
    pub fn new(tasks: Vec<TodoTask>) -> Self {
        Self {
            has_list: true,
            tasks,
        }
    }

    pub fn cleared() -> Self {
        Self {
            has_list: false,
            tasks: Vec::new(),
        }
    }
}

pub fn validate_and_normalize_tasks(tasks: Vec<TodoTask>) -> Result<Vec<TodoTask>, String> {
    if tasks.is_empty() {
        return Err("Todo list must contain at least one task.".into());
    }

    if tasks.len() > MAX_TODO_TASKS {
        return Err(format!(
            "Todo list supports at most {MAX_TODO_TASKS} tasks."
        ));
    }

    tasks
        .into_iter()
        .map(validate_and_normalize_task)
        .collect::<Result<Vec<_>, _>>()
}

pub fn parse_snapshot(output: &str) -> Result<TodoSnapshot, serde_json::Error> {
    serde_json::from_str(output).or_else(|_| {
        let nested = serde_json::from_str::<String>(output)?;
        serde_json::from_str(&nested)
    })
}

fn validate_and_normalize_task(task: TodoTask) -> Result<TodoTask, String> {
    if task.description.contains(['\n', '\r']) {
        return Err("Todo task description must be a single line.".into());
    }

    let description = task.description.trim();
    if description.is_empty() {
        return Err("Todo task description must not be empty.".into());
    }

    if description.chars().count() > MAX_TODO_DESCRIPTION_CHARS {
        return Err(format!(
            "Todo task description must be at most {MAX_TODO_DESCRIPTION_CHARS} characters."
        ));
    }

    Ok(TodoTask {
        description: description.to_string(),
        status: task.status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_and_normalize_tasks_rejects_empty_lists() {
        let error = validate_and_normalize_tasks(Vec::new()).expect_err("empty tasks must fail");

        assert!(error.contains("at least one task"));
    }

    #[test]
    fn validate_and_normalize_tasks_trims_descriptions() {
        let tasks = validate_and_normalize_tasks(vec![TodoTask {
            description: "  Investigate failing build.  ".into(),
            status: TodoStatus::InProgress,
        }])
        .expect("tasks should normalize");

        assert_eq!(tasks[0].description, "Investigate failing build.");
    }

    #[test]
    fn validate_and_normalize_tasks_rejects_multiline_descriptions() {
        let error = validate_and_normalize_tasks(vec![TodoTask {
            description: "Investigate\nfailing build.".into(),
            status: TodoStatus::Todo,
        }])
        .expect_err("multiline task must fail");

        assert!(error.contains("single line"));
    }

    #[test]
    fn validate_and_normalize_tasks_rejects_overlong_descriptions() {
        let description = "a".repeat(MAX_TODO_DESCRIPTION_CHARS + 1);
        let error = validate_and_normalize_tasks(vec![TodoTask {
            description,
            status: TodoStatus::Done,
        }])
        .expect_err("overlong task must fail");

        assert!(error.contains("at most"));
    }

    #[test]
    fn parse_snapshot_round_trips_cleared_state() {
        let encoded = serde_json::to_string(&TodoSnapshot::cleared()).expect("serialize");
        let parsed = parse_snapshot(&encoded).expect("parse");

        assert_eq!(parsed, TodoSnapshot::cleared());
    }

    #[test]
    fn parse_snapshot_accepts_json_string_wrapped_payloads() {
        let inner = serde_json::to_string(&TodoSnapshot::new(vec![TodoTask {
            description: "Track parser fallback.".into(),
            status: TodoStatus::Todo,
        }]))
        .expect("serialize inner");
        let wrapped = serde_json::to_string(&inner).expect("serialize wrapped");

        let parsed = parse_snapshot(&wrapped).expect("parse wrapped");
        assert_eq!(
            parsed,
            TodoSnapshot::new(vec![TodoTask {
                description: "Track parser fallback.".into(),
                status: TodoStatus::Todo,
            }])
        );
    }
}
