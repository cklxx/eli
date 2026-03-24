# Workspace Structure Review — Eli

**Date**: 2026-03-24
**Standard**: 10-star open source project
**Workspace**: 2 crates (`conduit`, `eli`), edition 2024, resolver 2

---

## Executive Summary

Eli is a well-structured two-crate workspace with clear domain separation: `conduit` (provider-agnostic LLM toolkit) and `eli` (hook-first agent framework). The architecture is sound — hook-based extensibility, envelope-based message passing, and tape-based history are coherent abstractions.

**Key issues** (in priority order):
1. **God files**: 5 files exceed 750 lines (`llm.rs` at 2782, `tools.rs` at 1358, `execution.rs` at 1176, `agent.rs` at 1059, `hooks.rs` at 1078) — these violate the "every function ≤15 lines" target
2. **Duplicate type definitions**: `Envelope` and `MessageHandler` are defined in two places each
3. **Ambiguous module names**: `types.rs`, `utils.rs` in eli; `types.rs` in conduit's parsing module
4. **Leaky re-exports**: conduit's `lib.rs` re-exports 30+ items, mixing auth OAuth details with core LLM types
5. **File placement**: some code lives in the wrong module (decisions logic in `llm.rs`, model specs in `settings.rs`)

**Strengths**:
- Clean crate boundary: conduit has zero dependency on eli
- Consistent patterns: builder pattern, trait objects, Arc-based sharing
- Good test coverage in types/utils modules
- Idiomatic Rust naming (snake_case modules, CamelCase types)

---

## Per-Crate Analysis

### Crate: `conduit` (v0.6.0)

**Purpose**: Provider-agnostic LLM toolkit — transport, streaming, tool schema, tape storage, OAuth auth.

**File count**: 47 `.rs` files | **~15,300 lines**

#### Module Structure

```
conduit/src/
├── lib.rs              (35 lines)  — re-exports, module declarations
├── llm.rs              (2782 lines) ⚠️ GOD FILE
├── adapter.rs          (10 lines)  — ProviderAdapter trait (private)
├── auth/
│   ├── mod.rs          (109 lines) — APIKeyResolver, re-exports
│   ├── github_copilot.rs (789 lines) — GitHub Copilot OAuth
│   └── openai_codex.rs (880 lines) — OpenAI Codex OAuth
├── clients/
│   ├── mod.rs          (12 lines)  — re-exports
│   ├── chat.rs         (1109 lines) ⚠️ GOD FILE
│   ├── embedding.rs    (178 lines)
│   ├── internal.rs     (474 lines)
│   ├── text.rs         (287 lines)
│   └── parsing/
│       ├── mod.rs      (30 lines)
│       ├── common.rs   (163 lines)
│       ├── completion.rs (251 lines)
│       ├── messages.rs (235 lines)
│       ├── responses.rs (371 lines)
│       └── types.rs    (54 lines) ⚠️ AMBIGUOUS NAME
├── core/
│   ├── mod.rs          (27 lines)  — re-exports
│   ├── errors.rs       (68 lines)
│   ├── api_format.rs   (32 lines)
│   ├── execution.rs    (1176 lines) ⚠️ GOD FILE
│   ├── client_registry.rs (234 lines)
│   ├── error_classify.rs (189 lines)
│   ├── request_builder.rs (305 lines)
│   ├── response_parser.rs (266 lines)
│   ├── results.rs      (355 lines)
│   ├── message_norm.rs (168 lines)
│   ├── provider_policies.rs (102 lines)
│   ├── provider_runtime.rs (174 lines)
│   ├── request_adapters.rs (66 lines)
│   ├── anthropic_messages.rs (369 lines)
│   └── tool_calls.rs   (152 lines)
├── providers/
│   ├── mod.rs          (12 lines)
│   ├── anthropic.rs    (105 lines)
│   └── openai.rs       (110 lines)
├── tape/
│   ├── mod.rs          (18 lines)
│   ├── entries.rs      (283 lines)
│   ├── context.rs      (270 lines)
│   ├── query.rs        (97 lines)
│   ├── session.rs      (64 lines)
│   ├── manager.rs      (901 lines)
│   └── store.rs        (406 lines)
└── tools/
    ├── mod.rs          (12 lines)
    ├── schema.rs       (754 lines)
    ├── executor.rs     (686 lines)
    └── context.rs      (139 lines)
```

#### Issues

