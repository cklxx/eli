# Structure Review — Eli Workspace

## Executive Summary

The eli workspace is a well-organized two-crate Rust project (`conduit` + `eli`) with clean separation between the LLM toolkit layer and the agent framework. The codebase follows Rust conventions well overall: snake_case file naming, logical module grouping, and thoughtful re-exports. At ~22K lines across 80 Rust files, the codebase is compact.

**Top issues** that would hold back 10-star open source quality:

1. **Duplicate `Envelope` type alias** defined in two places — confusing for contributors
2. **Generic module names** (`types.rs`, `utils.rs`, `core/`) that obscure intent
3. **`builtin/` is a 9-module monolith** doing too many unrelated things
4. **Mixed concerns** in `eli::tools.rs` (registry + sidecar state + text truncation)
5. **`conduit::core` shadows `std::core`** — idiomatic Rust avoids this

---

## Workspace Root

```
Cargo.toml          # workspace: conduit + eli, resolver=2, edition 2024 ✓
crates/
  conduit/          # Provider-agnostic LLM toolkit (v0.6.0)
  eli/              # Hook-first agent framework (v0.3.0)
sidecar/            # Node.js TypeScript bridge (not in workspace)
```

**Good**: Two-crate split is clean. `conduit` has zero dependency on `eli`. Workspace dependencies are well-factored.

**Issue**: `conduit`'s Cargo.toml description says _"Core library for the eli AI assistant"_ — this couples it to eli. For a standalone crate, the description should be self-sufficient (e.g. _"Provider-agnostic LLM toolkit with tape-based conversation management"_).

---

## Per-Crate Analysis

### Crate: `conduit`

**Structure:**
```
conduit/src/
├── lib.rs              # Re-exports, 6 top-level modules
├── adapter.rs          # ProviderAdapter trait (11 lines)
├── llm.rs              # LLM + LLMBuilder facade (~800+ lines)
├── auth/
│   ├── mod.rs          # APIKeyResolver + multi_api_key_resolver
│   ├── github_copilot.rs
│   └── openai_codex.rs
├── clients/
│   ├── mod.rs          # ChatClient, EmbeddingClient, TextClient, InternalOps
│   ├── chat.rs         # ChatClient + ToolCallAssembler (1109 lines)
│   ├── embedding.rs
│   ├── internal.rs     # InternalOps (474 lines)
│   ├── text.rs         # TextClient (287 lines)
│   └── parsing/
│       ├── mod.rs      # parser_for_transport factory
│       ├── common.rs
│       ├── completion.rs
│       ├── messages.rs
│       ├── responses.rs
│       └── types.rs    # TransportKind, ToolCallDelta, BaseTransportParser
├── core/
│   ├── mod.rs          # 15 submodules, re-exports
│   ├── anthropic_messages.rs
│   ├── api_format.rs
│   ├── client_registry.rs
│   ├── error_classify.rs
│   ├── errors.rs
│   ├── execution.rs    # LLMCore (1176 lines — largest file)
│   ├── message_norm.rs
│   ├── provider_policies.rs
│   ├── provider_runtime.rs
│   ├── request_adapters.rs
│   ├── request_builder.rs
│   ├── response_parser.rs
│   ├── results.rs
│   └── tool_calls.rs
├── providers/
│   ├── mod.rs          # adapter_for_transport factory
│   ├── anthropic.rs
│   └── openai.rs
├── tape/
│   ├── mod.rs          # Re-exports
│   ├── context.rs      # TapeContext, build_messages, anchor selection
│   ├── entries.rs      # TapeEntry
│   ├── manager.rs      # TapeManager, AsyncTapeManager (901 lines)
│   ├── query.rs        # TapeQuery
│   ├── session.rs      # TapeSession
│   └── store.rs        # TapeStore trait, InMemoryTapeStore, etc.
└── tools/
    ├── mod.rs          # Re-exports
    ├── context.rs      # ToolContext
    ├── executor.rs     # ToolExecutor, ToolCallResponse
    └── schema.rs       # Tool, ToolSet, tool_from_fn, tool_from_schema
```

#### Findings

