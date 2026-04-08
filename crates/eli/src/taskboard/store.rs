//! SQLite-backed task persistence running on a dedicated thread.
//!
//! rusqlite is synchronous, so we run the database on a `std::thread`
//! and communicate via tokio channels to avoid blocking the async runtime.

use std::path::PathBuf;

use chrono::Utc;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use super::{NewTask, Status, Task, TaskEvent, TaskFilter, TaskId};

/// Handle for sending commands to the store thread.
#[derive(Clone)]
pub struct TaskStore {
    sender: mpsc::Sender<StoreCommand>,
    events: broadcast::Sender<TaskEvent>,
}

enum StoreCommand {
    Create(NewTask, oneshot::Sender<Result<TaskId, StoreError>>),
    Get(TaskId, oneshot::Sender<Option<Task>>),
    List(TaskFilter, oneshot::Sender<Vec<Task>>),
    UpdateStatus(TaskId, Status, oneshot::Sender<Result<(), StoreError>>),
    Complete(
        TaskId,
        serde_json::Value,
        oneshot::Sender<Result<(), StoreError>>,
    ),
    Fail(TaskId, String, oneshot::Sender<Result<(), StoreError>>),
    Cancel(TaskId, String, oneshot::Sender<Result<(), StoreError>>),
    ClaimNext(Vec<String>, String, oneshot::Sender<Option<Task>>),
    ActiveCount(oneshot::Sender<usize>),
    TaskDepth(TaskId, oneshot::Sender<u32>),
    CountRecent(String, i64, oneshot::Sender<usize>),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(String),
    #[error("task not found: {0}")]
    NotFound(TaskId),
    #[error("invalid transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
    #[error("rate limit exceeded: {0}")]
    RateLimit(String),
    #[error("max depth exceeded: {0}")]
    MaxDepth(u32),
    #[error("store channel closed")]
    ChannelClosed,
}

impl TaskStore {
    /// Open (or create) a task store at the given path.
    /// Spawns a dedicated OS thread for SQLite operations.
    pub fn open(db_path: PathBuf) -> Result<Self, StoreError> {
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        let (event_tx, _) = broadcast::channel(128);
        let event_tx_clone = event_tx.clone();

        std::thread::Builder::new()
            .name("taskboard-sqlite".into())
            .spawn(move || {
                if let Err(e) = store_thread(db_path, cmd_rx, event_tx_clone) {
                    error!("taskboard store thread exited with error: {e}");
                }
            })
            .map_err(|e| StoreError::Db(format!("failed to spawn store thread: {e}")))?;

        Ok(Self {
            sender: cmd_tx,
            events: event_tx,
        })
    }

    /// Open an in-memory store (for testing).
    pub fn open_memory() -> Result<Self, StoreError> {
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        let (event_tx, _) = broadcast::channel(128);
        let event_tx_clone = event_tx.clone();

        std::thread::Builder::new()
            .name("taskboard-sqlite-mem".into())
            .spawn(move || {
                if let Err(e) = store_thread_memory(cmd_rx, event_tx_clone) {
                    error!("taskboard store thread exited with error: {e}");
                }
            })
            .map_err(|e| StoreError::Db(format!("failed to spawn store thread: {e}")))?;

        Ok(Self {
            sender: cmd_tx,
            events: event_tx,
        })
    }