| # | Issue | File(s) | Impact | Recommendation |
|---|-------|---------|--------|----------------|
| C1 | `llm.rs` is 2782 lines — largest file in workspace. Mixes builder, sync/async execution, embedding, streaming, tool auto-loop, and decision injection | `llm.rs` | **HIGH** | Split into `llm/mod.rs`, `llm/builder.rs`, `llm/execution.rs`, `llm/streaming.rs`, `llm/embedding.rs`, `llm/decisions.rs` |
| C2 | `execution.rs` is 1176 lines — `LLMCore` struct has too many responsibilities | `core/execution.rs` | **HIGH** | Extract retry logic, provider selection, and request preparation into focused modules |
| C3 | `chat.rs` is 1109 lines — mixes `PreparedChat`, `ToolCallAssembler`, and `ChatClient` | `clients/chat.rs` | **HIGH** | Split into `clients/chat/mod.rs`, `clients/chat/prepared.rs`, `clients/chat/assembler.rs` |
| C4 | `parsing/types.rs` — ambiguous name for a file containing `TransportKind`, `ToolCallDelta`, `BaseTransportParser` | `clients/parsing/types.rs` | **LOW** | Rename to `parsing/transport.rs` — content is about transport parsing contracts |
| C5 | `lib.rs` re-exports 30+ items including OAuth implementation details | `lib.rs` | **MEDIUM** | Narrow root re-exports to ~15 most-used types. Keep auth internals behind `conduit::auth::*` |
| C6 | `collect_active_decisions` and `inject_decisions_into_system_prompt` live in `llm.rs` but are pure tape/message transforms | `llm.rs` | **MEDIUM** | Move to `tape/decisions.rs` or `core/decisions.rs` |
| C7 | `manager.rs` in tape module is 901 lines with sync+async mirrored APIs | `tape/manager.rs` | **MEDIUM** | Split into `tape/sync_manager.rs` and `tape/async_manager.rs`, or use macro-based generation |
| C8 | `schema.rs` in tools is 754 lines mixing `Tool`, `ToolSet`, `ToolInput`, normalization, and schema serialization | `tools/schema.rs` | **MEDIUM** | Split into `tools/tool.rs` (Tool struct), `tools/toolset.rs` (ToolSet), `tools/normalize.rs` |
| C9 | `ApiFormat` is re-exported from `llm.rs` but defined in `core/api_format.rs` — confusing provenance | `lib.rs` | **LOW** | Re-export from `core` directly: `pub use crate::core::api_format::ApiFormat` |
| C10 | Auth files are 789 and 880 lines respectively — mostly OAuth flow boilerplate | `auth/` | **LOW** | Acceptable given OAuth complexity, but could extract shared OAuth primitives into `auth/oauth_flow.rs` |

---

### Crate: `eli` (v0.3.0)

**Purpose**: Hook-first agent framework — turn pipeline, channels, skills, CLI.

**File count**: 36 `.rs` files | **~13,000 lines**

#### Module Structure

```
eli/src/
├── lib.rs              (22 lines)  — re-exports, module declarations
├── main.rs             (102 lines) — CLI entry point
├── types.rs            (155 lines) ⚠️ AMBIGUOUS NAME
├── hooks.rs            (1078 lines) ⚠️ GOD FILE
├── framework.rs        (508 lines)
├── envelope.rs         (382 lines)
├── skills.rs           (495 lines)
├── tools.rs            (179 lines)
├── utils.rs            (178 lines) ⚠️ AMBIGUOUS NAME
├── builtin/
│   ├── mod.rs          (428 lines)
│   ├── agent.rs        (1059 lines) ⚠️ GOD FILE
│   ├── config.rs       (~250 lines)
│   ├── settings.rs     (500+ lines)
│   ├── context.rs      (small)
│   ├── shell_manager.rs (small)
│   ├── store.rs        (300+ lines)
│   ├── tape.rs         (313 lines)
│   ├── tape_viewer.rs  (small)
│   ├── tools.rs        (1358 lines) ⚠️ GOD FILE (largest in eli)
│   └── cli/
│       ├── mod.rs      (212 lines)
│       ├── run.rs      (33 lines)
│       ├── chat.rs     (51 lines)
│       ├── login.rs    (252 lines)
│       ├── model.rs    (475 lines)
│       ├── profile.rs  (137 lines)
│       ├── decisions.rs (88 lines)
│       ├── gateway.rs  (396 lines)
│       └── tape.rs     (13 lines)
└── channels/
    ├── mod.rs          (21 lines)  — re-exports
    ├── base.rs         (42 lines)  — Channel trait
    ├── message.rs      (366 lines)
    ├── handler.rs      (271 lines)
    ├── manager.rs      (450 lines)
    ├── cli.rs          (353 lines)
    ├── telegram.rs     (757 lines)
    └── webhook.rs      (196 lines)
```