| # | Issue | Impact | Recommendation |
|---|-------|--------|----------------|
| C1 | **`core/` module name shadows `std::core`** | HIGH | Rename to `engine/` or `runtime/`. The module contains execution logic, request building, error handling — "engine" fits better and avoids the `std::core` shadow. Contributors writing `use conduit::core::...` may get confused with `use core::...` in no-std contexts. |
| C2 | **`core/execution.rs` is 1176 lines** — the largest file, doing request dispatching, retry logic, streaming, and tape recording all in one | MEDIUM | Split into `core/dispatch.rs` (request execution + retries), `core/streaming.rs` (SSE/stream handling), and keep `core/execution.rs` as the `LLMCore` struct and its public API surface. |
| C3 | **`clients/internal.rs`** — name is vague; contains `InternalOps` trait | LOW | Rename to `clients/ops.rs` or `clients/internal_ops.rs` to match the type it contains. |
| C4 | **`clients/parsing/types.rs`** — generic name for what contains `TransportKind`, `ToolCallDelta`, `BaseTransportParser` | MEDIUM | Rename to `clients/parsing/transport.rs` — the module defines transport-level primitives, not generic "types". |
| C5 | **`adapter.rs` at crate root** — 11 lines with just the `ProviderAdapter` trait | LOW | Move into `providers/adapter.rs` or `providers/mod.rs`. The trait is only used by `providers/anthropic.rs` and `providers/openai.rs` — it belongs with its implementors. |
| C6 | **`core/request_adapters.rs` vs root `adapter.rs`** — two files with "adapter" in the name with different meanings | MEDIUM | Rename `core/request_adapters.rs` to `core/request_transform.rs` or `core/format_bridge.rs` to distinguish it from the provider adapter trait. |
| C7 | **`core/anthropic_messages.rs`** is `pub(crate)` — Anthropic-specific message building lives in `core/` alongside generic code | LOW | Move to `providers/anthropic_messages.rs` to co-locate provider-specific logic. |
| C8 | **`core/tool_calls.rs`** is `pub(crate)` — tool call normalization utilities | LOW | These are consumed by `core/message_norm.rs` and `llm.rs`. Fine where they are, but could also live in `tools/normalize.rs` for discoverability. |
| C9 | **Conduit Cargo.toml description** says _"Core library for the eli AI assistant"_ | MEDIUM | Change to standalone description. This crate should work independently of eli. |
| C10 | **`core/mod.rs` has 15 submodules** — largest module, many concerns | LOW | Acceptable if C1 rename + C2 split happen. The module has clear submodule boundaries. |

#### Public API Surface (lib.rs re-exports)

The re-exports are comprehensive and well-curated:
- `auth::*` — API key resolvers and OAuth flows
- `clients::InternalOps` — only the trait, not implementation details ✓
- `core::errors`, `core::results` — error and stream types
- `llm::*` — the main facade
- `tape::*` — tape primitives
- `tools::*` — tool schema and execution

**One concern**: `collect_active_decisions` and `inject_decisions_into_system_prompt` are re-exported from `llm.rs` at the crate root. These are eli-specific decision management functions exposed through a "provider-agnostic" LLM crate. Consider whether they belong in eli instead.

---

### Crate: `eli`

**Structure:**
```
eli/src/
├── lib.rs              # Re-exports, 8 top-level modules
├── main.rs             # CLI entry point, tracing init
├── envelope.rs         # Envelope helpers (field_of, content_of, normalize_envelope, OutboundMessage)
├── framework.rs        # EliFramework — the orchestration core
├── hooks.rs            # EliHookSpec trait, HookRuntime, ChannelHook, TapeStoreKind
├── skills.rs           # Skill discovery and rendering
├── tools.rs            # REGISTRY global, SIDECAR_URL, model_tools, render_tools_prompt, shorten_text
├── types.rs            # Envelope type alias, State, MessageHandler, PromptValue, TurnResult
├── utils.rs            # exclude_none, workspace_from_state, get_entry_text
├── builtin/
│   ├── mod.rs          # BuiltinImpl (427 lines — default EliHookSpec implementation)
│   ├── agent.rs        # Agent struct — prompt processing engine (1059 lines)
│   ├── cli/
│   │   ├── mod.rs      # CliCommand enum, execute(), shared helpers
│   │   ├── chat.rs
│   │   ├── decisions.rs
│   │   ├── gateway.rs
│   │   ├── login.rs
│   │   ├── model.rs
│   │   ├── profile.rs
│   │   ├── run.rs
│   │   └── tape.rs
│   ├── config.rs       # EliConfig, Profile, config.toml management
│   ├── context.rs      # select_messages — tape-to-LLM-messages conversion
│   ├── settings.rs     # AgentSettings — env-based config + model spec table
│   ├── shell_manager.rs # ManagedShell, ShellManager
│   ├── store.rs        # ForkTapeStore, FileTapeStore (1014 lines)
│   ├── tape.rs         # TapeService — high-level tape operations
│   ├── tape_viewer.rs  # axum web UI for tape inspection
│   └── tools.rs        # Builtin tool implementations (bash, fs, skill, tape, web)
└── channels/
    ├── mod.rs          # Re-exports
    ├── base.rs         # Channel trait
    ├── cli.rs          # CliChannel, CliRenderer
    ├── handler.rs      # BufferedMessageHandler
    ├── manager.rs      # ChannelManager, InboundProcessor, OutboundRouter
    ├── message.rs      # ChannelMessage, MediaItem, MessageKind
    ├── telegram.rs     # TelegramChannel (757 lines)
    └── webhook.rs      # WebhookChannel
```

