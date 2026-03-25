# Workspace Structure Review — Eli (Final)

**Date**: 2026-03-24
**Standard**: Pragmatic quality for a v0.3.0 single-developer agent framework
**Workspace**: 2 crates (`conduit` v0.6.0, `eli` v0.3.0), edition 2024, resolver 2
**Method**: All claims verified against source files post-refactor (81acc98). Line counts are `wc -l` actuals. Code/test splits measured from `#[cfg(test)]` boundaries.

---

## Executive Summary

Eli is a well-structured two-crate workspace with clear domain separation: `conduit` (provider-agnostic LLM toolkit) and `eli` (hook-first agent framework). The architecture is sound — hook-based extensibility, envelope-based message passing, and tape-based history are coherent abstractions.

**Recently completed** (refactor 81acc98):
- Extracted `llm_tests.rs` (1051 lines) — `llm.rs` dropped from 2782 to 1731 lines
- Created 4 hook dispatch macros — `hooks.rs` dropped from 1078 to 1027 lines (582 code, 445 tests)
- Removed duplicate `Envelope` type alias from `channels/manager.rs`

**What matters now** (3 items, ordered by impact):

1. **`conduit/llm.rs` is still a god object** — 1728 code lines across 6 concerns (tests extracted, but builder/chat/streaming/tool-loop/embedding/decisions remain interleaved). Extract embedding and decision injection as self-contained modules.
2. **`PromptInput`/`PromptValue` duplication** — semantically identical enums with subtle behavioral differences (`trim` vs no-trim in `is_empty`). Unification prevents future drift.
3. **Tracing gap in macro-dispatched hooks** — 4 macro-dispatched hooks have no tracing; 3 manually-dispatched hooks do. The higher-criticality hooks (`resolve_session`, `load_state`) are the ones missing diagnostics.

**What doesn't matter yet**: Splitting medium-sized files (600-1300 lines), dissolving `types.rs`/`utils.rs`, narrowing re-exports, adding `#![warn(missing_docs)]`. These are polish for a codebase whose API is still stabilizing.

**Structural question for v1.0**: `conduit` has exactly one consumer (`eli`), and its description says "Core library for the eli AI assistant." The two-crate split has ongoing costs (46 root re-exports, adapter wrappers, `pub` where `pub(crate)` would suffice). Not a problem today, but the most important structural decision to revisit before v1.0.

**Strengths**:
- Clean crate boundary: conduit has zero dependency on eli
- Consistent patterns: builder pattern, trait objects, Arc-based sharing
- Good test coverage where it exists (`store.rs` 52% tests, `llm.rs` 38% tests extracted)
- Idiomatic Rust naming throughout

---

## File Size Analysis (Post-Refactor, Verified)

All line counts from `wc -l` on current source. Code/test split from `#[cfg(test)]` line position.

| File | Total | Code | Tests | Verdict |
|------|-------|------|-------|---------|
| `conduit/src/llm.rs` | 1731 | 1728 | 3 | **Split** — god object, 6 concerns (tests already extracted) |
| `conduit/src/llm_tests.rs` | 1051 | 0 | 1051 | Tests only — extracted from llm.rs |
| `eli/src/builtin/tools.rs` | 1358 | 1241 | 117 | Monitor — flat registration, coherent |
| `conduit/src/core/execution.rs` | 1176 | ~1176 | 0 | Monitor — complex but single-purpose |
| `conduit/src/clients/chat.rs` | 1109 | ~1109 | 0 | Monitor — single client implementation |
| `eli/src/hooks.rs` | 1027 | 582 | 445 | **Done** — macro dedup applied, 3 manual methods remain for tracing |
| `eli/src/builtin/agent.rs` | 1059 | 959 | 100 | Monitor — tightly coupled to `Agent` struct |
| `eli/src/builtin/store.rs` | 1014 | 485 | 529 | **Fine** — well-tested normal module |
| `conduit/src/tape/manager.rs` | 903 | ~903 | ~0 | Monitor |
| `conduit/src/tools/schema.rs` | 754 | ~754 | ~0 | Monitor |
| `eli/src/builtin/settings.rs` | 681 | ~681 | ~0 | Monitor |
| `eli/src/builtin/config.rs` | 598 | ~598 | ~0 | Monitor |

**Key insight**: Raw line count is a poor proxy for complexity. `store.rs` (1014 lines) crossed the threshold because it has excellent test coverage, not because it has too many responsibilities.

---

## Recommendations

### HIGH Priority

#### H1. Continue decomposing `conduit/src/llm.rs` (1728 code lines, 6 concerns)

