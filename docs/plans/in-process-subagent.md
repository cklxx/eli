# Agent Tool Upgrade — Engineering Plan

**Date**: 2026-03-31
**Status**: Implemented — all 5 workstreams shipped
**Source**: Claude Code AgentTool engineering optimizations → Eli

---

## Goal

Upgrade the `subagent` tool into a full `agent` tool with: **sync + async modes**, **agent type system**, **worktree isolation**, **agent tracking/kill**, **structured results**, and **in-process fallback** when no external CLI is available. External CLIs (claude/codex/kimi) remain the primary execution engine.

---

## Architecture

### Execution Strategy

```
Model calls agent(prompt, subagent_type, run_in_background, isolation, ...)
  │
  ├─ resolve CLI (claude > codex > kimi)
  │   ├─ [found] spawn CLI subprocess (existing ShellManager path)
  │   └─ [not found] fallback: run agent_loop() in-process
  │
  ├─ [sync, default] wait for completion → return structured result
  └─ [async] return agent_id → monitor → inject_inbound on completion
```

### What Changes vs Current

| Current (`subagent`) | New (`agent`) |
|---|---|
| Always async (fire-and-forget) | Sync by default, async opt-in |
| No agent types | 3 built-in types with different capabilities |
| No kill/status | AgentTracker with kill, status, result retrieval |
| No isolation | Optional git worktree isolation |
| Plain text return | Structured JSON result (tokens, duration, status) |
| CLI-only, fails without CLI | CLI primary, in-process fallback |
| No concurrent cap | Max 5 concurrent background agents |

---

## Workstreams

### WS1: Agent Tool (replaces `tool_subagent`)

**Files**: `crates/eli/src/builtin/tools.rs`

New schema:

```json
{
  "properties": {
    "prompt": { "type": "string" },
    "description": { "type": "string", "description": "3-5 word summary" },
    "subagent_type": { "type": "string", "enum": ["general-purpose", "explore", "plan"] },
    "run_in_background": { "type": "boolean" },
    "isolation": { "type": "string", "enum": ["worktree"] },
    "cwd": { "type": "string" },
    "cli": { "type": "string" }
  },
  "required": ["prompt", "description"]
}
```

**Sync path** (default): spawn CLI → `wait_closed` → return structured result.
**Async path** (`run_in_background: true`): existing fire-and-forget pattern.
**Fallback**: if no CLI found, run `agent_loop()` in-process with filtered tools.

### WS2: AgentTracker (kill/status/result)

**Files**: NEW `crates/eli/src/builtin/subagent/tracker.rs`

Global tracker for background agents:

```rust
pub struct AgentTracker {
    agents: RwLock<HashMap<String, TrackedAgent>>,
    max_concurrent: usize,  // ELI_MAX_CONCURRENT_AGENTS, default 5
}

struct TrackedAgent {
    shell_id: Option<String>,        // if CLI-based
    cancellation: CancellationToken,  // if in-process
    started_at: Instant,
    agent_type: String,
    prompt_summary: String,
    result: Option<AgentResult>,
}
```

Tools: `agent.status`, `agent.kill`, `agent.result`

### WS3: Worktree Isolation

**Files**: NEW `crates/eli/src/builtin/subagent/worktree.rs`

When `isolation: "worktree"`:
1. `git worktree add <tmp> --detach`
2. Set cwd to worktree for CLI spawn
3. After completion: check `git diff --stat`
4. No changes → `git worktree remove`; changes → return path + branch

### WS4: In-Process Fallback

**Files**: NEW `crates/eli/src/builtin/subagent/fallback.rs`

When no external CLI is available, run `agent_loop()` in-process:
- Create ephemeral `TapeService` (InMemoryTapeStore already exists)
- Filter tools based on agent type (block recursive `agent` calls)
- Isolated `TurnContext` with own `CancellationToken`
- Return same structured result format

### WS5: Tests

Unit tests in `crates/eli/src/builtin/subagent/tests.rs`:
- Tool filter logic
- AgentTracker concurrent cap
- Worktree create/cleanup
- Structured result format
- In-process fallback smoke test

---

## Implementation Order

```
WS1 (agent tool + sync mode) → WS2 (tracker) → WS3 (worktree) → WS4 (fallback) → WS5 (tests)
```

All in one pass — WS1 is the core, each subsequent WS wires into it.

---

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Sync mode blocks parent turn too long | Default timeout 300s, model can set `run_in_background: true` for long tasks |
| Recursive agent spawn | In-process fallback always blocks `agent` tool |
| Too many background agents | AgentTracker enforces max concurrent (default 5) |
| Worktree leak on crash | Drop guard + startup sweep of stale worktrees |
| CLI not found, fallback also fails | Clear error message listing what was tried |
