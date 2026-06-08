use burd_protocol::default_state_dir;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionStatus {
    pub id: String,
    pub name: String,
    pub status: String,
    pub start_time: String,
    pub end_time: Option<String>,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub start_time: String,
    pub end_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLogs {
    pub task_id: String,
    pub logs: Vec<String>,
}

pub fn record_action(
    name: &str,
    status: &str,
    task_title: &str,
    description: &str,
    logs: Vec<String>,
) -> Result<ActionStatus, String> {
    let now = Utc::now().to_rfc3339();
    let suffix = Utc::now().timestamp_millis();
    let task_id = format!("task-{suffix}");
    let action = ActionStatus {
        id: format!("action-{suffix}"),
        name: name.to_string(),
        status: status.to_string(),
        start_time: now.clone(),
        end_time: Some(now.clone()),
        tasks: vec![Task {
            id: task_id.clone(),
            title: task_title.to_string(),
            description: description.to_string(),
            status: status.to_string(),
            start_time: now,
            end_time: Some(Utc::now().to_rfc3339()),
        }],
    };

    let mut actions = load_actions()?;
    actions.push(action.clone());
    save_json(actions_path(), &actions)?;

    let mut all_logs = load_logs()?;
    all_logs.push(TaskLogs {
        task_id,
        logs: if logs.is_empty() {
            vec![format!("{name}: {status}")]
        } else {
            logs
        },
    });
    save_json(logs_path(), &all_logs)?;

    Ok(action)
}

pub fn load_actions() -> Result<Vec<ActionStatus>, String> {
    load_json(actions_path()).or_else(|error| {
        if error.contains("not found") {
            Ok(Vec::new())
        } else {
            Err(error)
        }
    })
}

pub fn load_logs() -> Result<Vec<TaskLogs>, String> {
    load_json(logs_path()).or_else(|error| {
        if error.contains("not found") {
            Ok(Vec::new())
        } else {
            Err(error)
        }
    })
}

pub fn load_logs_for_task(task_id: &str) -> Result<Vec<TaskLogs>, String> {
    Ok(load_logs()?
        .into_iter()
        .filter(|entry| entry.task_id == task_id)
        .collect())
}

pub fn logs_summary() -> Result<serde_json::Value, String> {
    let actions = load_actions()?;
    let logs = load_logs()?;
    Ok(serde_json::json!({
        "actions_total": actions.len(),
        "logs_total": logs.len(),
        "latest_action": actions.last(),
    }))
}

fn actions_path() -> PathBuf {
    default_state_dir().join("actions.json")
}

fn logs_path() -> PathBuf {
    default_state_dir().join("logs.json")
}

fn load_json<T: for<'de> Deserialize<'de>>(path: PathBuf) -> Result<T, String> {
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("{} not found or unreadable: {error}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("invalid JSON at {}: {error}", path.display()))
}

fn save_json<T: Serialize>(path: PathBuf, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("failed to serialize JSON: {error}"))?;
    fs::write(&path, json).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_status_serializes() {
        let action = ActionStatus {
            id: "action".to_string(),
            name: "report generation".to_string(),
            status: "completed".to_string(),
            start_time: "2026-06-08T00:00:00Z".to_string(),
            end_time: Some("2026-06-08T00:00:01Z".to_string()),
            tasks: vec![Task {
                id: "task".to_string(),
                title: "Generate report".to_string(),
                description: "Build local report".to_string(),
                status: "completed".to_string(),
                start_time: "2026-06-08T00:00:00Z".to_string(),
                end_time: Some("2026-06-08T00:00:01Z".to_string()),
            }],
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("completed"));
    }
}