**Status**: PARTIALLY DONE — tests extracted to `llm_tests.rs` (1051 lines). Remaining concerns still interleaved in one file.

**Current**: `LLM` struct owns builder config, sync/async chat, streaming, tool auto-loops, embedding, and decision injection.

**Remaining steps** (pragmatic, not maximal):
1. Extract `EmbedInput` + embed methods to `llm/embedding.rs` — self-contained (89-line function)
2. Extract `collect_active_decisions` + `inject_decisions_into_system_prompt` to `llm/decisions.rs` — pure transforms, no `LLM` self dependency
3. *If still too large*: extract `ToolAutoLoop` as a separate type holding `&LLM` internals

**Why not the full 6-file split**: `LLMBuilder` and `LLM` share private fields — splitting them requires `pub(crate)` field leakage. `streaming.rs` and `chat.rs` use identical internal state. Start with the clean extractions and reassess.

**Current path**: `crates/conduit/src/llm.rs`
**Proposed**: `crates/conduit/src/llm/embedding.rs`, `crates/conduit/src/llm/decisions.rs`
**Impact**: HIGH — reduces cognitive load on the largest remaining file
**Effort**: LOW — embedding and decisions are self-contained

#### H2. Unify `PromptInput` and `PromptValue`

**Current paths**: `PromptInput` at `eli/src/builtin/agent.rs:189`, `PromptValue` at `eli/src/types.rs:46`
**Problem**: Semantically identical enums (both have `Text(String)` + `Parts(Vec<Value>)`) with behavioral differences:
- `PromptInput::is_empty()` trims whitespace; `PromptValue::is_empty()` does not
- `PromptInput::text()` filters `type == "text"` objects; `PromptValue::as_text()` accepts bare strings and `{text: ...}` objects
- `builtin/mod.rs` has explicit `prompt_value_to_input` and reverse conversion functions (lines 237-282), proving these are the same type with different wrappers

**Proposed fix**: Make `PromptInput` a newtype around `PromptValue` adding trim semantics, or add `trim_empty()` to `PromptValue` and use it in agent code.

**Impact**: HIGH — prevents semantic drift, eliminates 2 conversion functions
**Effort**: MEDIUM — must audit 8+ call sites in `builtin/mod.rs` and `agent.rs`

**Challenger rebuttal**: The challenger claimed "PromptInput doesn't exist." This is incorrect — it exists at `agent.rs:189` with `pub enum PromptInput { Text(String), Parts(Vec<Value>) }` and is used across 8+ call sites including `builtin/mod.rs:24`, `builtin/mod.rs:81`, and `builtin/mod.rs:237-282`.

---

### MEDIUM Priority

#### M1. Add tracing to macro-dispatched hooks

**Current path**: `eli/src/hooks.rs`
**Problem**: The macro dedup (81acc98) created a tracing gap:

| Method | Dispatch | Tracing |
|--------|----------|---------|
| `call_resolve_session` | `call_first_upgraded!` | **No** |
| `call_load_state` | `call_collect_upgraded!` | **No** |
| `call_save_state` | `call_notify_all!` | **No** |
| `call_dispatch_outbound` | `call_notify_all!` | **No** |
| `call_build_prompt` | Manual loop | Yes |
| `call_run_model` | Manual loop | Yes |
| `call_render_outbound` | Manual loop | Yes |

The higher-criticality hooks (`resolve_session`, `load_state`) lack diagnostics while lower-criticality hooks have them.

**Proposed fix**: Add optional tracing parameters to the macros, or create tracing-aware macro variants. This would also allow the 3 remaining manual methods (29-37 lines each) to use macros, completing the dedup.

**Impact**: MEDIUM — debugging aid, completes the macro migration
**Effort**: MEDIUM — macro must accept tracing closure

#### M2. Rename `MessageHandler` in `channels/handler.rs`

**Current paths**: `types.rs:18` defines `MessageHandler = Arc<dyn Fn(Envelope) -> Pin<Box<...>>>` and `channels/handler.rs:12` defines `MessageHandler = Arc<dyn Fn(ChannelMessage) -> BoxFuture<...>>`. Different parameter types, different future boxing — **not duplicates**, but a naming collision.
**Proposed path**: Rename to `ChannelMessageHandler` in `handler.rs`
**Impact**: MEDIUM — naming clarity
**Effort**: LOW

#### M3. Extract `MODEL_SPECS` from `settings.rs`

**Current path**: `eli/src/builtin/settings.rs:36` — large static table of 20+ model families
**Proposed path**: `eli/src/builtin/model_specs.rs` or a TOML data file
**Impact**: MEDIUM — separates data from logic
**Effort**: LOW

