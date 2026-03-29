# 2026-03-29 · SSE stream error after 8 minutes — server-side termination, silent retries

## Context

Fifth occurrence of `[temporary] SSE stream error: error decoding response body: request or response body error`.
Provider: `openai:gpt-5.4` via `chatgpt.com/backend-api/codex`. Elapsed: `480079ms` (8 minutes).
Previous fixes (stale pool eviction, build_candidate inside loop) were deployed and running. Still occurred.

## Root Cause

**Layer 1 — forced streaming**: When API key starts with `eyJ` (Codex CLI JWT),
`uses_openai_codex_backend()` returns true → `resolved_api_base()` = `https://chatgpt.com/backend-api/codex`.
`build_responses_body` detects `base.contains("chatgpt.com")` → injects `"stream": true` into body.
`body_forced_stream = !stream(false) && body["stream"](true)` = `true`.
Result: every tool round goes through `collect_sse_response`, even when caller passes `stream: false`.

**Layer 2 — error type**: `reqwest::Kind::Decode(Kind::Body)` = h2 stream interrupted by server
(RST_STREAM or TCP close). Displayed as `"error decoding response body: request or response body error"`.
Our code appends source: `{e}{source(&e)}` → the double-layered message we see.

**Layer 3 — why after 8 minutes**: `chatgpt.com/backend-api/codex` backend has a
per-stream or per-connection time limit (~8 min). After extended streaming, server resets the stream.
NOT a stale pool issue — the connection was actively streaming before failure.

**Layer 4 — why retries don't rescue**: Retries ARE triggered (`ErrorKind::Temporary` →
`should_retry()=true`). But `log_error` is gated behind `verbose > 0` (default 0), so
all retry attempts are completely silent. Evidence: `elapsed_ms=480079` = ~8 min exactly,
consistent with one long attempt + 3 fast-failing retries (server-side issue persists).

## Fix

Added `tracing::warn!` directly in the SSE-error branch of `run_chat` (not behind verbose),
logging `provider`, `model`, `attempt`, `max_attempts`, and `error` whenever an SSE stream
error triggers a retry. This makes retries visible in the console without requiring verbose mode.

## Rule

- `log_error` is verbose-gated — never assume retries are silent failures without checking verbose level.
- When an error has `elapsed_ms` ≈ N minutes and the same error appears with no retry logs, check if retries
  are happening silently (look for `verbose > 0` gates).
- The chatgpt.com Codex backend forces streaming on ALL requests regardless of the `stream` param —
  any request to this backend can produce SSE stream errors including simple tool-call rounds.
- To get `bytes_received` on the failing chunk, set `ELI_TRACE=1` (writes to `~/.eli/logs/eli-trace.log`).