    /// Subscribe to task events.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.events.subscribe()
    }

    /// Create a new task. Returns its ID.
    pub async fn create(&self, new_task: NewTask) -> Result<TaskId, StoreError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(StoreCommand::Create(new_task, tx))
            .await
            .map_err(|_| StoreError::ChannelClosed)?;
        rx.await.map_err(|_| StoreError::ChannelClosed)?
    }

    /// Get a task by ID.
    pub async fn get(&self, id: TaskId) -> Option<Task> {
        let (tx, rx) = oneshot::channel();
        self.sender.send(StoreCommand::Get(id, tx)).await.ok()?;
        rx.await.ok()?
    }

    /// List tasks matching a filter.
    pub async fn list(&self, filter: TaskFilter) -> Vec<Task> {
        let (tx, rx) = oneshot::channel();
        if self
            .sender
            .send(StoreCommand::List(filter, tx))
            .await
            .is_err()
        {
            return vec![];
        }
        rx.await.unwrap_or_default()
    }

    /// Update a task's status.
    pub async fn update_status(&self, id: TaskId, status: Status) -> Result<(), StoreError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(StoreCommand::UpdateStatus(id, status, tx))
            .await
            .map_err(|_| StoreError::ChannelClosed)?;
        rx.await.map_err(|_| StoreError::ChannelClosed)?
    }

    /// Mark a task as completed with a result.
    pub async fn complete(&self, id: TaskId, result: serde_json::Value) -> Result<(), StoreError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(StoreCommand::Complete(id, result, tx))
            .await
            .map_err(|_| StoreError::ChannelClosed)?;
        rx.await.map_err(|_| StoreError::ChannelClosed)?
    }

    /// Mark a task as failed.
    pub async fn fail(&self, id: TaskId, error: String) -> Result<(), StoreError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(StoreCommand::Fail(id, error, tx))
            .await
            .map_err(|_| StoreError::ChannelClosed)?;
        rx.await.map_err(|_| StoreError::ChannelClosed)?
    }

    /// Cancel a task.
    pub async fn cancel(&self, id: TaskId, reason: String) -> Result<(), StoreError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(StoreCommand::Cancel(id, reason, tx))
            .await
            .map_err(|_| StoreError::ChannelClosed)?;
        rx.await.map_err(|_| StoreError::ChannelClosed)?
    }

    /// Atomically claim the next available task matching any of the given kinds.
    pub async fn claim_next(&self, kinds: &[String], agent_id: &str) -> Option<Task> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(StoreCommand::ClaimNext(
                kinds.to_vec(),
                agent_id.to_string(),
                tx,
            ))
            .await
            .ok()?;
        rx.await.ok()?
    }

    /// Count of non-terminal tasks.
    pub async fn active_count(&self) -> usize {
        let (tx, rx) = oneshot::channel();
        if self
            .sender
            .send(StoreCommand::ActiveCount(tx))
            .await
            .is_err()
        {
            return 0;
        }
        rx.await.unwrap_or(0)
    }

    /// Compute the depth of a task in its parent chain.
    pub async fn task_depth(&self, id: TaskId) -> u32 {
        let (tx, rx) = oneshot::channel();
        if self
            .sender
            .send(StoreCommand::TaskDepth(id, tx))
            .await
            .is_err()
        {
            return 0;
        }
        rx.await.unwrap_or(0)
    }

    /// Count tasks created by a session in the last N seconds.
    pub async fn count_recent(&self, session: &str, seconds: i64) -> usize {
        let (tx, rx) = oneshot::channel();
        if self
            .sender
            .send(StoreCommand::CountRecent(session.to_string(), seconds, tx))
            .await
            .is_err()
        {
            return 0;
        }
        rx.await.unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// SQLite store thread
// ---------------------------------------------------------------------------

fn store_thread(
    db_path: PathBuf,
    rx: mpsc::Receiver<StoreCommand>,
    events: broadcast::Sender<TaskEvent>,
) -> Result<(), StoreError> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| StoreError::Db(format!("create dir: {e}")))?;
    }
    let conn =
        rusqlite::Connection::open(&db_path).map_err(|e| StoreError::Db(format!("open: {e}")))?;
    init_schema(&conn)?;
    info!("taskboard store opened at {}", db_path.display());
    run_event_loop(conn, rx, events)
}

fn store_thread_memory(
    rx: mpsc::Receiver<StoreCommand>,
    events: broadcast::Sender<TaskEvent>,
) -> Result<(), StoreError> {
    let conn = rusqlite::Connection::open_in_memory()
        .map_err(|e| StoreError::Db(format!("open in-memory: {e}")))?;
    init_schema(&conn)?;
    debug!("taskboard store opened in-memory");
    run_event_loop(conn, rx, events)
}

fn init_schema(conn: &rusqlite::Connection) -> Result<(), StoreError> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .map_err(|e| StoreError::Db(format!("pragma: {e}")))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            status_json TEXT NOT NULL,
            parent TEXT,
            session_origin TEXT NOT NULL,
            context_json TEXT NOT NULL,
            result_json TEXT,
            assigned_to TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 1,
            metadata_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(json_extract(status_json, '$.state'));
        CREATE INDEX IF NOT EXISTS idx_tasks_kind ON tasks(kind);
        CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent);
        CREATE INDEX IF NOT EXISTS idx_tasks_session ON tasks(session_origin);
        CREATE INDEX IF NOT EXISTS idx_tasks_created ON tasks(created_at);",
    )
    .map_err(|e| StoreError::Db(format!("schema: {e}")))?;
    Ok(())
}