---

### LOW Priority (defer until post-v1.0)

| # | Recommendation | Current Path | Impact | Justification |
|---|---------------|--------------|--------|---------------|
| L1 | Rename `types.rs` to `primitives.rs` | `eli/src/types.rs` (155 lines) | LOW | Style preference; small file, shared dependency root for 5+ modules |
| L2 | Rename `clients/parsing/types.rs` to `transport.rs` | `conduit/src/clients/parsing/types.rs` (54 lines) | LOW | Content is `TransportKind`, `ToolCallDelta`, `BaseTransportParser` |
| L3 | Narrow conduit `lib.rs` re-exports | `conduit/src/lib.rs` (46 re-exports) | LOW | 12 OAuth items noisy, but single-consumer crate |
| L4 | Update conduit package description | `crates/conduit/Cargo.toml:5` | LOW | Says "Core library for the eli AI assistant" — update if ever published standalone |
| L5 | Add `[workspace.lints]` for clippy settings | `Cargo.toml` | LOW | Nice-to-have for consistency |

---

### NOT Recommended (rejected with reasoning)

| Recommendation | Source | Verdict | Reasoning |
|---------------|--------|---------|-----------|
| Split `builtin/tools.rs` into domain files | Original | **Skip** | 1241 code lines of 20 flat tool-registration functions sharing 13 helpers. Splitting by domain forces helpers to `pub(super)` or duplicated |
| Split `hooks.rs` into 3 files | Original | **Skip** | One trait + one runtime. Macro dedup was the right tool. File split separates what should be read together |
| Split `builtin/agent.rs` into 3 files | Original | **Skip** | 959 code lines tightly coupled to `Agent` struct. Splitting means `pub(crate)` field leakage |
| Split `store.rs` into 3 files | Critique | **Skip** | 485 code lines, 529 test lines. Well-tested, not a god file |
| Dissolve `utils.rs` | Original | **Skip** | 178 lines, 4 well-tested functions. Moving them bloats recipients or creates circular imports |
| Dissolve `types.rs` into other modules | Original | **Skip** | 155 lines, 6 types imported by 5+ modules. Scattering creates circular import pressure |
| Add `#![warn(missing_docs)]` | Original | **Skip** | Hundreds of warnings on a zero-doc codebase. Phase in per-module after API stabilizes |
| Split `clients/chat.rs` | Original | **Skip** | 1109 lines, single client implementation. Tightly coupled |
| Split `tape/manager.rs` sync/async | Original | **Skip** | 903 lines; mirrored APIs are intentional |
| Split `llm.rs` into 6 files | Challenger | **Skip** | Tight coupling between LLM/LLMBuilder. Pragmatic 2-3 file extraction is better |

---

## Crate Boundary Analysis

The two-crate split (`conduit` + `eli`) is the most consequential structural decision in the workspace.

**Benefits**:
- Clean dependency direction (conduit knows nothing about eli)
- Forces discipline: LLM toolkit concerns stay separated from agent concerns
- Enables potential standalone conduit publication

**Costs**:
- 46 root re-exports in conduit's `lib.rs` (12 are OAuth implementation details)
- `pub` visibility where `pub(crate)` would suffice
- eli wraps conduit types extensively (`ForkTapeStore` adapts `AsyncTapeStore`, builtins wrap `conduit::Tool`)
- conduit's description says "Core library for the eli AI assistant" — not positioning itself as standalone

**Verdict**: Don't merge today. But stop treating the split as an unqualified strength. If conduit never gets external consumers, the adapter boilerplate is paying for optionality that may never be exercised. Revisit at v1.0.

---

## Module Structure (Verified, Post-Refactor)

### conduit (47 `.rs` files, ~15,300 lines)

