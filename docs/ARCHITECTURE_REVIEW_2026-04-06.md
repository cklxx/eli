# 2026-04-06 · Architecture & Code Quality Review

## Executive Summary

The codebase is **functionally correct** and well-organized at the module level. However, it exhibits classic AI-generated code patterns: excessive cloning, duplicated logic disguised by different variable names, type aliases that weaken contracts, and scattered constants. This review identifies **5 critical**, **8 major**, and **7 moderate** issues across both crates.

---

## Critical Issues

### C1. `Envelope = Value` Erases Type Safety

**Location:** `crates/eli/src/types.rs:11`

```rust
pub type Envelope = Value;
pub type State = HashMap<String, Value>;
```

`Envelope` is just `serde_json::Value`. Every function that accepts `Envelope` actually accepts *any* JSON—a number, a null, a malformed object. The compiler cannot enforce that an envelope has `content`, `session_id`, or `channel` fields. This is the root cause of the repeated manual JSON construction scattered across the codebase.

**Impact:** framework.rs alone constructs outbound envelopes in 3+ places with identical field-extraction boilerplate. Each is a divergence risk.

**Fix:** Replace with a newtype or struct:
```rust
pub struct Envelope {
    pub content: String,
    pub session_id: String,
    pub channel: String,
    pub chat_id: String,
    pub output_channel: String,
    pub extra: HashMap<String, Value>,
}
```

---

### C2. String-Typed `TapeEntry.kind` — No Compile-Time Safety

**Location:** `crates/nexil/src/tape/entries.rs`

`TapeEntry.kind` is a `String`, compared against magic literals in **40+ locations** across both crates:

```rust
entry.kind == "anchor"
entry.kind == "message"
entry.kind == "system"
entry.kind == "event"
entry.kind == "tool_call"
entry.kind == "tool_result"
entry.kind == "decision"
entry.kind == "decision_revoked"
```

A typo like `"mesage"` compiles and silently filters nothing.

**Fix:** Replace with an enum:
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TapeEntryKind {
    Anchor, Message, System, Event, ToolCall, ToolResult, Decision, DecisionRevoked,
}
```

---

### C3. Outbound Envelope Construction Duplicated 3+ Times

**Location:** `crates/eli/src/framework.rs` — lines ~271-316, ~485-504

Three functions build identical outbound JSON:

```rust
let channel = inbound.field_str("channel", "");
let chat_id = inbound.field_str("chat_id", "");
let output_channel = inbound
    .get("output_channel")
    .and_then(|v| v.as_str())
    .unwrap_or(&channel)
    .to_owned();