fn run_event_loop(
    conn: rusqlite::Connection,
    mut rx: mpsc::Receiver<StoreCommand>,
    events: broadcast::Sender<TaskEvent>,
) -> Result<(), StoreError> {
    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            StoreCommand::Create(new_task, reply) => {
                let task = new_task.into_task();
                let id = task.id;
                let res = insert_task(&conn, &task);
                if res.is_ok() {
                    let _ = events.send(TaskEvent::Created(id));
                }
                let _ = reply.send(res.map(|_| id));
            }
            StoreCommand::Get(id, reply) => {
                let _ = reply.send(get_task(&conn, id));
            }
            StoreCommand::List(filter, reply) => {
                let _ = reply.send(list_tasks(&conn, &filter));
            }
            StoreCommand::UpdateStatus(id, new_status, reply) => {
                let res = update_task_status(&conn, id, &new_status, &events);
                let _ = reply.send(res);
            }
            StoreCommand::Complete(id, result, reply) => {
                let res = complete_task(&conn, id, &result, &events);
                let _ = reply.send(res);
            }
            StoreCommand::Fail(id, error, reply) => {
                let res = fail_task(&conn, id, &error, &events);
                let _ = reply.send(res);
            }
            StoreCommand::Cancel(id, reason, reply) => {
                let res = cancel_task(&conn, id, &reason, &events);
                let _ = reply.send(res);
            }
            StoreCommand::ClaimNext(kinds, agent_id, reply) => {
                let _ = reply.send(claim_next_task(&conn, &kinds, &agent_id, &events));
            }
            StoreCommand::ActiveCount(reply) => {
                let count = conn
                    .query_row(
                        "SELECT COUNT(*) FROM tasks WHERE json_extract(status_json, '$.state') NOT IN ('done', 'failed', 'cancelled')",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                let _ = reply.send(count);
            }
            StoreCommand::TaskDepth(id, reply) => {
                let _ = reply.send(compute_depth(&conn, id));
            }
            StoreCommand::CountRecent(session, seconds, reply) => {
                let cutoff = (Utc::now() - chrono::Duration::seconds(seconds)).to_rfc3339();
                let count = conn
                    .query_row(
                        "SELECT COUNT(*) FROM tasks WHERE session_origin = ?1 AND created_at > ?2",
                        rusqlite::params![session, cutoff],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                let _ = reply.send(count);
            }
        }
    }
    info!("taskboard store thread shutting down");
    Ok(())
}

// ---------------------------------------------------------------------------
// SQL helpers
// ---------------------------------------------------------------------------

fn insert_task(conn: &rusqlite::Connection, task: &Task) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO tasks (id, kind, status_json, parent, session_origin, context_json, result_json, assigned_to, created_at, updated_at, priority, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            task.id.to_string(),
            task.kind,
            serde_json::to_string(&task.status).unwrap_or_default(),
            task.parent.map(|p| p.to_string()),
            task.session_origin,
            task.context.to_string(),
            task.result.as_ref().map(|r| r.to_string()),
            task.assigned_to,
            task.created_at.to_rfc3339(),
            task.updated_at.to_rfc3339(),
            task.priority,
            task.metadata.to_string(),
        ],
    )
    .map_err(|e| StoreError::Db(format!("insert: {e}")))?;
    Ok(())
}

fn get_task(conn: &rusqlite::Connection, id: TaskId) -> Option<Task> {
    conn.query_row(
        "SELECT id, kind, status_json, parent, session_origin, context_json, result_json, assigned_to, created_at, updated_at, priority, metadata_json FROM tasks WHERE id = ?1",
        rusqlite::params![id.to_string()],
        |row| Ok(row_to_task(row)),
    )
    .ok()?
}

