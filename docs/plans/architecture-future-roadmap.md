# Eli Architecture: Future Problems & Optimization Roadmap

## Context

ckl wants a comprehensive analysis of Eli's architecture to identify future problems and optimization opportunities. This builds on the existing review (`docs/ARCHITECTURE_REVIEW_2026-04-06.md`) and the known TODOS.md items, focusing on **new findings** not already documented.

**Existing review status:**
- C2 (TapeEntryKind string→enum): **Fixed**
- M1 (lock poisoning pattern): **Not fixed** — 47 `.expect("lock poisoned")` still in codebase
- C1 (Envelope=Value), C3-C5, M2-M8: **Open**
- TODOS P1s (segmented prompt, tool chain, classification router): **Open**

This plan adds **18 new findings** across 4 categories, with a phased roadmap.

---

## Category 1: Safety & Reliability (crash/data-loss risks)

### 1.1 Lock Poisoning Epidemic — P0 (NEW)

**Problem:** 47 `.expect("lock poisoned")` across 13 files. Any panic while holding a lock permanently poisons it. All subsequent access panics, cascading process-wide failure.

**Hot spots:**
- `tools.rs:19,28,35,39` — global REGISTRY + MODEL_TOOLS_CACHE
- `control_plane.rs` — 6 occurrences (INBOUND_INJECTOR, turn context)
- `builtin/mod.rs` — 9 occurrences (agents, last_active, channels)
- `tape/store.rs` — 6 occurrences (InMemoryTapeStore)
- `tool_middleware.rs` — 4 occurrences (CircuitBreaker, MetricsCollector)

**Impact:** In gateway mode (long-running), one tool handler panic permanently breaks the process. No recovery, no graceful degradation.

**Fix:** Replace all with `parking_lot::Mutex` (never poisons) or `.unwrap_or_else(|e| e.into_inner())` for recovery. Note: `parking_lot` is a transitive dep via tokio/crossterm (not arc-swap as previously claimed), so adding it as direct dep has no binary size impact.

**Effort:** S | **Deps:** None | **Existing overlap:** M1 in review doc (noted but not fixed)

### 1.2 Sidecar Subprocess Cleanup on Abnormal Termination — P2 (CORRECTED)

**Problem:** Normal gateway shutdown already handles SIGTERM + `child.wait()` (gateway.rs:432, 469, 565). But abnormal termination (panic, OOM, SIGKILL) and startup-failure paths do not clean up the sidecar process. The child handle is not stored in a location reachable by a global signal handler.

**Impact:** Orphan Node.js processes after crashes; port 3101 conflicts on restart.

**Fix:** Register a `ctrlc`/signal handler that kills the sidecar process group on abnormal exit. Ensure startup failures (sidecar health check timeout) also kill the spawned process.

**Effort:** S | **Deps:** None

### 1.3 Hook Panics Lose Backtrace — P2 (NEW)

**Problem:** `hooks.rs:51-73` — `call_notify_all!` and `call_sync_all!` macros use `catch_unwind` but only log `"hook.X panicked"`. No panic message, no backtrace, no source location.

**Impact:** Debugging production hook failures requires reproducing locally — no diagnostic info in logs.

**Fix:** Extract panic payload via `Any::downcast_ref::<String>()`. Log the message. Capture `std::backtrace::Backtrace` when `ELI_TRACE` is set.

**Effort:** S | **Deps:** None

### 1.4 Context Truncation Telemetry — P2 (CORRECTED)

**Problem:** `nexil/tape/context.rs:109-122` — `apply_context_budget()` drops conversation history via `aggressive_trim()`. A `TRIM_NOTICE` is already injected into surviving messages (context.rs:182), so the model IS notified. However, there is no `tracing::warn!` for operators, no telemetry event, and no metric tracking how often trimming occurs.

**Impact:** Operators of gateway deployments have no visibility into how often context is being trimmed.