serde_json::json!({
    "content": reply,
    "session_id": session_id,
    "channel": channel,
    "chat_id": chat_id,
    "output_channel": output_channel,
})
```

Copy-paste code, different variable names. Classic AI duplication.

**Fix:** Extract `fn build_routed_outbound(inbound: &Value, session_id: &str, content: &str) -> Value`.

---

### C4. 238 `.unwrap()` Calls in nexil (Non-Test Code)

**Location:** Throughout `crates/nexil/src/`

238 `.unwrap()` calls outside test modules. Hotspots:
- `tape/spill.rs` — **64 unwraps** (file I/O paths)
- `tape/manager.rs` — **39 unwraps**
- `tools/executor.rs` — **22 unwraps**
- `auth/openai_codex.rs` — **29 unwraps**
- `core/execution.rs` — **14 unwraps**

Many are on `.as_str()`, `.as_object()`, `.as_array()` chains that **will** panic on unexpected JSON shapes.

**Fix:** Audit each call. Replace with `ok_or_else(|| ConduitError::new(...))` or `.unwrap_or_default()` where safe.

---

### C5. `run_chat()` and `run_tools()` — God Functions

**Location:**
- `crates/nexil/src/core/execution.rs::run_chat()` — ~180 lines
- `crates/nexil/src/llm/tool_loop.rs::run_tools()` — ~157 lines

`run_chat()` handles: provider resolution, client caching, request building, HTTP execution, response parsing, error classification, retry logic, and fallback model iteration — all in one function.

`run_tools()` combines: message prep, iteration control, tape persistence, recovery nudging, tool execution, and result accumulation.

**Fix:** Extract into phases:
- `run_chat()` → `prepare_attempt()`, `execute_attempt()`, `classify_and_retry()`
- `run_tools()` → `prepare_round()`, `execute_round()`, `persist_round()` (partially done, but orchestrator still monolithic)

---

## Major Issues

### M1. 38 Identical Poison-Guard Lock Patterns

**Location:** 10 files across `crates/eli/src/`

```rust
self.agents.write().unwrap_or_else(|e| e.into_inner())
```

This pattern appears **38 times**. It recovers from poisoned locks by extracting the inner value — meaning if a thread panicked while holding the lock, the code silently continues with potentially corrupted state.

**Fix:** Pick a strategy:
- If panics are acceptable: `.lock().expect("lock poisoned")`
- If recovery is needed: use `parking_lot::Mutex` which never poisons

---

### M2. Multiple Message-Building Paths (4 Implementations)

**Location:**
- `crates/nexil/src/llm/helpers.rs::build_messages()`
- `crates/nexil/src/tape/context.rs::build_messages()`
- `crates/nexil/src/llm/helpers.rs::build_full_context_from_entries()`
- `crates/nexil/src/tape/manager.rs` — inline building

Four separate functions convert tape entries into LLM message arrays. Each handles system prompts, anchoring, and message ordering slightly differently.

**Fix:** Single `MessageBuilder` struct that all paths delegate to.

---

### M3. Tool Registry Dual-Write Anti-Pattern

**Location:** `crates/eli/src/tools.rs`

```rust
pub static REGISTRY: LazyLock<Mutex<HashMap<String, Tool>>> = ...
pub static MODEL_TOOLS_CACHE: LazyLock<Mutex<Vec<Tool>>> = ...
```

Two global mutable stores hold overlapping tool data. `MODEL_TOOLS_CACHE` is populated manually and never invalidated. If a plugin registers a tool dynamically, the cache is stale.

**Fix:** Single source of truth. Derive the cache on-demand or use a version counter.

---

### M4. Agent vs. Subagent Initialization Duplication

**Location:**
- `crates/eli/src/builtin/agent/mod.rs` — `Agent::new()`
- `crates/eli/src/builtin/subagent/fallback.rs` — inline setup

Nearly identical LLM initialization, workspace injection, and tool filtering. Changes to one must be mirrored in the other.

**Fix:** Shared `AgentFactory::create(config)` that both paths use.

---

### M5. CLI Commands Repeat Framework Setup

**Location:**
- `crates/eli/src/builtin/cli/chat.rs`
- `crates/eli/src/builtin/cli/run.rs`
- `crates/eli/src/builtin/cli/gateway.rs`

Each CLI command independently builds the inbound envelope, initializes the framework, processes the result, and renders output. Identical patterns, different files.

**Fix:** Extract `cli/common.rs` with `run_with_framework(inbound) -> Result`.

---

### M6. Excessive Cloning in Retry/Tool Loops

**Location:** `crates/nexil/src/llm/tool_loop.rs`, `crates/nexil/src/core/execution.rs`

```rust
// Cloned on every retry iteration
let mut candidates = vec![(self.provider.clone(), self.model.clone())];
tools_payload: tools_payload.clone(),
reasoning_effort: reasoning_effort.clone(),
kwargs: kwargs.clone(),
```

With `max_retries=3` and multiple fallback models, message vectors are cloned 40+ times unnecessarily.

**Fix:** Use `&[Value]` references through the retry loop. Clone only when mutation is actually needed (`Cow<'_, [Value]>`).

---

### M7. Hook System Swallows Errors Silently

**Location:** `crates/eli/src/hooks.rs` — `call_notify_all!` macro

Notification hooks (`save_state`, `on_error`) catch panics and only log. A `save_state` hook can fail silently, losing data. No way for callers to know that state was not saved.

**Fix:** At minimum, return a `Vec<HookError>` from notification hooks so callers can decide whether to proceed.

---

### M8. Two Channel Traits for the Same Concept

**Location:**
- `crates/eli/src/channels/base.rs` — `Channel` trait (transport-level)
- `crates/eli/src/hooks.rs` — `ChannelHook` trait (framework-level)

Both represent "a thing that receives and sends messages" but with different start signatures and lifecycle management. New channel implementors must understand which to use.

**Fix:** Unify into one trait, or clearly document when each applies with a decision flowchart.

---

## Moderate Issues

### m1. Magic Constants Scattered Across 6+ Files

