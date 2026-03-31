# Agent Stability, Recoverability & Parallel Tasks — Hardening Plan

**Date**: 2026-03-31
**Status**: Implemented — 4 items shipped, eng review cleared

---

## Goal

Harden the agent framework so that: (1) crashes don't lose data, (2) transient failures self-heal, (3) concurrent load doesn't cause cross-contamination.

---

## Workstream 1: Recoverability

### 1A. Tape Write Atomicity (FileTapeStore)
**Problem**: `FileTapeStore` via `AsyncTapeStoreAdapter<spawn_blocking>` has no file-level locking. Concurrent processes writing same tape can corrupt entries.
**Fix**: Add `flock(LOCK_EX)` around write operations in `InMemoryTapeStore::append()` when backed by file. Or: use append-only JSONL format with record separator + length prefix so partial writes are detectable and skippable on read.
**Files**: `crates/nexil/src/tape/store.rs`
**Complexity**: Low

### 1B. Tape Entry Validation on Read
**Problem**: `fetch_all` doesn't validate entry integrity. Partial writes (crash mid-append) produce corrupt JSON.
**Fix**: Each entry written as `\n{json}\n` with a trailing checksum line. On read, skip entries that fail checksum. Log warning for skipped entries.
**Files**: `crates/nexil/src/tape/store.rs`
**Complexity**: Low

### 1C. Session TTL & Cleanup
**Problem**: `BuiltinImpl.agents` HashMap grows forever — no eviction for abandoned sessions.
**Fix**: Add `last_active: Instant` to per-session entry. Background task sweeps every 5 minutes, evicts sessions idle > configurable TTL (default 30 min). Evicted sessions get tape flushed before removal.
**Files**: `crates/eli/src/builtin/mod.rs`
**Complexity**: Medium

### 1D. Inbound Message Journaling
**Problem**: If framework crashes during `process_inbound`, the inbound message is lost.
**Fix**: Append inbound envelope to tape *before* processing. Mark with `role: "inbound_journal"`. On recovery, detect un-processed journal entries and replay.
**Files**: `crates/eli/src/framework.rs`, `crates/nexil/src/tape/store.rs`
**Complexity**: Medium — requires journal entry type + replay logic

---

## Workstream 2: Stability

### 2A. Channel Dispatch Retry
**Problem**: `ChannelManager::dispatch()` logs error and returns false on send failure. Message is lost.
**Fix**: Add retry with exponential backoff (3 attempts, 1s/2s/4s). After exhausting retries, persist failed outbound to a dead-letter file for manual recovery.
**Files**: `crates/eli/src/channels/manager.rs`
**Complexity**: Low

### 2B. Per-Tool Timeout Enforcement
**Problem**: No timeout on individual tool calls. A hanging tool blocks the entire turn indefinitely.
**Fix**: Wrap each `tool.run()` in `tokio::time::timeout(duration)`. Default 60s, configurable per-tool via `Tool::timeout()` method. Timeout returns error payload to model.
**Files**: `crates/nexil/src/tools/executor.rs`
**Complexity**: Low

### 2C. Tool Circuit Breaker
**Problem**: If a tool repeatedly fails, the model keeps retrying it, wasting tokens. Only 1 recovery nudge exists.
**Fix**: Track consecutive failures per tool name within a turn. After 3 consecutive failures of same tool, inject a "tool unavailable" message and remove it from available tools for remainder of turn.
**Files**: `crates/eli/src/builtin/agent/agent_run.rs`, `crates/nexil/src/tools/executor.rs`
**Complexity**: Medium

---

## Workstream 3: Parallel & Isolation

### 3A. Bounded Inbound Channel
**Problem**: `mpsc::UnboundedSender<ChannelMessage>` — rapid message flood can spike memory.
**Fix**: Replace with bounded channel (capacity 256). Channels apply backpressure when full. Log warning when >80% capacity.
**Files**: `crates/eli/src/channels/manager.rs`
**Complexity**: Low

### 3B. Per-Session HTTP Client Pool
**Problem**: Shared `ClientRegistry` — SSE error eviction affects all sessions using that provider.
**Fix**: Key client cache by `(provider, session_id)` instead of just provider. Eviction only affects the session that hit the error.
**Files**: `crates/nexil/src/core/client_registry.rs` (or equivalent)
**Complexity**: Medium — need to thread session_id through client creation

---

## Priority & Sequencing

**Phase A** (highest impact, lowest risk):
1. **2B** Per-tool timeout — prevents indefinite hangs
2. **2A** Channel dispatch retry — prevents silent message loss
3. **1A** Tape write atomicity — prevents corruption

**Phase B** (important hardening):
4. **1B** Tape entry validation — resilience to partial writes
5. **1C** Session TTL — prevents memory leaks
6. **3A** Bounded inbound channel — prevents memory spikes

**Phase C** (advanced):
7. **2C** Tool circuit breaker — prevents token waste
8. **1D** Inbound message journaling — full crash recovery
9. **3B** Per-session client pool — isolation under concurrent load

---

## Consequential Decisions

1. **Tape format change (1A/1B)**: Adding checksums or length prefixes changes the on-disk format. Need migration strategy for existing tapes. Options: (a) new format only for new entries, old entries read as-is; (b) one-time migration script.

2. **Session eviction policy (1C)**: Evicting a session means losing in-memory agent state. Need to decide: flush tape + re-bootstrap on next message, or keep tape warm in a LRU cache?

3. **Tool timeout defaults (2B)**: 60s default may be too short for LLM-calling tools or large file operations. Need per-tool override mechanism.

4. **Bounded channel capacity (3A)**: Too small = backpressure drops messages from fast channels (Telegram group floods). Too large = defeats the purpose. 256 is a starting point.

5. **Client pool keying (3B)**: Per-session isolation increases connection count. For providers with connection limits, this could cause 429s. May need per-provider policy.