fn list_tasks(conn: &rusqlite::Connection, filter: &TaskFilter) -> Vec<Task> {
    let mut sql = String::from(
        "SELECT id, kind, status_json, parent, session_origin, context_json, result_json, assigned_to, created_at, updated_at, priority, metadata_json FROM tasks WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];

    if let Some(ref status) = filter.status {
        sql.push_str(&format!(
            " AND json_extract(status_json, '$.state') = ?{}",
            params.len() + 1
        ));
        params.push(Box::new(status.clone()));
    }
    if let Some(ref kind) = filter.kind {
        sql.push_str(&format!(" AND kind = ?{}", params.len() + 1));
        params.push(Box::new(kind.clone()));
    }
    if let Some(parent) = filter.parent {
        sql.push_str(&format!(" AND parent = ?{}", params.len() + 1));
        params.push(Box::new(parent.to_string()));
    }
    if let Some(ref session) = filter.session_origin {
        sql.push_str(&format!(" AND session_origin = ?{}", params.len() + 1));
        params.push(Box::new(session.clone()));
    }
    sql.push_str(" ORDER BY priority DESC, created_at ASC");
    if let Some(limit) = filter.limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            warn!("taskboard list query failed: {e}");
            return vec![];
        }
    };
    let rows = match stmt.query_map(param_refs.as_slice(), |row| Ok(row_to_task(row))) {
        Ok(r) => r,
        Err(e) => {
            warn!("taskboard list query failed: {e}");
            return vec![];
        }
    };

    rows.filter_map(|r| r.ok().flatten()).collect()
}

fn update_task_status(
    conn: &rusqlite::Connection,
    id: TaskId,
    new_status: &Status,
    events: &broadcast::Sender<TaskEvent>,
) -> Result<(), StoreError> {
    let old = get_task(conn, id).ok_or(StoreError::NotFound(id))?;
    let status_json = serde_json::to_string(new_status).unwrap_or_default();
    let now = Utc::now().to_rfc3339();
    let assigned = match new_status {
        Status::Claimed { agent_id, .. } => Some(agent_id.clone()),
        _ => old.assigned_to.clone(),
    };

    conn.execute(
        "UPDATE tasks SET status_json = ?1, updated_at = ?2, assigned_to = ?3 WHERE id = ?4",
        rusqlite::params![status_json, now, assigned, id.to_string()],
    )
    .map_err(|e| StoreError::Db(format!("update status: {e}")))?;

    let _ = events.send(TaskEvent::StatusChanged {
        id,
        from: Box::new(old.status),
        to: Box::new(new_status.clone()),
    });
    Ok(())
}

fn complete_task(
    conn: &rusqlite::Connection,
    id: TaskId,
    result: &serde_json::Value,
    events: &broadcast::Sender<TaskEvent>,
) -> Result<(), StoreError> {
    let old = get_task(conn, id).ok_or(StoreError::NotFound(id))?;
    let status_json = serde_json::to_string(&Status::Done).unwrap_or_default();
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "UPDATE tasks SET status_json = ?1, result_json = ?2, updated_at = ?3 WHERE id = ?4",
        rusqlite::params![status_json, result.to_string(), now, id.to_string()],
    )
    .map_err(|e| StoreError::Db(format!("complete: {e}")))?;

    let _ = events.send(TaskEvent::StatusChanged {
        id,
        from: Box::new(old.status),
        to: Box::new(Status::Done),
    });
    let _ = events.send(TaskEvent::Completed {
        id,
        result: result.clone(),
    });
    Ok(())
}

fn fail_task(
    conn: &rusqlite::Connection,
    id: TaskId,
    error: &str,
    events: &broadcast::Sender<TaskEvent>,
) -> Result<(), StoreError> {
    let old = get_task(conn, id).ok_or(StoreError::NotFound(id))?;
    let new_status = Status::Failed {
        error: error.to_string(),
        agent_id: old.assigned_to.clone(),
        stage: None,
        tool_trace: vec![],
        retries: match &old.status {
            Status::Failed { retries, .. } => retries + 1,
            _ => 0,
        },
        suggested_fix: None,
    };
    let status_json = serde_json::to_string(&new_status).unwrap_or_default();
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "UPDATE tasks SET status_json = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![status_json, now, id.to_string()],
    )
    .map_err(|e| StoreError::Db(format!("fail: {e}")))?;

    let _ = events.send(TaskEvent::StatusChanged {
        id,
        from: Box::new(old.status),
        to: Box::new(new_status),
    });
    let _ = events.send(TaskEvent::Failed {
        id,
        error: error.to_string(),
    });
    Ok(())
}

