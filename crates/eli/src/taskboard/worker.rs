//! Serial task worker — polls the board and executes one task at a time.

use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::Status;
use super::store::TaskStore;

/// A simple serial worker that claims and executes tasks one at a time.
pub struct TaskWorker {
    store: TaskStore,
    /// Task kinds this worker can handle.
    capabilities: Vec<String>,
    /// Agent identifier for claim tracking.
    agent_id: String,
    /// Interval between poll attempts.
    poll_interval: Duration,
}

impl TaskWorker {
    pub fn new(store: TaskStore, capabilities: Vec<String>, agent_id: String) -> Self {
        Self {
            store,
            capabilities,
            agent_id,
            poll_interval: Duration::from_secs(2),
        }
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Run the worker loop until cancelled.
    pub async fn run(&self, cancel: CancellationToken) {
        info!(
            agent_id = %self.agent_id,
            capabilities = ?self.capabilities,
            "task worker started"
        );

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!(agent_id = %self.agent_id, "task worker shutting down");
                    break;
                }
                _ = tokio::time::sleep(self.poll_interval) => {
                    self.poll_and_execute().await;
                }
            }
        }
    }

    async fn poll_and_execute(&self) {
        let task = match self
            .store
            .claim_next(&self.capabilities, &self.agent_id)
            .await
        {
            Some(t) => t,
            None => return,
        };

        let task_id = task.id;
        let kind = task.kind.clone();
        info!(
            task_id = %task_id,
            kind = %kind,
            "claimed task"
        );

        // Transition to running
        if let Err(e) = self
            .store
            .update_status(
                task_id,
                Status::Running {
                    progress: 0.0,
                    last_heartbeat: chrono::Utc::now(),
                },
            )
            .await
        {
            warn!(task_id = %task_id, error = %e, "failed to set task to running");
            return;
        }

        // Execute the task.
        // Phase 1: delegate to a simple executor that processes the task context.
        // In future phases, this will use process_inbound with synthetic envelopes.
        match self.execute_task(&task).await {
            Ok(result) => {
                if let Err(e) = self.store.complete(task_id, result).await {
                    error!(task_id = %task_id, error = %e, "failed to mark task complete");
                }
                info!(task_id = %task_id, kind = %kind, "task completed");
            }
            Err(e) => {
                let error_msg = format!("{e}");
                if let Err(store_err) = self.store.fail(task_id, error_msg.clone()).await {
                    error!(task_id = %task_id, error = %store_err, "failed to mark task failed");
                }
                warn!(task_id = %task_id, kind = %kind, error = %error_msg, "task failed");
            }
        }
    }

    /// Execute a single task. Returns the result value on success.
    ///
    /// Phase 1 implementation: extracts the prompt from context and returns it
    /// as a placeholder. The real implementation will use process_inbound via
    /// the plugin adapter.
    async fn execute_task(
        &self,
        task: &super::Task,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        // Phase 1: basic execution — extract prompt from context
        let prompt = task
            .context
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("(no prompt)");

        debug!(
            task_id = %task.id,
            prompt = %prompt,
            "executing task"
        );

        // Placeholder: in Phase 2, this will call process_inbound via the
        // taskboard plugin adapter, using worktree isolation for code tasks.
        Ok(serde_json::json!({
            "status": "executed",
            "prompt": prompt,
            "agent_id": self.agent_id,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taskboard::NewTask;
    use crate::taskboard::store::TaskStore;
    use serde_json::json;

    #[tokio::test]
    async fn worker_claims_and_completes() {
        let store = TaskStore::open_memory().unwrap();
        let worker = TaskWorker::new(store.clone(), vec!["test".into()], "test-worker".into());

        // Create a task
        let id = store
            .create(NewTask {
                kind: "test".into(),
                session_origin: "s1".into(),
                context: json!({"prompt": "hello"}),
                parent: None,
                priority: 1,
                metadata: json!({}),
            })
            .await
            .unwrap();

        // Worker polls and executes
        worker.poll_and_execute().await;

        // Task should be completed
        let task = store.get(id).await.unwrap();
        assert_eq!(task.status.label(), "done");
        assert!(task.result.is_some());
    }

    #[tokio::test]
    async fn worker_ignores_non_matching_kinds() {
        let store = TaskStore::open_memory().unwrap();
        let worker = TaskWorker::new(store.clone(), vec!["explore".into()], "explorer".into());

        // Create a task with a different kind
        store
            .create(NewTask {
                kind: "implement".into(),
                session_origin: "s1".into(),
                context: json!({}),
                parent: None,
                priority: 1,
                metadata: json!({}),
            })
            .await
            .unwrap();

        // Worker should not claim it
        worker.poll_and_execute().await;

        let tasks = store.list(crate::taskboard::TaskFilter::default()).await;
        assert_eq!(tasks[0].status.label(), "todo");
    }

    #[tokio::test]
    async fn worker_run_loop_cancellation() {
        let store = TaskStore::open_memory().unwrap();
        let worker = TaskWorker::new(store.clone(), vec!["test".into()], "worker".into())
            .with_poll_interval(Duration::from_millis(50));

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(async move {
            worker.run(cancel_clone).await;
        });

        // Let it poll a couple times
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel.cancel();
        handle.await.unwrap();
    }
}
