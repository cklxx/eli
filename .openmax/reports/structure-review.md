# Workspace Structure Review — Eli (Final)

**Date**: 2026-03-24
**Standard**: Pragmatic quality for a v0.3.0 single-developer agent framework
**Workspace**: 2 crates (`conduit` v0.6.0, `eli` v0.3.0), edition 2024, resolver 2
**Method**: All claims verified against source files. Line counts are `wc -l` actuals. Code/test splits measured from `#[cfg(test)]` boundaries.

---

## Executive Summary

Eli is a well-structured two-crate workspace with clear domain separation: `conduit` (provider-agnostic LLM toolkit) and `eli` (hook-first agent framework). The architecture is sound — hook-based extensibility, envelope-based message passing, and tape-based history are coherent abstractions.

**What matters now** (3 items, ordered by impact):

1. **`conduit/llm.rs` is a god object** — 1728 lines of code across 7 distinct concerns. The only file where structural decomposition has clear ROI. Extract types (`ToolAutoLoop`, `EmbeddingClient`) not just files.
2. **`PromptInput`/`PromptValue` duplication** — semantically identical enums with subtle behavioral differences (`trim` vs no-trim in `is_empty`). Unification prevents future drift.
3. **`Envelope` type alias duplicate** — trivial 1-line fix, removes a real (if minor) source of confusion.

**What doesn't matter yet**: Splitting medium-sized files (600-1300 lines), dissolving `types.rs`/`utils.rs`, narrowing re-exports, adding `#![warn(missing_docs)]`. These are polish for a codebase whose API is still stabilizing.

**Structural question neither splitting nor renaming will answer**: `conduit` has exactly one consumer (`eli`), and its own description says "Core library for the eli AI assistant." The two-crate split has ongoing costs (46 root re-exports, adapter wrappers in eli, `pub` where `pub(crate)` would suffice). This isn't a problem to fix today, but it's the most important structural decision to revisit before v1.0.

**Strengths**:
- Clean crate boundary: conduit has zero dependency on eli
- Consistent patterns: builder pattern, trait objects, Arc-based sharing
- Good test coverage where it exists (`store.rs` 52% tests, `llm.rs` 38% tests)
- Idiomatic Rust naming throughout (snake_case modules, CamelCase types)

---

## File Size Analysis (Verified)

All line counts from `wc -l`. Code/test split from `#[cfg(test)]` line position.

| File | Total | Code | Tests | Code % | Verdict |
|------|-------|------|-------|--------|---------|
| `conduit/src/llm.rs` | 2782 | 1728 | 1054 | 62% | **Split — god object with 7 concerns** |
| `eli/src/builtin/tools.rs` | 1358 | 1241 | 117 | 91% | Monitor — flat registration, coherent |
| `conduit/src/core/execution.rs` | 1176 | ~1176 | 0 | 100% | Monitor — complex but single-purpose |
| `conduit/src/clients/chat.rs` | 1109 | ~1109 | 0 | 100% | Monitor — single client implementation |
| `eli/src/hooks.rs` | 1078 | 633 | 445 | 59% | **Macro dedup** — 11 repetitive `call_*` methods |
| `eli/src/builtin/agent.rs` | 1059 | 959 | 100 | 91% | Monitor — tightly coupled to `Agent` struct |
| `eli/src/builtin/store.rs` | 1014 | 485 | 529 | 48% | **Fine** — well-tested normal module |
| `conduit/src/tape/manager.rs` | 903 | ~903 | ~0 | ~100% | Monitor |
| `conduit/src/tools/schema.rs` | 754 | ~754 | ~0 | ~100% | Monitor |
| `eli/src/builtin/settings.rs` | 681 | ~681 | ~0 | ~100% | Monitor |
| `eli/src/builtin/config.rs` | 598 | ~598 | ~0 | ~100% | Monitor |

**Key insight**: Raw line count is a poor proxy for complexity. `store.rs` (1014 lines) crossed the threshold because it has excellent test coverage, not because it has too many responsibilities. Judge by concerns, not lines.

---

## Recommendations

### HIGH Priority

#### H1. Decompose `conduit/src/llm.rs` (1728 code lines, 7 concerns)

