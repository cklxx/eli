# Conduit Provider Adapter Refactor

This design note captures a Claude-generated target architecture and execution
plan for refactoring `conduit` so provider behavior has a single choke point
and request/response shaping lives in provider-family adapters.

## Target Architecture

- **Central `ProviderRegistry`** — a static or Arc-wrapped map from provider ID → `Box<dyn ProviderFactory>`; all provider resolution flows through here, no scattered `match` arms on model strings.
- **`ProviderFactory` trait** — one method: `fn build(&self, config: &ProviderConfig) -> Box<dyn ProviderClient>`; factories own credential validation, base-URL resolution, and feature-flag gating before a client is ever constructed.
- **`ProviderClient` trait** — the single choke point: `fn execute(&self, req: NormalizedRequest) -> Stream<NormalizedResponse>`; nothing outside this trait touches provider-specific wire formats.
- **Provider-family adapters** (`OpenAIAdapter`, `AnthropicAdapter`, `GeminiAdapter`) — each adapter implements request shaping (`NormalizedRequest → wire JSON`) and response shaping (`wire JSON → NormalizedResponse`) as private methods; adapter = the only file you touch to fix a provider's quirks.
- **`NormalizedRequest` / `NormalizedResponse` types** — canonical internal envelope that every adapter speaks; tool calls, streaming deltas, stop reasons, and usage stats all have first-class fields rather than being buried in `serde_json::Value`.
- **Message normalization at intake** — one pass at the framework boundary converts raw channel messages into `NormalizedRequest`; adapters never see unstructured input, eliminating ad-hoc format fixes scattered across callers.
- **Capability flags on `ProviderFactory`** — `fn supports(&self, cap: Capability) -> bool` (streaming, vision, tool use, JSON mode); the runtime queries flags before routing, rather than discovering missing features at runtime via error codes.
- **No provider logic in `LLM` / `LLMBuilder`** — builder resolves config and selects a factory; `LLM` is a thin executor that calls `client.execute()` and drives the stream; all behavioral complexity lives inside adapters, keeping the core auditable and stable.

## Proposed Rust Modules

- **`crates/nexil/src/providers/mod.rs`** — New top-level module for all provider-family adapters; re-exports family submodules.
- **`crates/nexil/src/providers/anthropic.rs`** — Anthropic-family adapter: request/response mapping, streaming SSE parsing, model-name normalization. Extracted from `execution.rs`.
- **`crates/nexil/src/providers/openai.rs`** — OpenAI-compatible adapter (also covers Azure, Groq, Together). Chat completion serialization, tool-call delta stitching. Extracted from `execution.rs`.
- **`crates/nexil/src/providers/gemini.rs`** — Google Gemini adapter: `generateContent` shape, safety-rating passthrough. New file.
- **`crates/nexil/src/core/api_format.rs`** — Keep as the canonical wire-type definitions and format enums. Remove dispatch logic; adapters own translation.
- **`crates/nexil/src/core/provider_runtime.rs`** — Keep and promote into the provider registry + dispatch layer: maps model strings to adapter trait objects and holds retry/timeout policy. `execution.rs` delegates here.
- **`crates/nexil/src/core/execution.rs`** — Must shed provider-specific HTTP building, per-family streaming state machines, response deserialization branches, and model-name matching arms. Retains only the turn lifecycle: build request, dispatch via `provider_runtime`, collect stream, return normalized response.
- **`crates/nexil/src/core/stream.rs`** — New file for streaming primitives (`StreamEvent`, `DeltaAccumulator`, SSE frame parser) currently buried in `execution.rs`.
- **`crates/nexil/src/adapter.rs`** — Defines the `ProviderAdapter` trait (`fn build_request`, `fn parse_event`, `fn finish`). All provider files implement this.
- **`crates/nexil/src/llm.rs`** — Keep but simplify. `LLMBuilder` wires the chosen adapter into `provider_runtime`; remove inline provider conditionals that leaked in here.
- **`crates/eli/src/builtin/agent.rs`** — Keep with no structural dependency on conduit's internals; it should consume conduit's public API only.
- **`crates/eli/src/builtin/settings.rs`** — Keep and extend with a provider-family override so users can choose an adapter independently of the model string when needed.
- **`crates/nexil/src/providers/tests/`** — One integration-test file per adapter (`anthropic_test.rs`, `openai_test.rs`) with recorded HTTP fixtures.

## Execution Plan

### Phase 1 — Audit & Map

Read `execution.rs`, `llm.rs`, `provider_runtime.rs`, and `api_format.rs` in
full. Enumerate every provider-specific branch and ad-hoc format conversion.
Produce a flat list of `(provider, concern, line range)`. Verification focus:
the list covers every callsite that touches a provider name or transport
format.

### Phase 2 — Define the Choke Point

Design a single `ProviderRuntime`/`ProviderAdapter` seam with the minimum
surface needed to replace all branches: `build_request`, `parse_response`, and
streaming callbacks such as `stream_chunk` or `parse_event`. Confirm the trait
is object-safe and decide up front how async streaming crosses the boundary.
Verification focus: the seam is small enough that every provider family can
implement it without leaking transport-specific details back into
`execution.rs`.

### Phase 3 — Extract One Adapter

Pick the simplest provider family, likely OpenAI-compatible. Move its request
and response shaping into `providers/openai.rs` behind the new seam. Keep other
providers on the old path temporarily. Verification focus: build stays green,
OpenAI behavior is unchanged, and no other provider is touched.

### Phase 4 — Port Remaining Adapters

Extract Anthropic next, then additional families one at a time. When the last
provider is migrated, delete the old branching code from `execution.rs`.
Verification focus: `execution.rs` no longer contains provider-name strings and
the workspace test suite still passes.

### Phase 5 — Collapse `execution.rs`

Reduce `execution.rs` to orchestration only: resolve the adapter, dispatch the
request, drive the stream, and return normalized output. If the file becomes
small enough, rename it to `dispatch.rs` to reflect its real role.
Verification focus: no business logic remains in dispatch.

### Phase 6 — Harden the Seam

Add unit tests that exercise `build_request` and `parse_response` for each
adapter with canned JSON fixtures. Add an exhaustiveness check so introducing a
new provider family requires an adapter implementation. Verification focus:
every adapter has happy-path and error-path coverage.

### Phase 7 — Reflect & Document

Update architecture docs to describe the adapter pattern and remove stale
references to `execution.rs` as the runtime center of gravity. Verification
focus: docs and code agree on the new module boundaries.

## Invariants, Risks, and Tests

### Invariants

- Provider identity is resolved in exactly one place; no provider strings leak into dispatch or turn orchestration.
- Streaming and non-streaming use the same adapter seam, not parallel hierarchies.
- `eli` passes configuration only; runtime behavior is decided inside `conduit`.

### Risks

- **Trait object safety**: async streaming across adapters needs a deliberate design (`async_trait`, boxed futures, or another stable boundary).
- **Hidden format coupling**: some “OpenAI-compatible” providers diverge at the edges; family grouping must be validated before assuming uniform behavior.
- **Coverage gap**: if tests rely on live credentials, fixture-based adapter tests become the primary feedback loop.
- **Public API drift**: changing `LLMBuilder` internals is cheap; changing its public surface should be treated as a separate compatibility decision.

### Tests

- Per-adapter request-building tests that assert provider field names and payload shape.
- Per-adapter response-parsing tests with canned JSON blobs.
- Dispatch tests for unknown providers and family overrides.
- Streaming tests that verify chunk boundaries do not lose text, tool calls, or usage data.