fn cancel_task(
    conn: &rusqlite::Connection,
    id: TaskId,
    reason: &str,
    events: &broadcast::Sender<TaskEvent>,
) -> Result<(), StoreError> {
    let old = get_task(conn, id).ok_or(StoreError::NotFound(id))?;
    if old.status.is_terminal() {
        return Err(StoreError::InvalidTransition {
            from: old.status.label().into(),
            to: "cancelled".into(),
        });
    }
    let new_status = Status::Cancelled {
        reason: reason.to_string(),
    };
    let status_json = serde_json::to_string(&new_status).unwrap_or_default();
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "UPDATE tasks SET status_json = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![status_json, now, id.to_string()],
    )
    .map_err(|e| StoreError::Db(format!("cancel: {e}")))?;

    let _ = events.send(TaskEvent::StatusChanged {
        id,
        from: Box::new(old.status),
        to: Box::new(new_status),
    });
    Ok(())
}

fn claim_next_task(
    conn: &rusqlite::Connection,
    kinds: &[String],
    agent_id: &str,
    events: &broadcast::Sender<TaskEvent>,
) -> Option<Task> {
    if kinds.is_empty() {
        return None;
    }
    let placeholders: Vec<String> = (0..kinds.len()).map(|i| format!("?{}", i + 1)).collect();
    let sql = format!(
        "SELECT id FROM tasks WHERE json_extract(status_json, '$.state') = 'todo' AND kind IN ({}) ORDER BY priority DESC, created_at ASC LIMIT 1",
        placeholders.join(",")
    );

    let params: Vec<&dyn rusqlite::types::ToSql> = kinds
        .iter()
        .map(|k| k as &dyn rusqlite::types::ToSql)
        .collect();

    let task_id: Option<String> = conn
        .query_row(&sql, params.as_slice(), |row| row.get(0))
        .ok();

    let task_id = task_id?;
    let now = Utc::now();
    let new_status = Status::Claimed {
        agent_id: agent_id.to_string(),
        claimed_at: now,
    };
    let status_json = serde_json::to_string(&new_status).unwrap_or_default();

    // Atomic claim: only succeeds if still 'todo'
    let updated = conn
        .execute(
            "UPDATE tasks SET status_json = ?1, assigned_to = ?2, updated_at = ?3 WHERE id = ?4 AND json_extract(status_json, '$.state') = 'todo'",
            rusqlite::params![status_json, agent_id, now.to_rfc3339(), task_id],
        )
        .unwrap_or(0);

    if updated == 0 {
        return None;
    }

    let id = Uuid::parse_str(&task_id).ok()?;
    let _ = events.send(TaskEvent::StatusChanged {
        id,
        from: Box::new(Status::Todo),
        to: Box::new(new_status),
    });

    get_task(conn, id)
}