**Current**: `LLM` struct owns builder config, sync/async chat, streaming, tool auto-loops, embedding, and decision injection — a god *object*, not just a god file.

**Proposed approach** (pragmatic, not maximal):
1. Extract tests to `llm/tests.rs` — instantly halves the file, zero risk
2. Extract `EmbedInput` + embed methods to `llm/embedding.rs` — self-contained (89-line function)
3. Extract `collect_active_decisions` + `inject_decisions_into_system_prompt` to `llm/decisions.rs` or `tape/decisions.rs` — pure transform functions, no `LLM` self dependency
4. *If still too large*: extract `ToolAutoLoop` as a separate type holding `&LLM` internals

**Why not the full 6-file split**: `LLMBuilder` and `LLM` share private fields — splitting them requires `pub(crate)` field leakage. `streaming.rs` and `chat.rs` use identical internal state. Start with the clean extractions and reassess.

**Impact**: HIGH — reduces cognitive load, makes the largest file navigable
**Effort**: LOW-MEDIUM — test extraction is mechanical, embedding/decisions are self-contained

#### H2. Unify `PromptInput` and `PromptValue`

**Current path**: `PromptInput` at `eli/src/builtin/agent.rs:189`, `PromptValue` at `eli/src/types.rs:46`
**Problem**: Semantically identical enums (both have `Text(String)` + `Parts(Vec<Value>)`) with behavioral differences:
- `PromptInput::is_empty()` → `s.trim().is_empty()` (whitespace = empty)
- `PromptValue::is_empty()` → `s.is_empty()` (strict)
- `PromptInput::text()` filters by `type == "text"` objects
- `PromptValue::as_text()` accepts bare strings and `{text: ...}` objects

**Proposed fix**: Make `PromptInput` a newtype wrapper around `PromptValue` that adds trim semantics, or add a `trim_empty()` method to `PromptValue` and use it in agent code. The behavioral difference is intentional — the agent layer wants whitespace-only input treated as empty.

**Impact**: HIGH — prevents semantic drift between two nearly-identical types
**Effort**: MEDIUM — must audit 8+ call sites in `builtin/mod.rs` and `agent.rs`

#### H3. Remove duplicate `Envelope` type alias

**Current**: `eli/src/types.rs:12` defines `pub type Envelope = Value;` AND `eli/src/channels/manager.rs:104` defines `pub type Envelope = serde_json::Value;`
**Proposed fix**: Delete the alias in `channels/manager.rs`, import from `crate::types::Envelope`
**Impact**: LOW (trivial fix, but removes a real duplicate)
**Effort**: LOW — 1-line change

---

### MEDIUM Priority

#### M1. Deduplicate `hooks.rs` dispatch methods with a macro

**Current path**: `eli/src/hooks.rs` — 633 code lines, 11 `call_*` methods following near-identical patterns:
```rust
match result {
    Ok(Ok(Some(val))) => return Ok(Some(val)),
    Ok(Ok(None)) => continue,
    Ok(Err(e)) => { tracing + error wrapping; return Err(...) },
    Err(_) => { panic handling; return Err(HookError::Panic(...)) }
}
```

**Proposed fix**: Write a `call_hook!` macro to deduplicate the dispatch pattern. Eliminates ~200-300 lines of repetition without changing file structure.
**Why not split into 3 files**: `EliHookSpec` is a single trait with one runtime implementation. Splitting a trait from its only consumer adds indirection without clarity.

**Impact**: MEDIUM — real DRY improvement, reduces maintenance burden
**Effort**: MEDIUM — macro must handle varying return types and error semantics

#### M2. Rename `MessageHandler` in `channels/handler.rs`

**Current**: `types.rs:18` defines `MessageHandler = Arc<dyn Fn(Envelope) -> Pin<Box<...>>>` and `channels/handler.rs:12` defines `MessageHandler = Arc<dyn Fn(ChannelMessage) -> BoxFuture<...>>`. These are **not duplicates** — different parameter types, different future boxing.
**Proposed fix**: Rename to `ChannelMessageHandler` in `handler.rs` for clarity
**Impact**: MEDIUM — naming collision, even if no current scope conflict
**Effort**: LOW

#### M3. Extract `MODEL_SPECS` from `settings.rs`

