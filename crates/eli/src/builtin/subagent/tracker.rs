//! Global tracker for background agents — status, kill, result retrieval.

use std::collections::HashMap;
use std::time::Instant;

use tokio::sync::RwLock;

/// Maximum concurrent background agents (overridden by `ELI_MAX_CONCURRENT_AGENTS`).
fn max_concurrent() -> usize {
    std::env::var("ELI_MAX_CONCURRENT_AGENTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5)
}

// ---------------------------------------------------------------------------
// TrackedAgent
// ---------------------------------------------------------------------------

/// Metadata and result for a single background agent.
pub struct TrackedAgent {
    pub shell_id: Option<String>,
    pub agent_type: String,
    pub prompt_summary: String,
    pub cwd: String,
    pub cli: String,
    pub started_at: Instant,
    pub result: Option<AgentResult>,
}

/// Final result once an agent completes.
#[derive(Clone)]
pub struct AgentResult {
    pub exit_code: Option<i32>,
    pub output: String,
    pub artifacts: String,
    pub duration_ms: u64,
}

impl TrackedAgent {
    pub fn is_running(&self) -> bool {
        self.result.is_none()
    }
}

// ---------------------------------------------------------------------------
// AgentTracker
// ---------------------------------------------------------------------------

/// Global registry of background agents.
pub struct AgentTracker {
    agents: RwLock<HashMap<String, TrackedAgent>>,
}

impl Default for AgentTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentTracker {
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
        }
    }

    /// Number of currently running (not completed) background agents.
    pub async fn running_count(&self) -> usize {
        self.agents
            .read()
            .await
            .values()
            .filter(|a| a.is_running())
            .count()
    }

    /// Check if we can spawn another background agent.
    pub async fn can_spawn(&self) -> bool {
        self.running_count().await < max_concurrent()
    }

    /// Register a new background agent. Returns false if at capacity.
    pub async fn register(
        &self,
        agent_id: &str,
        shell_id: Option<String>,
        agent_type: &str,
        prompt_summary: &str,
        cwd: &str,
        cli: &str,
    ) -> bool {
        if !self.can_spawn().await {
            return false;
        }
        let entry = TrackedAgent {
            shell_id,
            agent_type: agent_type.to_owned(),
            prompt_summary: prompt_summary.to_owned(),
            cwd: cwd.to_owned(),
            cli: cli.to_owned(),
            started_at: Instant::now(),
            result: None,
        };
        self.agents.write().await.insert(agent_id.to_owned(), entry);
        true
    }

    /// Record completion of an agent.
    pub async fn complete(&self, agent_id: &str, result: AgentResult) {
        if let Some(agent) = self.agents.write().await.get_mut(agent_id) {
            agent.result = Some(result);
        }
    }

    /// Kill a background agent by shell ID.
    pub async fn kill(&self, agent_id: &str) -> Option<AgentResult> {
        let shell_id = {
            let agents = self.agents.read().await;
            let agent = agents.get(agent_id)?;
            if !agent.is_running() {
                return agent.result.clone();
            }
            agent.shell_id.clone()
        };

        // Terminate the underlying shell process.
        if let Some(ref sid) = shell_id {
            let mgr = crate::builtin::shell_manager::shell_manager();
            let (output, exit_code, _) = mgr
                .terminate(sid)
                .await
                .unwrap_or_else(|e| (format!("kill error: {e}"), Some(-1), "error".to_owned()));
            let result = AgentResult {
                exit_code,
                output,
                artifacts: "(killed before completion)".to_owned(),
                duration_ms: {
                    let agents = self.agents.read().await;
                    agents
                        .get(agent_id)
                        .map(|a| a.started_at.elapsed().as_millis() as u64)
                        .unwrap_or(0)
                },
            };
            self.complete(agent_id, result.clone()).await;
            return Some(result);
        }
        None
    }

    /// Get a snapshot of all agents for display.
    pub async fn list(&self) -> Vec<(String, AgentSummary)> {
        self.agents
            .read()
            .await
            .iter()
            .map(|(id, a)| {
                (
                    id.clone(),
                    AgentSummary {
                        agent_type: a.agent_type.clone(),
                        prompt_summary: a.prompt_summary.clone(),
                        cli: a.cli.clone(),
                        running: a.is_running(),
                        elapsed_ms: a.started_at.elapsed().as_millis() as u64,
                        exit_code: a.result.as_ref().and_then(|r| r.exit_code),
                    },
                )
            })
            .collect()
    }

    /// Get the result of a completed agent.
    pub async fn get_result(&self, agent_id: &str) -> Option<AgentResult> {
        self.agents
            .read()
            .await
            .get(agent_id)
            .and_then(|a| a.result.clone())
    }

    /// Evict completed agents older than `max_age`.
    pub async fn evict_completed(&self, max_age: std::time::Duration) {
        let mut agents = self.agents.write().await;
        agents.retain(|_, a| a.is_running() || a.started_at.elapsed() < max_age);
    }
}

/// Summary for display.
pub struct AgentSummary {
    pub agent_type: String,
    pub prompt_summary: String,
    pub cli: String,
    pub running: bool,
    pub elapsed_ms: u64,
    pub exit_code: Option<i32>,
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

static AGENT_TRACKER: std::sync::LazyLock<AgentTracker> =
    std::sync::LazyLock::new(AgentTracker::new);

pub fn agent_tracker() -> &'static AgentTracker {
    &AGENT_TRACKER
}