**Fix:** Add `tracing::warn!` with count of dropped messages. Emit a tape event for telemetry. Consider making TRIM_NOTICE a system message instead of assistant content.

**Effort:** S | **Deps:** None

---

## Category 2: Performance & Scalability

### 2.1 Token Counting is Model-Unaware — P1 (NEW)

**Problem:** `nexil/tape/context.rs:96-101` — hardcoded `MAX_TOTAL_CONTEXT_CHARS = 400_000` (ASCII) and `200_000` (CJK). No model-specific context window awareness.

- Claude 3.5: 200K tokens ≈ 800K chars → unnecessarily trimmed at 400K
- GPT-4o-mini: 128K tokens ≈ 512K chars → 400K chars ≈ 100K tokens, close to overflow
- Gemini: 1M tokens → massively over-trimmed

**Fix:** The model registry already exists — `infer_context_window()` in `model_specs.rs:120` with 40+ models, wired to `settings.rs:239`. The gap is plumbing: nexil's `apply_context_budget()` doesn't receive the model's context window. Pass `context_window: usize` parameter from eli through to nexil's budget function. Also: current budgeter only counts string `content` (context.rs:163), so block-based multimodal content is undercounted. Full fix needs request-level budget, not just tape trimming.

**Effort:** S-M (plumbing, not new registry) | **Deps:** None | **Existing overlap:** Adjacent to P1 "Segmented Prompt Builder" but distinct (tape context vs prompt sections)

### 2.2 SSE Buffer Reallocation — P2 (NEW)

**Problem:** Two allocation inefficiencies in streaming:
1. `response_parser.rs` — `push_str(&String::from_utf8_lossy(&chunk))` allocates new String on every chunk even when valid UTF-8
2. `clients/parsing/sse.rs:57` — `buffer = buffer[start..].to_owned()` copies remaining buffer on every drain

**Impact:** For 100KB streaming response with 64-byte chunks: ~1600 unnecessary allocations. Measurable in flamegraphs for extended-thinking responses.

**Fix:** (1) Check `std::str::from_utf8()` first, only lossy on error. (2) Use `self.buffer.replace_range(..start, "")` instead of `to_owned()` (`buffer.drain(..start)` does not work on `String`). **Correctness issue:** per-chunk `from_utf8_lossy` can corrupt multibyte UTF-8 characters split across chunk boundaries. Must accumulate raw bytes and decode only on complete codepoints.

**Effort:** S-M (correctness fix adds complexity) | **Deps:** None

### 2.3 Tool Registry Mutex on Hot Path — P2 (NEW)

**Problem:** `tools.rs:9-14` — `REGISTRY` and `MODEL_TOOLS_CACHE` are both `LazyLock<Mutex<...>>`. Every turn calls `model_tools_cached()` which locks mutex + clones `Vec<Tool>`.

**Impact:** Under concurrent gateway load, lock contention on every turn. The clone allocates all tool schemas per turn.

**Fix:** Root cause: `populate_model_tools_cache()` is never called after `register_builtin_tools()`, so the cache is always empty and `model_tools_cached()` falls back to locking REGISTRY every turn. Step 1: wire the cache call in `BuiltinImpl::new()` after registration. Step 2: replace `Mutex<Vec<Tool>>` with `OnceLock<Vec<Tool>>` (note: `OnceLock` breaks `register_tool()` public API for dynamic plugins — use `ArcSwap` if hot-reload needed).

**Effort:** S | **Deps:** None | **Existing overlap:** M3 in review doc (dual-write anti-pattern)

### 2.4 Tool Loop Unbounded Context Growth — P1 (ELEVATED from P2)

**Problem:** `nexil/llm/tool_loop.rs:157` — 250-iteration hardcoded cap. No awareness of accumulated `in_memory_msgs` size. A tool chain producing 50KB/iteration can grow context to 12.5MB before hitting the iteration limit.

