//! Standalone task board — persistent, queryable task tracking for multi-agent work.
//!
//! This module is **decoupled** from eli's framework and hook system.
//! It has zero imports from `framework.rs`, `hooks.rs`, or `control_plane.rs`.
//! Integration with eli happens through a thin plugin adapter in `builtin/taskboard_plugin.rs`.

pub mod store;
pub mod worker;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique task identifier.
pub type TaskId = Uuid;

/// A persistent, trackable unit of work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    /// Free-form task kind (e.g. "explore", "implement", "review").
    /// Intentionally not an enum to avoid calcification.
    pub kind: String,
    pub status: Status,
    /// Parent task for decomposition.
    pub parent: Option<TaskId>,
    /// Session that created this task.
    pub session_origin: String,
    /// Task input / context.
    pub context: serde_json::Value,
    /// Task output / result.
    pub result: Option<serde_json::Value>,
    /// Agent currently assigned.
    pub assigned_to: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 0=low, 1=normal, 2=high, 3=urgent.
    pub priority: u8,
    /// Extensible metadata.
    pub metadata: serde_json::Value,
}

/// Task lifecycle status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Todo,
    Claimed {
        agent_id: String,
        claimed_at: DateTime<Utc>,
    },
    Running {
        progress: f32,
        last_heartbeat: DateTime<Utc>,
    },
    Done,
    Failed {
        error: String,
        agent_id: Option<String>,
        stage: Option<String>,
        tool_trace: Vec<String>,
        retries: u32,
        suggested_fix: Option<String>,
    },
    Blocked {
        reason: String,
        waiting_on: Option<TaskId>,
    },
    Cancelled {
        reason: String,
    },
}

impl Status {
    /// Short label for display.
    pub fn label(&self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Claimed { .. } => "claimed",
            Status::Running { .. } => "running",
            Status::Done => "done",
            Status::Failed { .. } => "failed",
            Status::Blocked { .. } => "blocked",
            Status::Cancelled { .. } => "cancelled",
        }
    }

    /// Whether this status represents a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Status::Done | Status::Failed { .. } | Status::Cancelled { .. }
        )
    }
}

/// Events emitted by the task board.
#[derive(Debug, Clone)]
pub enum TaskEvent {
    Created(TaskId),
    StatusChanged {
        id: TaskId,
        from: Box<Status>,
        to: Box<Status>,
    },
    Completed {
        id: TaskId,
        result: serde_json::Value,
    },
    Failed {
        id: TaskId,
        error: String,
    },
}

/// Filter criteria for listing tasks.
#[derive(Debug, Default, Clone)]
pub struct TaskFilter {
    pub status: Option<String>,
    pub kind: Option<String>,
    pub parent: Option<TaskId>,
    pub session_origin: Option<String>,
    pub limit: Option<usize>,
}

/// Builder for creating new tasks.
pub struct NewTask {
    pub kind: String,
    pub session_origin: String,
    pub context: serde_json::Value,
    pub parent: Option<TaskId>,
    pub priority: u8,
    pub metadata: serde_json::Value,
}

impl NewTask {
    pub fn into_task(self) -> Task {
        let now = Utc::now();
        Task {
            id: Uuid::new_v4(),
            kind: self.kind,
            status: Status::Todo,
            parent: self.parent,
            session_origin: self.session_origin,
            context: self.context,
            result: None,
            assigned_to: None,
            created_at: now,
            updated_at: now,
            priority: self.priority,
            metadata: self.metadata,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_labels() {
        assert_eq!(Status::Todo.label(), "todo");
        assert_eq!(Status::Done.label(), "done");
        assert!(Status::Done.is_terminal());
        assert!(!Status::Todo.is_terminal());
    }

    #[test]
    fn new_task_builder() {
        let task = NewTask {
            kind: "explore".into(),
            session_origin: "test-session".into(),
            context: serde_json::json!({"prompt": "find callers"}),
            parent: None,
            priority: 1,
            metadata: serde_json::Value::Null,
        }
        .into_task();

        assert_eq!(task.kind, "explore");
        assert_eq!(task.status.label(), "todo");
        assert!(task.result.is_none());
    }

    #[test]
    fn status_serialization_roundtrip() {
        let status = Status::Failed {
            error: "timeout".into(),
            agent_id: Some("worker-1".into()),
            stage: Some("execute".into()),
            tool_trace: vec!["fs.read".into()],
            retries: 2,
            suggested_fix: Some("increase timeout".into()),
        };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: Status = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.label(), "failed");
    }
}
