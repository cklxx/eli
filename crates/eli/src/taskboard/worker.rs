//! Serial task worker — polls the board and executes tasks via Claude Code CLI.
//!
//! Each task is executed by spawning a Claude Code (or Codex/Kimi) subprocess.
//! The worker captures output, updates heartbeat during execution, and reports
//! results back to the task store.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::Status;
use super::store::TaskStore;

/// Max output bytes to capture from the CLI process.
const MAX_OUTPUT_BYTES: usize = 50_000;

/// Heartbeat interval while a task is running.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// A serial worker that claims tasks and executes them via Claude Code CLI.
pub struct TaskWorker {
    store: TaskStore,
    /// Task kinds this worker can handle.
    capabilities: Vec<String>,
    /// Agent identifier for claim tracking.
    agent_id: String,
    /// Interval between poll attempts.
    poll_interval: Duration,
    /// Working directory for task execution.
    workspace: PathBuf,
}

impl TaskWorker {
    pub fn new(
        store: TaskStore,
        capabilities: Vec<String>,
        agent_id: String,
        workspace: PathBuf,
    ) -> Self {
        Self {
            store,
            capabilities,
            agent_id,
            poll_interval: Duration::from_secs(2),
            workspace,
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
            workspace = %self.workspace.display(),
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
        info!(task_id = %task_id, kind = %kind, "claimed task");

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

        // Execute via Claude Code CLI
        match self.execute_via_cli(&task).await {
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

    /// Execute a task by spawning a Claude Code CLI process.
    async fn execute_via_cli(
        &self,
        task: &super::Task,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let prompt = task
            .context
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or("task context missing 'prompt' field")?;

        // Detect available CLI
        let cli = detect_cli().await.ok_or(
            "no Claude Code CLI found — install claude, codex, or kimi and ensure it's in PATH",
        )?;

        debug!(task_id = %task.id, cli = %cli.name, "spawning CLI process");

        // Write prompt to temp file
        let prompt_file = write_prompt_tempfile(prompt)?;
        let prompt_path = prompt_file.path().to_path_buf();

        // Build command
        let shell_cmd = build_cli_command(&cli, &prompt_path);

        // Spawn process
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&shell_cmd)
            .current_dir(&self.workspace)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn CLI: {e}"))?;

        let mut stdout = child.stdout.take().expect("stdout piped");
        let mut stderr = child.stderr.take().expect("stderr piped");

        // Read output with periodic heartbeat updates
        let task_id = task.id;
        let store = self.store.clone();
        let mut output = Vec::new();
        let mut err_output = Vec::new();
        let mut buf = vec![0u8; 4096];
        let mut err_buf = vec![0u8; 4096];
        let mut stdout_done = false;
        let mut stderr_done = false;
        let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        heartbeat_interval.tick().await; // consume first immediate tick

        loop {
            tokio::select! {
                // Read stdout
                result = stdout.read(&mut buf), if !stdout_done => {
                    match result {
                        Ok(0) => stdout_done = true,
                        Ok(n) => {
                            if output.len() < MAX_OUTPUT_BYTES {
                                let take = n.min(MAX_OUTPUT_BYTES - output.len());
                                output.extend_from_slice(&buf[..take]);
                            }
                        }
                        Err(e) => {
                            warn!(task_id = %task_id, error = %e, "stdout read error");
                            stdout_done = true;
                        }
                    }
                }
                // Read stderr
                result = stderr.read(&mut err_buf), if !stderr_done => {
                    match result {
                        Ok(0) => stderr_done = true,
                        Ok(n) => {
                            if err_output.len() < MAX_OUTPUT_BYTES {
                                let take = n.min(MAX_OUTPUT_BYTES - err_output.len());
                                err_output.extend_from_slice(&err_buf[..take]);
                            }
                        }
                        Err(e) => {
                            warn!(task_id = %task_id, error = %e, "stderr read error");
                            stderr_done = true;
                        }
                    }
                }
                // Heartbeat
                _ = heartbeat_interval.tick() => {
                    let _ = store.update_status(
                        task_id,
                        Status::Running {
                            progress: 0.5, // indeterminate
                            last_heartbeat: chrono::Utc::now(),
                        },
                    ).await;
                }
                // Process exit (only check when both streams are done)
                status = child.wait(), if stdout_done && stderr_done => {
                    let exit_code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
                    let stdout_str = String::from_utf8_lossy(&output).to_string();
                    let stderr_str = String::from_utf8_lossy(&err_output).to_string();

                    debug!(
                        task_id = %task_id,
                        exit_code = exit_code,
                        stdout_len = stdout_str.len(),
                        stderr_len = stderr_str.len(),
                        "CLI process exited"
                    );

                    if exit_code != 0 {
                        let error_detail = if stderr_str.is_empty() {
                            stdout_str.clone()
                        } else {
                            stderr_str.clone()
                        };
                        return Err(format!(
                            "CLI exited with code {exit_code}: {}",
                            truncate_output(&error_detail, 500)
                        ).into());
                    }

                    return Ok(serde_json::json!({
                        "output": truncate_output(&stdout_str, 2000),
                        "exit_code": exit_code,
                        "cli": cli.name,
                        "agent_id": self.agent_id,
                    }));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CLI detection — standalone, no imports from builtin/
// ---------------------------------------------------------------------------

struct CliInfo {
    name: String,
    path: String,
}

/// Detect the first available Claude Code CLI in PATH.
async fn detect_cli() -> Option<CliInfo> {
    for name in ["claude", "codex", "kimi"] {
        if let Ok(output) = Command::new("which").arg(name).output().await {
            if !output.status.success() {
                continue;
            }
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Some(CliInfo {
                name: name.to_string(),
                path,
            });
        }
    }
    None
}

/// Build the shell command to run the CLI with a prompt file.
fn build_cli_command(cli: &CliInfo, prompt_path: &Path) -> String {
    let quoted_path = shell_quote(&cli.path);
    let quoted_prompt = shell_quote(&prompt_path.to_string_lossy());

    match cli.name.as_str() {
        "claude" => format!("{quoted_path} -p --output-format text < {quoted_prompt}"),
        "codex" => format!("{quoted_path} exec < {quoted_prompt}"),
        "kimi" => format!("{quoted_path} -p \"$(cat {quoted_prompt})\" --print"),
        _ => format!("{quoted_path} < {quoted_prompt}"),
    }
}

/// Write a prompt to a temporary file (kept alive by the returned NamedTempFile).
fn write_prompt_tempfile(
    prompt: &str,
) -> Result<tempfile::NamedTempFile, Box<dyn std::error::Error + Send + Sync>> {
    let mut file = tempfile::Builder::new()
        .prefix(".eli-task-prompt-")
        .tempfile()
        .map_err(|e| format!("failed to create prompt tempfile: {e}"))?;
    file.write_all(prompt.as_bytes())
        .map_err(|e| format!("failed to write prompt: {e}"))?;
    file.flush()
        .map_err(|e| format!("failed to flush prompt: {e}"))?;
    Ok(file)
}

/// Quote a path for safe shell usage.
fn shell_quote(s: &str) -> String {
    if s.contains(|c: char| c.is_whitespace() || c == '\'' || c == '"' || c == '\\') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

/// Truncate output to max bytes, preserving UTF-8 boundaries.
fn truncate_output(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..s.floor_char_boundary(max)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taskboard::NewTask;
    use crate::taskboard::store::TaskStore;
    use serde_json::json;

    #[test]
    fn test_build_cli_command_claude() {
        let cli = CliInfo {
            name: "claude".into(),
            path: "/usr/local/bin/claude".into(),
        };
        let cmd = build_cli_command(&cli, Path::new("/tmp/prompt.txt"));
        assert!(cmd.contains("claude"));
        assert!(cmd.contains("-p"));
        assert!(cmd.contains("--output-format text"));
        assert!(cmd.contains("/tmp/prompt.txt"));
    }

    #[test]
    fn test_build_cli_command_codex() {
        let cli = CliInfo {
            name: "codex".into(),
            path: "/usr/bin/codex".into(),
        };
        let cmd = build_cli_command(&cli, Path::new("/tmp/prompt.txt"));
        assert!(cmd.contains("codex"));
        assert!(cmd.contains("exec"));
    }

    #[test]
    fn test_shell_quote_simple() {
        assert_eq!(shell_quote("hello"), "hello");
        assert_eq!(shell_quote("/usr/bin/foo"), "/usr/bin/foo");
    }

    #[test]
    fn test_shell_quote_spaces() {
        let quoted = shell_quote("/path/with spaces/bin");
        assert!(quoted.starts_with('\''));
        assert!(quoted.ends_with('\''));
    }

    #[test]
    fn test_truncate_output() {
        assert_eq!(truncate_output("hello", 10), "hello");
        assert_eq!(truncate_output("hello world", 5).len(), 5);
    }

    #[test]
    fn test_write_prompt_tempfile() {
        let f = write_prompt_tempfile("test prompt").unwrap();
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(content, "test prompt");
    }

    #[tokio::test]
    async fn test_detect_cli_runs() {
        // Just verify it doesn't panic — actual result depends on environment
        let _result = detect_cli().await;
    }

    #[tokio::test]
    async fn worker_claims_and_executes_with_echo() {
        // Use a simple echo command to test the execution flow
        // We test the store interaction, not the actual CLI
        let store = TaskStore::open_memory().unwrap();

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

        // Manually claim and verify store works
        let claimed = store.claim_next(&["test".into()], "worker-1").await;
        assert!(claimed.is_some());
        assert_eq!(claimed.unwrap().id, id);

        // Complete the task
        store
            .complete(id, json!({"output": "test output"}))
            .await
            .unwrap();

        let task = store.get(id).await.unwrap();
        assert_eq!(task.status.label(), "done");
        assert!(task.result.is_some());
    }

    #[tokio::test]
    async fn worker_run_loop_cancellation() {
        let store = TaskStore::open_memory().unwrap();
        let workspace = std::env::current_dir().unwrap();
        let worker = TaskWorker::new(store, vec!["test".into()], "worker".into(), workspace)
            .with_poll_interval(Duration::from_millis(50));

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(async move {
            worker.run(cancel_clone).await;
        });

        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel.cancel();
        handle.await.unwrap();
    }
}
