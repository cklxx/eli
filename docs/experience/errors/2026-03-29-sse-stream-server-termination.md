# 2026-03-29 · SSE stream error — 120s reqwest timeout firing 4×, totaling exactly 480s

## Context

Fifth occurrence of `[temporary] SSE stream error: error decoding response body: request or response body error`.
Provider: `openai:gpt-5.4` via `chatgpt.com/backend-api/codex`. `elapsed_ms=480079`.
Previous fixes (stale pool eviction, build_candidate inside loop) were deployed and running. Still occurred.

## Root Cause

### Layer 1 — forced streaming (why SSE is involved at all)

`api_key.starts_with("eyJ")` (Codex CLI JWT) → `uses_openai_codex_backend()=true`
→ `resolved_api_base()="https://chatgpt.com/backend-api/codex"`
→ `build_responses_body`: `base.contains("chatgpt.com")` injects `"stream": true`
→ `body_forced_stream = !stream(false) && body["stream"](true)` = `true`
→ every tool round goes through `collect_sse_response`, even when caller passes `stream: false`

### Layer 2 — the actual trigger: reqwest 120s timeout

`client_registry.rs:116-117`:
```rust
let is_oauth_token =
    api_key.is_some_and(|k| k.to_ascii_lowercase().starts_with("sk-ant-oat"));
```
Only Anthropic OAuth (`sk-ant-oat`) gets the 600s extension (`timeout_secs.max(600)`).
The Codex JWT (`eyJ`) is NOT matched → gets the **default 120s timeout**.

### Layer 3 — why the error looks like a body error, not an explicit timeout

When the 120s tower timeout fires during h2 body streaming, it doesn't produce `Kind::Request`
("error sending request"). Instead: tower cancels the hyper/h2 future → h2 RST_STREAM →
reqwest wraps as `Kind::Decode(Kind::Body)` → display: "error decoding response body: request or response body error".
`is_timeout()` returns false for this chain (checks `source` one level deep, misses the nested h2 error).

### Layer 4 — the smoking gun: 4 × 120s = 480s

`max_retries.unwrap_or(3)` → 4 total attempts.
No backoff between retries (just `continue`).
Each attempt: same large context → same slow generation → same 120s timeout.

**4 × 120,000ms = 480,000ms. elapsed_ms = 480,079ms. Δ = 79ms (framework overhead).**

### Layer 5 — why retries were invisible

`log_error()` in `error_classify.rs` is gated by `verbose > 0`. Default `verbose=0` →
all retry attempts were completely silent in the console.

### Layer 6 — OpenAI Responses SSE error events not handled

`parse_responses_sse` only looks for `response.completed`. No `"error"` event handling.
The Anthropic error event handler (`"error"` type in `MessagesAccumulator`) is only used
for `TransportKind::Messages`. So even if the Codex backend sent an SSE error event,
it would be silently dropped and fall through to "SSE stream ended without response.completed event".

## Fix

1. **Root fix** (`client_registry.rs`): Added `is_codex_jwt = api_key.starts_with("eyJ")`.
   `timeout_secs = if is_oauth_token || is_codex_jwt { max(timeout, 600) }`.
   Now the Codex JWT path gets 600s, same as Anthropic OAuth.

2. **Visibility fix** (`execution.rs`): Added unconditional `warn!` when SSE stream error
   triggers a retry, showing `provider/model/attempt/max_attempts/error`. Not gated by verbose.

## Rules

- When `elapsed_ms` ≈ N × 120,000: check if the reqwest 120s timeout is firing. Specifically
  check `client_registry::build_client` for the `is_oauth_token` guard — it only covers `sk-ant-oat`.
- `Kind::Decode(Kind::Body)` does NOT mean `is_timeout()=true`, even when the root cause IS a timeout.
  The error path through h2 RST_STREAM bypasses the `is_timeout()` chain.
- `log_error()` is `verbose > 0` gated — retry attempts are invisible by default.
- The chatgpt.com Codex backend forces streaming on ALL requests. Any slow tool round can hit the timeout.
- `parse_responses_sse` has no error event handling — add it if the Codex backend starts sending error events.