**Impact:** Context window overflow errors mid-tool-loop, triggering fallback model or hard error. The 250 cap is a proxy for the real constraint (token budget).

**Fix:** Track accumulated message size in the loop. Break with clear error when approaching model limit. Make iteration cap configurable via `ChatRequest`.

**Effort:** M | **Deps:** 2.1 (model-aware limits)

### 2.5 Tape Entries Double-Cloned on Read — P3 (NEW)

**Problem:** `nexil/tape/store.rs:226-232` — `InMemoryTapeStore::read()` clones every entry via `entries.iter().map(|e| e.copy())`. Then `fetch_all_in_memory` (line 173) does `entries[start_index..].to_vec()`. Two full clones per read.

**Impact:** For long conversations (limit 10K entries at line 279), significant allocation pressure per turn.

**Fix:** Return `Arc<Vec<TapeEntry>>` with copy-on-write semantics. Or use `im::Vector` for structural sharing.

**Effort:** M | **Deps:** None

---

## Category 3: Developer Experience & Extensibility

### 3.1 State Merge Precedence Inconsistency — P2 (NEW)

**Problem:** `framework.rs:357-360` — `build_state` iterates hook states in reverse, then uses `entry(k).or_insert(v)`. Combined with `call_load_state` iterating plugins in forward order, the result is: **first-registered plugin wins**. This contradicts the "last-registered wins" convention used by all other first-result hooks.

**Impact:** Plugin authors cannot predict state precedence without reading framework internals.

**Fix:** Document clearly. Consider reversing to match convention. Add test with two conflicting plugins.

**Effort:** S | **Deps:** None

### 3.2 No Rust Integration Tests for Full Pipeline — P2 (NEW)

**Problem:** Rust crate has ~100 unit tests but no integration test for: inbound message → ChannelManager → process_inbound → hook chain → outbound dispatch. Full pipeline testing requires Python integration tests against a running gateway.

**Impact:** Regressions in message routing, debounce, or dispatch only caught by slow, expensive Python tests.

**Fix:** Add `tests/integration.rs` with mock channels + mock hooks + `EliFramework` wired together.

**Effort:** M | **Deps:** None

### 3.3 HookError Uses String Hook Point — P3 (NEW)

**Problem:** `hooks.rs:26` — `hook_point: &'static str` instead of an enum. Can't pattern-match, no compile-time validation.

**Fix:** `HookPoint` enum derived from the `HOOK_NAMES` array (hooks.rs:314-328).

**Effort:** S | **Deps:** None

### 3.4 Config Path Hardcoded + Duplicated — P3 (NEW)

**Problem:** `eli_home()` defined in both `builtin/config.rs:9-17` and `main.rs:25-31`. No `--config-dir` CLI flag.

**Fix:** Consolidate to one location. Add CLI flag.

**Effort:** S | **Deps:** None

### 3.5 .env Loaded 4 Times — P3 (NEW)

**Problem:** `dotenvy::dotenv()` called in `gateway.rs:391`, `tape.rs:5`, `decisions.rs:10`, `settings.rs:223`. Redundant disk reads.

**Fix:** Call once in `main()`. Remove other call sites.

**Effort:** S | **Deps:** None

---

## Category 4: Feature Gaps (vs modern agent frameworks)

### 4.1 No Streaming Cancellation — P2 (NEW)

**Problem:** Once SSE streaming starts in `response_parser.rs`, there's no way to abort mid-stream. `CancellationToken` only checked at tool-loop iteration boundaries (`tool_loop.rs:166`), not during streaming.

**Impact:** `/stop` command doesn't actually stop the current LLM call — waits for full response.

**Fix:** Pass `CancellationToken` into streaming layer. Use `tokio::select!` between `stream.next()` and `cancellation.cancelled()`.

**Effort:** M | **Deps:** None

### 4.2 No Runtime Provider Registration — P2 (NEW)

