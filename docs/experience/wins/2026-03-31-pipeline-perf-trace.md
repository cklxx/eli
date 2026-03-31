# 2026-03-31 · Turn pipeline performance trace & optimization

## Context

Conducted full-chain performance analysis of the turn pipeline. Added wall-clock
timing instrumentation to every stage in `framework.rs::process_inbound` and ran
end-to-end traces with release builds.

## What Worked

### Optimizations applied (commit 31bfa55)

| Change | Before | After |
|--------|--------|-------|
| `preview_json` | Full `to_string()` then truncate | `BoundedWriter` stops at 1000 chars |
| SSE buffer | `to_owned()` per line (~100 allocs/stream) | cursor scan + single `drain` |
| Hook runtime snapshot | `RwLock.read().clone()` (Vec alloc + N Arc bumps) | `ArcSwap.load_full()` (1 Arc bump, 1μs) |
| `wrap_tools_fn` | Clone plugins Vec per turn | Capture `Arc<HookRuntime>` |
| Tool map | `HashMap<String, &Tool>` (clone names) | `HashMap<&str, &Tool>` (zero-copy) |
| `plugin_name()` | `.to_owned()` on every hook call (~60/turn) | `&str` on happy path, `.to_owned()` only on error |

### End-to-end timing (release, 3-run median)

```
snapshot:           1 μs   (~0%)     — ArcSwap load
resolve_session:    9 μs   (~0%)
load_state:       6.5 ms   (0.3%)   — tape_has_entries() I/O
build_prompt:      19 μs   (~0%)
sys_prompt:       8.1 ms   (0.4%)   — SOUL.md + SYSTEM.md + skill discovery I/O
run_model:       ~2.6 s    (96%+)   — LLM network round-trip
save_state:       2.4 ms   (0.1%)   — tape write
render_outbound:  107 μs   (~0%)
dispatch:           7 μs   (~0%)
─────────────────────────────────────
framework overhead: ~17ms / turn (excluding LLM)
```

### Remaining optimization targets

1. **`load_state` 6.5ms** — `tape_has_entries()` queries tape store to detect new
   sessions. Could cache result or check lazily.
2. **`sys_prompt` 8.1ms** — synchronous file reads (SOUL.md, SYSTEM.md) plus skill
   directory walk with YAML parsing. Could cache per workspace with fswatch invalidation.

## Rule

- Framework overhead is sub-20ms; LLM latency dominates at 96%+. Future optimizations
  should target `load_state` (tape cache) and `sys_prompt` (file cache) if sub-10ms
  framework budget matters.
- Always measure before optimizing — the actual bottleneck distribution was different
  from what code inspection suggested (e.g. hook snapshot was already fast after ArcSwap).