#### Findings

| # | Issue | Impact | Recommendation |
|---|-------|--------|----------------|
| E1 | **Duplicate `Envelope` type alias** — defined in both `types.rs:12` and `channels/manager.rs:104` | HIGH | Remove the alias from `channels/manager.rs` and import from `crate::types::Envelope`. Having two identical `pub type Envelope = Value;` in the same crate is confusing and error-prone. The `channels/mod.rs` even re-exports the one from `manager.rs`, creating an alternate path. |
| E2 | **`types.rs`** — generic name for a module containing `Envelope`, `State`, `PromptValue`, `TurnResult`, `MessageHandler`, `OutboundDispatcher`, `OutboundChannelRouter` | MEDIUM | Rename to `primitives.rs` or `core_types.rs`. These are the framework's core domain types, not throwaway "types". The name should signal importance. |
| E3 | **`utils.rs`** — grab-bag of unrelated utilities: JSON null filtering, workspace path resolution, YAML rendering | MEDIUM | Inline the functions into their callers or split: `exclude_none`/`exclude_none_map` → `envelope.rs` (JSON value manipulation), `workspace_from_state` → `framework.rs` or `builtin/settings.rs`, `get_entry_text` → `builtin/tape.rs`. Then delete `utils.rs`. |
| E4 | **`tools.rs`** mixes 4 unrelated concerns: global REGISTRY, SIDECAR_URL state, tool name conversion, prompt rendering, text truncation | HIGH | Split into: `tool_registry.rs` (REGISTRY + model_tools), `tool_prompt.rs` (render_tools_prompt), move SIDECAR_URL to `builtin/agent.rs` or a dedicated `sidecar.rs`, and move `shorten_text` to wherever it's used (it's a generic utility). |
| E5 | **`builtin/` is a 9-module monolith** acting as the default plugin, the agent runtime, config management, tape storage, shell management, tool registration, and a web server | HIGH | Restructure into purpose-driven modules: `runtime/` (agent, settings, config), `storage/` (store, tape, context), `tools/` (tools, shell_manager), and keep `builtin/mod.rs` as just the `BuiltinImpl` hook glue. The current structure makes "builtin" mean "everything that isn't a channel or the framework". |
| E6 | **`builtin/context.rs`** — confusing name, actually does tape→messages conversion (`select_messages`) | MEDIUM | Rename to `builtin/tape_context.rs` or `builtin/message_builder.rs`. The current name collides conceptually with `conduit::tools::context` (`ToolContext`). |
| E7 | **`builtin/mod.rs` is 427 lines** — contains BuiltinImpl, envelope_to_channel_message, extract_message_text, prompt_value_to_input, and the full EliHookSpec impl | LOW | The conversion functions (`envelope_to_channel_message`, `extract_message_text`, `prompt_value_to_input`) could move to `envelope.rs` or `channels/message.rs` to slim down the file. |
| E8 | **`builtin/tape.rs` vs `conduit::tape`** — naming collision across crates | LOW | Not a compile error since they're in different crates, but `eli::builtin::tape::TapeService` vs `conduit::tape::TapeManager` is confusing. Consider renaming `builtin/tape.rs` to `builtin/tape_service.rs`. |
| E9 | **`channels/manager.rs` re-exports `Envelope`** even though it's just `serde_json::Value` defined locally | HIGH | Part of E1. The channel module shouldn't define its own `Envelope` — import from `crate::types`. |
| E10 | **`EmbedInput` referenced in lib.rs re-export from `llm` module** — where is it defined? | LOW | `EmbedInput` appears in the `conduit::llm` re-export but isn't in the `llm.rs` snippet we read. Verify it exists; dead re-exports are confusing. |
| E11 | **`skills.rs` at crate root** — 495 lines mixing YAML parsing, file I/O, regex, template substitution, and rendering | MEDIUM | Consider moving to `builtin/skills.rs` since skills are a builtin feature, not a framework primitive. Or create `skills/` module with `discovery.rs` and `render.rs`. |
| E12 | **`main.rs` duplicates `eli_home()`** — same function exists in `builtin/config.rs` | LOW | Import from `builtin::config::eli_home()` instead of duplicating. |