**Problem:** `provider_runtime.rs:33-54` — transport selection hardcoded by provider name. Adding custom provider requires source modification.

**Impact:** Users with local models, internal APIs, or niche providers must fork nexil.

**Fix:** `ProviderRegistry` with `register(name, config)` method. Existing providers become default entries.

**Effort:** M | **Deps:** None

### ~~4.3 Self-Learning Loop~~ — REMOVED

Removed per review. Was introduced as a roadmap item then immediately excluded from scope. Either commit to this as a separate strategic bet with success criteria, or don't mention it. See Hermes Agent's procedural memory for reference if/when this becomes a priority.

### 4.4 OAuth Token Auto-Refresh — P3 (NEW)

**Problem:** `auth/openai_codex.rs` — `refresh_openai_codex_oauth_tokens()` must be called manually. `ClientRegistry` caches stale tokens.

**Impact:** Long-running gateway sessions fail after token expiry with 401.

**Fix:** Token-refresh middleware. On 401, auto-refresh + invalidate cached client + retry.

**Effort:** M | **Deps:** None

---

## Dependency Graph

```
Phase 1 (Safety)          Phase 2 (Performance)       Phase 3 (DX/Extensibility)
─────────────────         ──────────────────────       ─────────────────────────
1.1 Lock Poisoning ──┐
1.2 Sidecar Reaping  │    2.1 Token Counting ──┐       3.1 State Merge Docs
1.3 Panic Backtrace  │    2.2 SSE Buffers      │       3.2 Integration Tests
1.4 Silent Truncation│    2.3 Registry ArcSwap─┼──→ 4.3 Hot-Reload (if done)
3.5 Single .env Load │    2.4 Tool Loop Growth←┘       3.3 HookPoint Enum
3.4 Config Consolidate    2.5 Tape Cloning             4.1 Stream Cancellation
                                                        4.2 Provider Registration
                                                        4.4 OAuth Auto-Refresh
```

---

## Phased Roadmap

### Phase 1: Safety Foundations (1-2 days CC time)
All S-effort, zero dependencies, immediate impact on reliability.

| # | Item | Effort | Impact |
|---|------|--------|--------|
| 1 | 1.1 Lock poisoning → parking_lot | S | Prevents cascading process failure |
| 2 | 1.2 Sidecar abnormal termination cleanup | S | No more orphan processes on crash |
| 3 | 1.4 Truncation telemetry (TRIM_NOTICE exists) | S | Operators see trim frequency |
| 4 | 1.3 Panic backtrace in hooks | S | Debuggable hook failures |
| 5 | 3.5 Single .env load | S | Trivial cleanup |
| 6 | 3.4 Config path consolidation | S | Remove duplication |

### Phase 2: Performance (2-3 days CC time)
Addresses scaling issues. Unlocks Phase 3 items.

| # | Item | Effort | Impact |
|---|------|--------|--------|
| 1 | 2.1 Plumb context_window to nexil budget | S-M | Correct context management per model |
| 2 | 2.3 Tool registry ArcSwap | S | Lock-free hot path |
| 3 | 2.2 SSE buffer optimization | S | Fewer allocations in streaming |
| 4 | 2.4 Tool loop context budget (P1!) | M | Prevents mid-loop overflow — 375MB peak at 10x |
| 5 | 2.5 Tape entry COW | M | Less allocation pressure |

### Phase 3: DX & Extensibility (3-5 days CC time)
Improves development velocity and opens up extension points.

| # | Item | Effort | Impact |
|---|------|--------|--------|
| 1 | 4.1 Streaming cancellation | M | /stop actually stops |
| 2 | 4.2 Runtime provider registration | M | Custom providers without fork |
| 3 | 3.2 Rust integration tests | M | Faster regression detection |
| 4 | 3.1 State merge precedence fix | S | Correct plugin semantics |
| 5 | 4.4 OAuth auto-refresh | M | Gateway stays alive |

