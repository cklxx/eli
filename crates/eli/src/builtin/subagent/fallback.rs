//! In-process fallback: run `agent_loop()` when no external CLI is available.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use nexil::ConduitError;
use nexil::tape::InMemoryTapeStore;
use serde_json::Value;

use crate::builtin::agent::Agent;
use crate::builtin::store::ForkTapeStore;
use crate::builtin::tape::TapeService;
use crate::types::PromptValue;

/// Result from an in-process agent run.
pub struct FallbackResult {
    pub content: String,
    pub duration_ms: u64,
}

/// Run the agent loop in-process with an ephemeral tape.
///
/// Used as a fallback when no external CLI (claude/codex/kimi) is found.
/// The agent gets its own isolated tape and tool set; the `agent` tool is
/// excluded to prevent infinite recursion.
pub async fn run_in_process(
    prompt: &str,
    workspace: &str,
    model_override: Option<&str>,
) -> Result<FallbackResult, ConduitError> {
    let start = Instant::now();

    // Build an isolated agent with ephemeral tape.
    let mut agent = Agent::new();
    let mem_store = InMemoryTapeStore::new();
    let fork_store = ForkTapeStore::from_sync(mem_store);
    let tapes_dir = std::env::temp_dir().join("eli-fallback-tapes");
    let tapes = TapeService::new(tapes_dir, fork_store);

    // Override the agent's tape service to use our ephemeral one.
    agent.set_tapes(tapes);

    let session_id = format!("fallback-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Build minimal state — only workspace, blocked tools.
    let mut state: HashMap<String, Value> = HashMap::new();
    state.insert(
        "_runtime_workspace".to_owned(),
        Value::String(workspace.to_owned()),
    );

    // Block the agent/subagent tools to prevent recursion.
    let blocked = HashSet::from(["agent".to_owned(), "subagent".to_owned()]);
    let all_tools: HashSet<String> = {
        let reg = crate::tools::REGISTRY
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reg.keys()
            .filter(|k| !blocked.contains(k.as_str()))
            .cloned()
            .collect()
    };

    let content = agent
        .run(
            &session_id,
            PromptValue::Text(prompt.to_owned()),
            &state,
            model_override,
            None,
            Some(&all_tools),
        )
        .await?;

    Ok(FallbackResult {
        content,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}