fn compute_depth(conn: &rusqlite::Connection, id: TaskId) -> u32 {
    let mut depth = 0u32;
    let mut current = Some(id);
    while let Some(cid) = current {
        let parent: Option<String> = conn
            .query_row(
                "SELECT parent FROM tasks WHERE id = ?1",
                rusqlite::params![cid.to_string()],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        match parent {
            Some(p) => {
                current = Uuid::parse_str(&p).ok();
                depth += 1;
                if depth > 100 {
                    break; // safety: prevent infinite loops from data corruption
                }
            }
            None => break,
        }
    }
    depth
}

fn row_to_task(row: &rusqlite::Row) -> Option<Task> {
    let id_str: String = row.get(0).ok()?;
    let id = Uuid::parse_str(&id_str).ok()?;
    let kind: String = row.get(1).ok()?;
    let status_json: String = row.get(2).ok()?;
    let status: Status = serde_json::from_str(&status_json).ok()?;
    let parent_str: Option<String> = row.get(3).ok()?;
    let parent = parent_str.and_then(|p| Uuid::parse_str(&p).ok());
    let session_origin: String = row.get(4).ok()?;
    let context_str: String = row.get(5).ok()?;
    let context: serde_json::Value = serde_json::from_str(&context_str).unwrap_or_default();
    let result_str: Option<String> = row.get(6).ok()?;
    let result = result_str.and_then(|r| serde_json::from_str(&r).ok());
    let assigned_to: Option<String> = row.get(7).ok()?;
    let created_str: String = row.get(8).ok()?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_str)
        .ok()?
        .with_timezone(&Utc);
    let updated_str: String = row.get(9).ok()?;
    let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_str)
        .ok()?
        .with_timezone(&Utc);
    let priority: u8 = row.get::<_, i32>(10).ok()?.try_into().unwrap_or(1);
    let meta_str: String = row.get(11).ok()?;
    let metadata: serde_json::Value = serde_json::from_str(&meta_str).unwrap_or_default();

    Some(Task {
        id,
        kind,
        status,
        parent,
        session_origin,
        context,
        result,
        assigned_to,
        created_at,
        updated_at,
        priority,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn test_store() -> TaskStore {
        TaskStore::open_memory().expect("open in-memory store")
    }

    #[tokio::test]
    async fn create_and_get() {
        let store = test_store().await;
        let id = store
            .create(NewTask {
                kind: "explore".into(),
                session_origin: "test".into(),
                context: json!({"prompt": "hello"}),
                parent: None,
                priority: 1,
                metadata: json!({}),
            })
            .await
            .unwrap();

        let task = store.get(id).await.unwrap();
        assert_eq!(task.kind, "explore");
        assert_eq!(task.status.label(), "todo");
    }

    #[tokio::test]
    async fn claim_atomicity() {
        let store = test_store().await;
        let id = store
            .create(NewTask {
                kind: "test".into(),
                session_origin: "s1".into(),
                context: json!({}),
                parent: None,
                priority: 1,
                metadata: json!({}),
            })
            .await
            .unwrap();

        // First claim succeeds
        let claimed = store.claim_next(&["test".into()], "worker-1").await;
        assert!(claimed.is_some());
        assert_eq!(claimed.unwrap().id, id);

        // Second claim returns None (already claimed)
        let claimed2 = store.claim_next(&["test".into()], "worker-2").await;
        assert!(claimed2.is_none());
    }

    #[tokio::test]
    async fn complete_and_fail() {
        let store = test_store().await;
        let id = store
            .create(NewTask {
                kind: "test".into(),
                session_origin: "s1".into(),
                context: json!({}),
                parent: None,
                priority: 1,
                metadata: json!({}),
            })
            .await
            .unwrap();

        store.complete(id, json!({"output": "done"})).await.unwrap();
        let task = store.get(id).await.unwrap();
        assert_eq!(task.status.label(), "done");
        assert!(task.result.is_some());
    }

    #[tokio::test]
    async fn cancel_terminal_fails() {
        let store = test_store().await;
        let id = store
            .create(NewTask {
                kind: "test".into(),
                session_origin: "s1".into(),
                context: json!({}),
                parent: None,
                priority: 1,
                metadata: json!({}),
            })
            .await
            .unwrap();

        store.complete(id, json!({})).await.unwrap();
        let res = store.cancel(id, "changed mind".into()).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn list_with_filter() {
        let store = test_store().await;
        for kind in &["explore", "implement", "explore"] {
            store
                .create(NewTask {
                    kind: kind.to_string(),
                    session_origin: "s1".into(),
                    context: json!({}),
                    parent: None,
                    priority: 1,
                    metadata: json!({}),
                })
                .await
                .unwrap();
        }

        let all = store.list(TaskFilter::default()).await;
        assert_eq!(all.len(), 3);

        let explores = store
            .list(TaskFilter {
                kind: Some("explore".into()),
                ..Default::default()
            })
            .await;
        assert_eq!(explores.len(), 2);
    }

    #[tokio::test]
    async fn task_depth_tracking() {
        let store = test_store().await;
        let root = store
            .create(NewTask {
                kind: "root".into(),
                session_origin: "s1".into(),
                context: json!({}),
                parent: None,
                priority: 1,
                metadata: json!({}),
            })
            .await
            .unwrap();

        let child = store
            .create(NewTask {
                kind: "child".into(),
                session_origin: "s1".into(),
                context: json!({}),
                parent: Some(root),
                priority: 1,
                metadata: json!({}),
            })
            .await
            .unwrap();

        assert_eq!(store.task_depth(root).await, 0);
        assert_eq!(store.task_depth(child).await, 1);
    }

    #[tokio::test]
    async fn persistence_survives_queries() {
        let store = test_store().await;
        let id = store
            .create(NewTask {
                kind: "persist-test".into(),
                session_origin: "s1".into(),
                context: json!({"data": 42}),
                parent: None,
                priority: 2,
                metadata: json!({"tag": "important"}),
            })
            .await
            .unwrap();

        // Re-read and verify all fields
        let task = store.get(id).await.unwrap();
        assert_eq!(task.priority, 2);
        assert_eq!(task.context["data"], 42);
        assert_eq!(task.metadata["tag"], "important");
    }
}