### Phase 4: Existing Review Items (from ARCHITECTURE_REVIEW_2026-04-06)
These are already documented. Cross-reference for completeness.

| Item | Status | New overlap |
|------|--------|-------------|
| C1: Envelope=Value → typed struct | Open | Enhances 3.1 |
| C3: Outbound construction dedup | Open | — |
| C4: 238 unwrap() in nexil | Open | — |
| C5: God functions (run_chat, run_tools) | Open | — |
| M2: 4 message-building paths | Open | — |
| M4: Agent/subagent init duplication | Open | — |
| M6: Excessive cloning in retry loops | Open | — |

---

## Verification

After implementing each phase:
```bash
cargo clippy --workspace -- -D warnings    # lint clean
cargo test --workspace                      # all tests pass
cargo build --release                       # release builds
```

For Phase 2 items, add benchmark tests to verify improvements.
For Phase 3 integration tests, verify the full channel→framework→output path.

---

## What's NOT in Scope

- P3 Python/JS bindings (XL effort, TODOS.md)
- P1 Gateway classification router (TODOS.md, separate feature)
- Full Envelope→struct migration (L, already in existing review as C1)
- Gateway operational controls: cost ceilings, audit retention, observability metrics (deferred to TODOS)
- Full request budget design including multimodal content counting (deferred to TODOS)

---

## Decision Audit Trail

| # | Phase | Decision | Classification | Principle | Rationale | Rejected |
|---|-------|----------|---------------|-----------|-----------|----------|
| 1 | CEO | Mode: SELECTIVE EXPANSION | Mechanical | P3 pragmatic | Plan is analysis, not feature | — |
| 2 | CEO | Accept premises 1-3 | Mechanical | P6 action | Findings are code-verified | P4: challenge premise re scale |
| 3 | CEO | Challenge premise 4 (fix before build) | Taste | P1+P6 | Cheap fixes = insurance, but shouldn't delay features | — |
| 4 | CEO | Keep all 18 findings | Mechanical | P1 completeness | Each well-scoped with file refs | — |
| 5 | CEO | Defer 4.3 self-learning | Mechanical | P3 pragmatic | XL effort, outside blast radius | — |
| 6 | ENG | Fix parking_lot attribution | Mechanical | P5 explicit | Claim was wrong (arc-swap doesn't bring it) | — |
| 7 | ENG | Downgrade 1.2 sidecar | Taste | P5 explicit | Normal shutdown already handled; gap is abnormal only | — |
| 8 | ENG | Downgrade 1.4 truncation | Taste | P5 explicit | TRIM_NOTICE already exists; gap is telemetry only | — |
| 9 | ENG | Elevate 2.4 to P1 | Mechanical | P1 completeness | 10x concurrent = 375MB peak, real risk | — |
| 10 | ENG | Fix 2.1 approach | Taste | P5 explicit | Plumbing gap, not new registry. Crate boundary design needed | — |
| 11 | ENG | Fix 2.2 drain suggestion | Mechanical | P5 explicit | buffer.drain doesn't work on String | — |
| 12 | ENG | Add 2.2 correctness issue | Mechanical | P1 completeness | Lossy UTF-8 can corrupt multibyte chars | — |
| 13 | ENG | Wire 2.3 cache call first | Mechanical | P3 pragmatic | populate_model_tools_cache() never called — fix that first | — |

---

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `/plan-ceo-review` | Scope & strategy | 1 | issues_found | 5 (Claude) + 8 (Codex) |
| Codex Review | `/codex review` | Independent 2nd opinion | 2 | issues_found | CEO + Eng voices |
| Eng Review | `/plan-eng-review` | Architecture & tests (required) | 1 | issues_found | 8 (Claude) + 8 (Codex) |
| Design Review | `/plan-design-review` | UI/UX gaps | 0 | skipped | No UI scope |

**VERDICT:** REVIEWS COMPLETE — plan needs corrections before implementation.