#### Issues

| # | Issue | File(s) | Impact | Recommendation |
|---|-------|---------|--------|----------------|
| E1 | `builtin/tools.rs` is 1358 lines — a flat list of tool registrations | `builtin/tools.rs` | **HIGH** | Split into `builtin/tools/mod.rs`, `builtin/tools/fs.rs`, `builtin/tools/shell.rs`, `builtin/tools/web.rs`, `builtin/tools/git.rs` etc. Each tool group gets its own file |
| E2 | `hooks.rs` is 1078 lines — `HookRuntime` has 12+ `call_*` methods that are structurally identical | `hooks.rs` | **HIGH** | Extract to `hooks/mod.rs` + `hooks/runtime.rs` + `hooks/spec.rs`. Consider a macro for the repetitive `call_*` dispatch methods |
| E3 | `builtin/agent.rs` is 1059 lines — mixes agent loop, system prompt construction, and command execution | `builtin/agent.rs` | **HIGH** | Split into `builtin/agent/mod.rs`, `builtin/agent/loop.rs`, `builtin/agent/prompt.rs` |
| E4 | **Duplicate `Envelope` type**: defined in both `types.rs:12` and `channels/manager.rs:104` | `types.rs`, `channels/manager.rs` | **HIGH** | Remove duplicate from `channels/manager.rs`, import from `crate::types::Envelope` |
| E5 | **Duplicate `MessageHandler` type**: defined in both `types.rs:18` and `channels/handler.rs:12` | `types.rs`, `channels/handler.rs` | **HIGH** | Remove duplicate from `channels/handler.rs`, import from `crate::types::MessageHandler` |
| E6 | `types.rs` — generic grab-bag name | `types.rs` | **MEDIUM** | Rename to `primitives.rs` or merge contents into relevant modules (`Envelope` → `envelope.rs`, `PromptValue`/`TurnResult` → `framework.rs`, `State` → `hooks.rs`) |
| E7 | `utils.rs` — classic anti-pattern grab-bag | `utils.rs` | **MEDIUM** | `exclude_none`/`exclude_none_map` → `envelope.rs` (JSON helpers). `workspace_from_state` → `framework.rs`. `get_entry_text` → `builtin/tape.rs` |
| E8 | `builtin/settings.rs` contains `MODEL_SPECS` table (20+ model families, 500+ lines) — config data in code | `builtin/settings.rs` | **MEDIUM** | Extract model specs to a TOML/JSON data file, or at least to a dedicated `builtin/model_specs.rs` |
| E9 | `channels/telegram.rs` is 757 lines — Telegram-specific media handling, message parsing, shutdown logic | `channels/telegram.rs` | **LOW** | Could split into `channels/telegram/mod.rs`, `channels/telegram/media.rs`, but acceptable as-is given it's a single channel implementation |
| E10 | `PromptInput` enum in `builtin/agent.rs` duplicates `PromptValue` in `types.rs` — both have Text/Parts variants with identical semantics | `builtin/agent.rs`, `types.rs` | **MEDIUM** | Remove `PromptInput`, use `PromptValue` everywhere |
| E11 | `builtin/mod.rs` is 428 lines — mixes module declarations, `BuiltinImpl` struct, and envelope conversion | `builtin/mod.rs` | **MEDIUM** | Extract `BuiltinImpl` to `builtin/plugin.rs`, keep `mod.rs` as pure declarations |
| E12 | `channels/mod.rs` re-exports 17 items including implementation details like `BufferedMessageHandler` | `channels/mod.rs` | **LOW** | Narrow to trait + public structs only |

---

## Cross-Crate Issues

| # | Issue | Impact | Recommendation |
|---|-------|--------|----------------|
| X1 | No `#![warn(missing_docs)]` in either crate | **MEDIUM** | Add to both `lib.rs` for open-source quality |
| X2 | No workspace-level `[workspace.lints]` for consistent clippy/rustc settings | **LOW** | Add `[workspace.lints.clippy]` and `[workspace.lints.rust]` sections |
| X3 | `conduit` description says "Core library for the eli AI assistant" — should describe itself independently for open-source | **LOW** | Change to "Provider-agnostic LLM toolkit" |

---

## Naming Convention Audit

### Module Naming (snake_case) — Mostly Good