`DEFAULT_HARD_CAP: 32_000`, `DEFAULT_SESSION_TTL_SECS: 1800`, `INBOUND_CHANNEL_CAPACITY: 256`, `BASH_OUTPUT_LARGE_THRESHOLD: 30_000`, etc. — no central configuration.

**Fix:** `builtin/constants.rs` or a config struct.

### m2. Tape Anchor Handling Duplicated 4 Times

`TapeEntry::anchor(name, state, ...)` constructed in `tape.rs::ensure_bootstrap_anchor()`, `tape.rs::handoff()`, `tape.rs::reset()`, and `agent/agent_run.rs::write_handoff_anchor()`.

**Fix:** Anchor factory functions in `TapeEntry`.

### m3. PromptBuilder Over-Parameterized for 3 Modes

`PromptMode` enum with priority-sorted sections, truncation, re-sorting — ~150 lines. Only 3 modes exist. Three functions would be simpler and more readable.

### m4. `ValueExt` Trait Has 9 Methods with Inconsistent Naming

`field()` vs `get_str_field()` vs `require_str_field()` — three naming conventions for "get a string from JSON". Should consolidate to 3-4 methods with consistent naming.

### m5. `control_plane.rs` Task-Local Context — Implicit Dependencies

`TURN_CTX.try_with(...)` called in 12+ places with inconsistent fallback behavior. Some return `None`, some return empty `Vec`. Callers can't tell from signatures that they need to be inside a turn context.

### m6. Shell Manager Nested Mutex/Arc

Double-locked pattern `self.shells.lock().await` → `shell_arc.lock().await`. Background read tasks are spawned but never joined on cleanup.

### m7. Inconsistent Error Types Across Crate Boundary

`HookError` (eli) wraps `anyhow::Error`. `ConduitError` (nexil) uses `ErrorKind + String`. No conversion traits. Manual mapping at boundaries.

---

## Pattern Summary: Classic AI Code Smells

| Smell | Occurrences | Root Cause |
|-------|-------------|------------|
| **Copy-paste with renamed variables** | Envelope construction (3x), agent init (2x), CLI setup (3x) | AI generates per-callsite instead of extracting helpers |
| **Type aliases that weaken contracts** | `Envelope = Value`, `State = HashMap` | AI uses flexible types instead of domain types |
| **String-typed enums** | `TapeEntry.kind`, tape event names | AI avoids enum boilerplate |
| **Excessive `.clone()`** | 16 in tool_loop.rs alone | AI defaults to owned types to avoid lifetime reasoning |
| **God functions** | `run_chat()` 180 lines, `process_inbound()` 142 lines | AI generates linear flows instead of composed phases |
| **Global mutable state** | `REGISTRY`, `MODEL_TOOLS_CACHE`, `SIDECAR_URL` | AI reaches for globals instead of dependency injection |
| **Scattered constants** | 6+ files with magic numbers | AI inlines values at point of use |

---

## Recommended Refactoring Priority

### Phase 1 — Type Safety (High Impact, Low Risk)
1. Replace `TapeEntry.kind: String` with enum (~2h, touches tests heavily but mechanically)
2. Replace `Envelope = Value` with newtype struct (~4h, cascading changes)
3. Consolidate outbound construction into helper (~1h)

### Phase 2 — Deduplication (High Impact, Medium Risk)
4. Single `MessageBuilder` to replace 4 message-building paths (~4h)
5. `AgentFactory` shared between agent and subagent (~2h)
6. CLI `common.rs` extraction (~1h)
7. Anchor/event factory functions in `TapeEntry` (~2h)

### Phase 3 — Structural (Medium Impact, Higher Risk)
8. Break up `run_chat()` and `run_tools()` (~4h)
9. Unify channel traits or document boundary (~3h)
10. Centralize constants and configuration (~2h)
11. Audit and fix `.unwrap()` calls in nexil (~6h)

### Phase 4 — Polish
12. Lock strategy decision (parking_lot or expect) (~1h)
13. Hook error propagation (~3h)
14. Tool registry single source of truth (~2h)

**Estimated total: ~37 hours**

---

## What's Done Well

- **Module organization** — two-crate split is clean, dependency direction correct
- **Hook system design** — "last-registered wins" is a good plugin model
- **Tape abstraction** — append-only with anchoring is clever and debuggable
- **Turn pipeline** — 7-stage linear pipeline is easy to reason about
- **Feature flags** — gateway and tape-viewer properly gated
- **Workspace dependencies** — consistent version management
