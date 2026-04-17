# 2026-04-17 · `provider:model` parsing collides with Ollama tag syntax

## Context
After unifying `agent-infer / ollama / vllm / lmstudio / llama.cpp` under a
single `local` provider, login auto-detected an Ollama mock serving
`llama3.2:3b`. The login flow gated prefixing on `hit.model_id.contains(':')`
and stored the model id verbatim. `eli run "ping"` then took ~2s and
returned an error — the mock server saw `GET /v1/models` but no
`POST /v1/chat/completions`.

## Root Cause
The runtime parses model ids via `split_once(':')` in
`nexil/src/core/execution.rs:67`, treating the prefix as the provider name.
Ollama uses `name:tag` for models (`llama3.2:3b`, `qwen2.5:7b`) — that colon
is part of the *model name*, not a provider boundary. Stored value
`"llama3.2:3b"` parsed as `provider="llama3.2"`, `model="3b"`. Lookup of
unknown provider `llama3.2` short-circuited before any HTTP request was made.

The same broken heuristic existed in two places:
`builtin/cli/login.rs::login_local` and `builtin/cli/model.rs::model_switch`.

## Fix
Added `is_known_provider(name)` in `nexil/src/core/provider_policies.rs` —
returns true only when the name normalizes to one of the canonical built-in
providers. Both call sites now switch on
`split_once(':').filter(|(p,_)| is_known_provider(p))` instead of bare
`contains(':')`. Unknown prefixes are treated as part of the model name and
get the active provider prefixed.

## Rule
**`contains(':')` is never a valid heuristic for "already has a provider
prefix" once a backend admits colons in model names.** When a serialization
format encodes a discriminator with a delimiter, validate that the prefix
*is* a known discriminator value — don't infer it from "delimiter present".
The cost of "is this a known provider?" is a single match arm; the cost of
guessing wrong is a silently broken backend.
