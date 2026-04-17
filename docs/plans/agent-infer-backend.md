# Plan: agent-infer as a first-class Eli backend

**Owner:** ckl · **Date:** 2026-04-17 · **Status:** proposed

## Goal

One-command onboarding for the local agent-infer server (`/Users/bytedance/code/agent-infer`): `eli login agent-infer` auto-detects the running instance, picks up its served model, saves a profile, sets it active. `eli chat` just works afterwards. Also unlock the same path for any OpenAI-compatible local server (Ollama, vLLM, LM Studio) as a side benefit.

## Facts (no decisions needed)

- agent-infer serves OpenAI-compatible HTTP: `/v1/chat/completions`, `/v1/completions`, `/v1/models`, `/v1/responses`, `/v1/stats`, `/metrics`.
- Default bind: `127.0.0.1:8000`. No auth on localhost.
- `GET /v1/models` lists loaded model(s) → use as model-name source of truth.
- Eli's `ApiFormat::Responses` (OpenAI Chat Completions format) already covers the protocol → **zero transport work**.

## The one real trade-off

Profile schema currently = `{ provider, model }`. No per-profile `api_base`. Two options:

- **(A) Add `api_base: Option<String>` to `Profile`** — chosen. Also unblocks any other OpenAI-compat local server. Schema change is additive, old configs keep working (serde default). ~20 lines of config code.
- **(B) Keep schema, rely on `ELI_AGENT_INFER_API_BASE` env var.** Rejected — breaks as soon as user runs more than one local server, and doesn't generalize.

## Architecture delta

| Layer | Change | File(s) |
|---|---|---|
| nexil provider registry | Register `agent-infer` with base `http://127.0.0.1:8000/v1`, `ApiFormat::Responses` | `crates/nexil/src/core/provider_registry.rs` |
| nexil provider policies | Alias (`agent-infer` → canonical), no hardcoded default model (read from `/v1/models` at login time) | `crates/nexil/src/core/provider_policies.rs` |
| nexil client headers | Skip `Authorization` when api_key is empty (local server has no auth) — verify existing behavior, add guard if needed | `crates/nexil/src/core/client_registry.rs` |
| eli config schema | `Profile.api_base: Option<String>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` | `crates/eli/src/builtin/config.rs` |
| eli model resolution | When profile has `api_base`, it overrides the registry's base URL for that request | `crates/eli/src/builtin/settings.rs` |
| eli login command | New branch `eli login agent-infer` → detect, probe `/v1/models`, save profile with discovered model + api_base | `crates/eli/src/cli/login.rs` |
| eli detect module | New `cli/detect.rs`: HTTP GET `${base}/v1/models` with 500ms timeout, parse first model id | new file |
| eli profile picker | `eli use` with no arg → `dialoguer::Select` over configured profiles, star the active one | `crates/eli/src/cli/profile.rs` |
| Tests | Integration test stubs the `/v1/models` endpoint with `wiremock`, asserts profile saved | `crates/eli/tests/` |

## Detection strategy

1. Candidate order: `$AGENT_INFER_URL` → `http://127.0.0.1:8000` → `http://127.0.0.1:8012` (Metal script's alt port).
2. Each candidate: `GET /v1/models` with 500ms connect + 1s total timeout. Parse `{"data":[{"id":...}]}`.
3. First success wins; report the URL + model id back to the user, ask for confirmation before save (one `y/N` prompt, default N to match other `eli login` flows).
4. If none respond: print the exact command to start agent-infer (`./scripts/start_metal_serve.sh` on Mac, docker line on Linux) and exit non-zero.

## UX flows

**First-time login** (agent-infer already running):
```
$ eli login agent-infer
🔍 Probing agent-infer... found http://127.0.0.1:8000
📦 Served model: mlx-community/Qwen3-0.6B-4bit
💾 Save as profile 'agent-infer' and set active? [y/N] y
✅ Done. Try: eli chat
```

**Switching profiles** (new picker):
```
$ eli use
? Active profile:
❯ agent-infer  (local · mlx-community/Qwen3-0.6B-4bit)  ← active
  openai       (gpt-5.4-mini)
  anthropic    (claude-sonnet-4-6)
```

**Detection failure**:
```
$ eli login agent-infer
🔍 Probing agent-infer... no server responded
   Candidates tried: http://127.0.0.1:8000, http://127.0.0.1:8012
   Start it with:
     cd ~/code/agent-infer && ./scripts/start_metal_serve.sh
```

## Work breakdown (sequential)

1. **Schema extension** — add `api_base` to `Profile`, default-skip in serde, thread through `resolve_*` helpers. Tests: round-trip a config with and without the field.
2. **Provider registration** — add `agent-infer` to registry + policies. Unit test: registry returns the right base/format.
3. **Detect module** — plain `reqwest` GET with timeout. Unit test: mock `/v1/models`, assert parsed model id.
4. **Login branch** — wire detect → prompt → save. Integration test end-to-end with wiremock.
5. **Profile picker** — `eli use` no-arg → `dialoguer::Select`. Unit test: non-interactive fallback when stdin is not a TTY (print list, exit 0).
6. **Docs** — `README` short section + `docs/providers/agent-infer.md`.

Each step is one commit. Each commit runs `cargo fmt --all -- --check` + `cargo clippy --workspace -- -D warnings` + `cargo test --workspace` before push (per `feedback_fix_ci_before_push`).

## Out of scope

- Auto-starting agent-infer if it's not running. User launches it explicitly; Eli only consumes.
- Multi-endpoint load balancing.
- Streaming-specific handling (`/v1/chat/completions` already streams fine via existing code path).
- Responses-API–specific optimizations — `ApiFormat::Responses` route covers it.

## Risks

- **Header handling:** if `client_registry.rs` always sends `Authorization: Bearer ...` even with empty key, agent-infer may 401. Needs a one-line check before coding — guard with a unit probe in step 2.
- **Dialoguer as new dep:** already used? If not, adds ~1 dep. Acceptable (small, well-maintained). Worst case: fall back to `std::io` prompt loop.
- **Model hot-swap:** if agent-infer reloads with a different model, the saved profile model string is stale. Mitigation: `eli use agent-infer` re-queries `/v1/models` and updates the in-memory model string for the session if it differs; log a warning. Persisted profile update only on explicit `eli login agent-infer` re-run.