**Current path**: `eli/src/builtin/settings.rs` (681 lines) contains a large static table of 20+ model families
**Proposed fix**: Move to `builtin/model_specs.rs` or a TOML data file
**Impact**: MEDIUM — separates data from logic
**Effort**: LOW

---

### LOW Priority (defer until post-v1.0)

| # | Recommendation | Current Path | Impact | Notes |
|---|---------------|--------------|--------|-------|
| L1 | Rename `eli/src/types.rs` → `primitives.rs` | `eli/src/types.rs` (155 lines) | LOW | Style preference; the file is small, tested, and serves as a shared dependency root for 5+ modules. Do NOT dissolve — scattering contents creates circular import pressure |
| L2 | Rename `conduit/src/clients/parsing/types.rs` → `transport.rs` | `clients/parsing/types.rs` (54 lines) | LOW | Content is about `TransportKind`, `ToolCallDelta`, `BaseTransportParser` |
| L3 | Narrow conduit `lib.rs` re-exports | `conduit/src/lib.rs` (46 re-exports) | LOW | 12 OAuth items at root level are noisy, but conduit has one consumer. Optimize for eli's convenience, not library aesthetics |
| L4 | Update conduit package description | `crates/conduit/Cargo.toml:5` | LOW | Currently says "Core library for the eli AI assistant" — should describe itself independently if ever published standalone |
| L5 | Add `[workspace.lints]` for consistent clippy settings | `Cargo.toml` | LOW | Nice-to-have for consistency |

---

### NOT Recommended (rejected with reasoning)

| Recommendation | Source | Verdict | Reasoning |
|---------------|--------|---------|-----------|
| Split `builtin/tools.rs` into domain files | Original (E1) | **Skip** | 1241 code lines of 20 flat tool-registration functions sharing 13 helpers. Splitting by domain (fs/shell/web/git) forces helpers to `pub(super)` or duplicated. Section headers suffice. Revisit if it grows past ~2000 lines |
| Split `hooks.rs` into 3 files | Original (E2) | **Skip** | One trait + one runtime. Macro dedup (M1) is the right tool. File split separates what should be read together |
| Split `builtin/agent.rs` into 3 files | Original (E3) | **Skip** | 959 code lines tightly coupled to `Agent` struct. Splitting means `pub(crate)` field leakage or accessor boilerplate for minimal clarity gain |
| Split `store.rs` into 3 files | Critique (GAP-1) | **Skip** | 485 code lines, 529 test lines. Well-tested normal module, not a god file |
| Dissolve `utils.rs` | Original (E7) | **Skip** | 178 lines, 4 well-tested functions. Moving `exclude_none` to `envelope.rs` (382 lines) bloats it; moving `workspace_from_state` to `framework.rs` mixes concerns |
| Dissolve `types.rs` into other modules | Original (E6) | **Skip** | 155 lines, 6 types imported by 5+ modules. Scattering creates circular import pressure. Rename to `primitives.rs` if the name bothers you |
| Add `#![warn(missing_docs)]` | Original (X1) | **Skip** | Would generate hundreds of warnings on a zero-doc codebase. Phase in per-module after API stabilizes |
| Split `clients/chat.rs` | Original (C3) | **Skip** | 1109 lines for a single client implementation (`PreparedChat`, `ToolCallAssembler`, `ChatClient`). Tightly coupled — splitting adds indirection |
| Split `tape/manager.rs` sync/async | Original (C7) | **Skip** | 903 lines; mirrored APIs are intentional for ergonomics |
| Narrow `channels/mod.rs` re-exports | Original (E12) | **Skip** | 15 items (not 17 as originally claimed). Reasonable for internal module |

---

## Crate Boundary Analysis

The two-crate split (`conduit` + `eli`) is the most consequential structural decision in the workspace, and deserves explicit acknowledgment:

**Benefits**:
- Clean dependency direction (conduit knows nothing about eli)
- Forces discipline: LLM toolkit concerns stay separated from agent concerns
- Enables potential standalone conduit publication