#### Public API Surface (lib.rs re-exports)

```rust
pub use framework::EliFramework;
pub use hooks::{ChannelHook, EliHookSpec, HookError, HookRuntime, TapeStoreKind};
pub use types::{Envelope, MessageHandler, OutboundChannelRouter, OutboundDispatcher, PromptValue, State, TurnResult};
```

**Good**: Clean, minimal surface. Only framework primitives are exported.

**Concern**: `HookRuntime` is exposed — this is an internal dispatch mechanism. Plugin authors only need `EliHookSpec`, not the runtime. Consider making `HookRuntime` `pub(crate)`.

---

## Cross-Crate Issues

| # | Issue | Impact | Recommendation |
|---|-------|--------|----------------|
| X1 | **`conduit` re-exports decision functions** (`collect_active_decisions`, `inject_decisions_into_system_prompt`) that are eli-specific agent behavior | MEDIUM | Move to `eli::builtin::agent` or `eli::builtin::decisions`. Conduit should be a generic LLM toolkit. |
| X2 | **Both crates define `DEFAULT_MODEL`** — `conduit::llm` has `"openai:gpt-4o-mini"`, `eli::builtin::settings` has `"openrouter:qwen/qwen3-coder-next"` | LOW | The eli one overrides conduit's. Document which takes precedence, or remove conduit's default since eli always sets it. |
| X3 | **`eli::builtin::settings` re-exports from `conduit::core::execution`** — `ApiBaseConfig`, `ApiKeyConfig` | LOW | Fine for convenience, but these types originate deep in conduit's internal `core` module. If conduit's `core` is renamed (C1), these paths need updating. |

---

## Prioritized Action Plan

### Phase 1: Fix Correctness & Confusion (HIGH impact)

1. **E1/E9**: Remove duplicate `Envelope` type from `channels/manager.rs`. Use `crate::types::Envelope` everywhere.
2. **E4**: Split `eli::tools.rs` into `tool_registry.rs` + move SIDECAR_URL to `builtin/`.
3. **C1**: Rename `conduit::core` → `conduit::engine` (or `conduit::runtime`).

### Phase 2: Improve Discoverability (MEDIUM impact)

4. **C4**: Rename `clients/parsing/types.rs` → `clients/parsing/transport.rs`.
5. **E2**: Rename `types.rs` → `primitives.rs`.
6. **E3**: Dissolve `utils.rs` — inline functions into their logical homes.
7. **E6**: Rename `builtin/context.rs` → `builtin/tape_context.rs`.
8. **C6**: Rename `core/request_adapters.rs` → `core/request_transform.rs`.
9. **C9**: Fix conduit's Cargo.toml description.
10. **X1**: Move decision management functions from conduit to eli.

### Phase 3: Structural Improvements (MEDIUM-LOW impact)

11. **E5**: Restructure `builtin/` into purpose-driven sub-modules (`runtime/`, `storage/`, `tools/`).
12. **C2**: Split `core/execution.rs` (1176 lines) into focused files.
13. **E11**: Move `skills.rs` into `builtin/skills.rs` or create `skills/` module.
14. **C5**: Move `adapter.rs` into `providers/`.
15. **C7**: Move `anthropic_messages.rs` to `providers/`.

### Phase 4: Polish (LOW impact)

16. **E12**: Deduplicate `eli_home()`.
17. **E8**: Rename `builtin/tape.rs` → `builtin/tape_service.rs`.
18. **C3**: Rename `clients/internal.rs` → `clients/ops.rs`.
19. **E7**: Extract conversion functions from `builtin/mod.rs`.
20. Make `HookRuntime` `pub(crate)` in eli's lib.rs.

---

## What's Already Good

- **Crate split**: `conduit` and `eli` have clean, one-directional dependency
- **Module naming**: snake_case throughout, no abbreviations, no stuttering
- **Re-exports**: Both lib.rs files have curated, useful re-export blocks
- **Test placement**: Tests co-located in each file with `#[cfg(test)]` — idiomatic
- **File sizes**: Most files are 200-500 lines — good granularity (exceptions noted above)
- **`pub(crate)` usage**: Internal modules like `adapter.rs`, `anthropic_messages.rs`, `tool_calls.rs` correctly use restricted visibility
- **Workspace dependencies**: Shared deps factored into `[workspace.dependencies]`
- **Channel architecture**: Clean trait hierarchy (`Channel` for transport, `ChannelHook` for framework)
- **Tape subsystem**: Well-structured with clear separation (entries, store, manager, context, session, query)