```
conduit/src/
├── lib.rs              (35 lines)   — 46 re-exports, module declarations
├── llm.rs              (1731 lines) ⚠ 1728 code + 3 cfg(test) stub — tests extracted
├── llm_tests.rs        (1051 lines) — extracted tests from llm.rs
├── adapter.rs          (10 lines)   — ProviderAdapter trait (private)
├── auth/
│   ├── mod.rs          (109 lines)  — APIKeyResolver, re-exports
│   ├── github_copilot.rs (789 lines) — GitHub Copilot OAuth
│   └── openai_codex.rs (880 lines)  — OpenAI Codex OAuth
├── clients/
│   ├── mod.rs          (12 lines)
│   ├── chat.rs         (1109 lines) — PreparedChat + ToolCallAssembler + ChatClient
│   ├── embedding.rs    (178 lines)
│   ├── internal.rs     (474 lines)
│   ├── text.rs         (287 lines)
│   └── parsing/
│       ├── mod.rs      (30 lines)
│       ├── common.rs   (163 lines)
│       ├── completion.rs (251 lines)
│       ├── messages.rs (235 lines)
│       ├── responses.rs (371 lines)
│       └── types.rs    (54 lines)
├── core/
│   ├── mod.rs          (27 lines)
│   ├── errors.rs       (68 lines)
│   ├── api_format.rs   (32 lines)
│   ├── execution.rs    (1176 lines) — LLMCore orchestration
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
│   ├── manager.rs      (903 lines)
│   └── store.rs        (406 lines)
└── tools/
    ├── mod.rs          (12 lines)
    ├── schema.rs       (754 lines)
    ├── executor.rs     (686 lines)
    └── context.rs      (139 lines)
```

### eli (36 `.rs` files, ~13,400 lines)

```
eli/src/
├── lib.rs              (22 lines)   — 11 re-exports
├── main.rs             (102 lines)  — CLI entry point
├── types.rs            (155 lines)  — shared type aliases (Envelope, State, etc.)
├── hooks.rs            (1027 lines) — 582 code, 445 tests; 4 macros + 3 manual methods
├── framework.rs        (508 lines)  — turn pipeline
├── envelope.rs         (382 lines)  — envelope construction/helpers
├── skills.rs           (495 lines)  — skill loading/execution
├── tools.rs            (tool registry + logging helpers)
├── builtin/
│   ├── mod.rs          (428 lines)  — BuiltinImpl + envelope conversion
│   ├── agent.rs        (1059 lines) — 959 code, 100 tests; agent loop + PromptInput
│   ├── config.rs       (598 lines)
│   ├── settings.rs     (681 lines)  — includes MODEL_SPECS table
│   ├── shell_manager.rs (small)
│   ├── store.rs        (1014 lines) — 485 code, 529 tests; well-tested tape stores
│   ├── tape.rs         (313 lines)
│   ├── tape_viewer.rs  (small)
│   ├── tools.rs        (1358 lines) — 1241 code, 117 tests; 20 tool registrations
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
    ├── mod.rs          (21 lines)   — 14 re-exports
    ├── base.rs         (42 lines)   — Channel trait
    ├── message.rs      (366 lines)
    ├── handler.rs      (271 lines)  — BufferedMessageHandler + MessageHandler (naming collision)
    ├── manager.rs      (448 lines)  — Envelope duplicate removed
    ├── cli.rs          (353 lines)
    ├── telegram.rs     (757 lines)
    └── webhook.rs      (196 lines)
```

---

## Naming Convention Audit

**Module naming (snake_case)**: All correct. No violations.
**Struct/Enum naming (CamelCase)**: All correct. No violations.
**Function naming (snake_case)**: All correct. `if_()` on `TextClient` uses trailing underscore to avoid keyword — idiomatic Rust.

---

## Code Quality Ratings

| Dimension | Score | Notes |
|-----------|-------|-------|
| **Architecture** | 8/10 | Clean hook-based extensibility, solid tape abstraction, coherent crate boundary |
| **Readability** | 7/10 | Consistent naming, good module structure. `llm.rs` long functions hurt scanability |
| **DRY** | 7/10 | Hook dispatch macros improved from 6/10. Remaining: `PromptInput`/`PromptValue` duplication, tool registration boilerplate |
| **Test Coverage** | 5/10 | `store.rs` and `utils.rs` excellent. `hooks.rs` macros and `tools.rs` severely under-tested |

---

## Errata from Prior Report Versions

| Error | Source | Correction |
|-------|--------|------------|
| Report showed pre-refactor line counts | Original (2656a58) | Updated all counts to post-refactor actuals |
| H1/H3/M1 listed as pending | Original | H1-step-1, H3, M1 marked as DONE; remaining work scoped |
| "PromptInput doesn't exist" | Challenger | **Incorrect** — exists at `agent.rs:189` with 8+ usage sites |
| `hooks.rs` "1050 code, 28 tests" | Challenger | Actual: 582 code, 445 tests (`#[cfg(test)]` at line 583) |
| `hooks.rs` "633 code, 445 tests" | Original | Was correct pre-refactor; post-refactor: 582 code, 445 tests |
| `channels/mod.rs` re-export count 17/19 | Original/Critique | Actual: 14 items |
| `conduit/lib.rs` re-exports "30+" | Original | Actual: 46 items |
| `MessageHandler` is duplicated | Original | **Not a duplicate** — different param types. Reclassified as naming collision |