**Costs**:
- 46 root re-exports in conduit's `lib.rs` (12 are OAuth implementation details)
- `pub` visibility where `pub(crate)` would suffice
- eli wraps conduit types extensively (`ForkTapeStore` adapts `AsyncTapeStore`, builtins wrap `conduit::Tool`)
- The `Envelope` type alias duplicate exists precisely because eli can't put its types in conduit
- conduit's description says "Core library for the eli AI assistant" — it's not positioning itself as standalone

**Verdict**: Don't merge today. But stop treating the split as an unqualified strength. If conduit never gets external consumers, the adapter boilerplate is paying for optionality that may never be exercised. Revisit at v1.0.

---

## Module Structure (Verified)

### conduit (47 `.rs` files, ~15,300 lines)

```
conduit/src/
├── lib.rs              (35 lines)   — 46 re-exports, module declarations
├── llm.rs              (2782 lines) ⚠️ GOD OBJECT — 1728 code, 1054 tests
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

### eli (36 `.rs` files, ~13,000 lines)

```
eli/src/
├── lib.rs              (22 lines)   — 11 re-exports
├── main.rs             (102 lines)  — CLI entry point
├── types.rs            (155 lines)  — shared type aliases (Envelope, State, etc.)
├── hooks.rs            (1078 lines) — 633 code, 445 tests; 11 repetitive call_* methods
├── framework.rs        (508 lines)  — turn pipeline
├── envelope.rs         (382 lines)  — envelope construction/helpers
├── skills.rs           (495 lines)  — skill loading/execution
├── tools.rs            (179 lines)  — tool integration
├── utils.rs            (178 lines)  — 4 well-tested helper functions
├── builtin/
│   ├── mod.rs          (428 lines)  — BuiltinImpl + envelope conversion
│   ├── agent.rs        (1059 lines) — 959 code, 100 tests; agent loop + PromptInput
│   ├── config.rs       (598 lines)
│   ├── settings.rs     (681 lines)  — includes MODEL_SPECS table
│   ├── context.rs      (small)
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
    ├── mod.rs          (21 lines)   — 15 re-exports
    ├── base.rs         (42 lines)   — Channel trait
    ├── message.rs      (366 lines)
    ├── handler.rs      (271 lines)  — BufferedMessageHandler + MessageHandler (naming collision)
    ├── manager.rs      (450 lines)  — duplicate Envelope alias at line 104
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
| **Architecture** | 8/10 | Clean hook-based extensibility, solid tape abstraction, coherent crate boundary (even if debatable) |
| **Readability** | 7/10 | Consistent naming, good module structure. `llm.rs` long functions hurt scanability |
| **DRY** | 6/10 | Hook dispatch boilerplate (11 methods), tool definition boilerplate (17 repetitions), `Envelope` alias duplicate, `PromptInput`/`PromptValue` duplication |
| **Test Coverage** | 5/10 | `store.rs` and `utils.rs` excellent. `hooks.rs` and `tools.rs` severely under-tested relative to code complexity |

---

## Errata from Prior Report Versions

This final version corrects the following errors found by the review pipeline:

| Error | Source | Correction |
|-------|--------|------------|
| E5 claimed `MessageHandler` is duplicated | Original report | **Not a duplicate** — `types.rs` takes `Envelope`, `handler.rs` takes `ChannelMessage`. Different types, different future boxing. Reclassified as naming collision (MEDIUM) |
| `store.rs` listed as "300+" lines | Original report | Actually **1014 lines** (485 code + 529 tests). Missed from god file list but is NOT a god file — it's well-tested |
| `config.rs` listed as "~250" lines | Original report | Actually **598 lines** |
| `settings.rs` listed as "500+" lines | Original report | Actually **681 lines** |
| File count table said 5 but listed 6 | Original report | Corrected to 7 files >1000 lines (including `store.rs`) |
| `PromptInput` claimed not to exist | Challenger report | **It exists** at `builtin/agent.rs:189` with 8+ usage sites. The duplication with `PromptValue` is real |
| `hooks.rs` code/test split inconsistent | Challenger report | One table says "1050 code, 28 tests", another says "746 code, 332 tests". Actual: **633 code, 445 tests** (cfg(test) at line 634) |
| channels/mod.rs re-export count | Original (17), Critique (19) | Actual: **15 items** |
| conduit lib.rs re-exports "30+" | Original report | Actual: **46 items** |