All module names use snake_case correctly. Specific issues:

| File | Issue | Suggestion |
|------|-------|------------|
| `types.rs` (eli) | Ambiguous — what types? | `primitives.rs` or dissolve into relevant modules |
| `utils.rs` (eli) | Classic grab-bag | Dissolve into relevant modules |
| `types.rs` (conduit/parsing) | Ambiguous | `transport.rs` |
| `results.rs` (conduit/core) | Vague — "results of what?" | `stream_types.rs` or `output.rs` |
| `internal.rs` (conduit/clients) | Internal to what? | `ops.rs` or `tape_ops.rs` |

### Struct/Enum Naming (CamelCase) — Good

All types use proper CamelCase. No issues found.

### Function Naming (snake_case) — Good

All functions use proper snake_case. Minor style note: `if_()` method on `TextClient` uses trailing underscore to avoid keyword — idiomatic Rust.

---

## Public API Surface Audit

### conduit — 80+ pub items, 6 traits

**Too broad at root**: `lib.rs` re-exports OAuth tokens, login functions, and implementation details alongside core LLM types. A user who just wants `LLM::new().chat()` shouldn't see `GitHubCopilotOAuthTokens` at the top level.

**Recommended tiers**:
1. **Root** (~10 items): `LLM`, `LLMBuilder`, `ChatRequest`, `Tool`, `ToolSet`, `ToolContext`, `ConduitError`, `ErrorKind`, `TapeEntry`, `TapeManager`
2. **Module-level** (rest): `conduit::auth::*`, `conduit::tape::*`, `conduit::tools::*`, `conduit::core::*`

### eli — 11 pub items at root

**Appropriate**: Framework, hooks, and core types. Good restraint.

---

## Prioritized Action Plan

### Phase 1: Critical (HIGH impact, fix first)

1. **Split `conduit/llm.rs`** (2782 → ~6 files of 400-500 lines each)
   - `llm/mod.rs` — `LLM` struct definition, re-exports
   - `llm/builder.rs` — `LLMBuilder`
   - `llm/chat.rs` — sync/async chat execution
   - `llm/streaming.rs` — stream_chat, stream_tool_calls
   - `llm/embedding.rs` — `EmbedInput`, embed methods
   - `llm/decisions.rs` — `collect_active_decisions`, `inject_decisions_into_system_prompt`

2. **Split `eli/builtin/tools.rs`** (1358 → grouped tool files)
   - `tools/mod.rs` — registration + `with_tape_runtime`
   - `tools/fs.rs` — filesystem tools
   - `tools/shell.rs` — shell execution
   - `tools/web.rs` — web/fetch tools
   - `tools/git.rs` — git tools

3. **Fix duplicate types** (E4, E5)
   - Remove `Envelope` from `channels/manager.rs`
   - Remove `MessageHandler` from `channels/handler.rs`
   - Import from `crate::types`

4. **Remove duplicate `PromptInput`** (E10)
   - Use `PromptValue` from `types.rs` in `builtin/agent.rs`

### Phase 2: Important (MEDIUM impact)

5. **Split `hooks.rs`** into `hooks/` module directory
6. **Split `builtin/agent.rs`** into `builtin/agent/` module directory
7. **Dissolve `types.rs`** — merge items into their natural homes
8. **Dissolve `utils.rs`** — move helpers to where they're used
9. **Extract `MODEL_SPECS`** from `settings.rs` to `model_specs.rs`
10. **Narrow conduit `lib.rs` re-exports** to ~10 core items
11. **Split `clients/chat.rs`** into focused files

### Phase 3: Polish (LOW impact)

12. Rename `parsing/types.rs` → `parsing/transport.rs`
13. Rename `core/results.rs` → `core/output.rs`
14. Add `#![warn(missing_docs)]` to both crates
15. Add `[workspace.lints]` section
16. Update conduit package description

---

## File Size Distribution

| Range | Count | Files |
|-------|-------|-------|
| >1000 lines | 5 | `llm.rs`, `tools.rs`, `execution.rs`, `chat.rs`, `agent.rs`, `hooks.rs` |
| 500-1000 | 7 | `manager.rs`, `openai_codex.rs`, `github_copilot.rs`, `schema.rs`, `executor.rs`, `telegram.rs`, `settings.rs` |
| 200-500 | 15 | Most mid-size modules |
| <200 lines | ~56 | Focused, well-scoped files |

The top 5 files account for ~7,500 lines — roughly 27% of the codebase in 6% of files. Splitting these is the single highest-leverage refactoring.
