# Structure Review Challenges — Grounded in Code

**Date**: 2026-03-24
**Challenging**: `structure-review.md` + `structure-review-critique.md`
**Method**: Every claim verified against actual source files.
**Verdict**: Both reports over-index on line count, under-index on cohesion. One phantom issue found. Several recommendations would add churn without improving the codebase.

---

## Challenge 1: "God files" are inflated by inline tests

Both reports use raw line counts as the measure of complexity. Actual code vs test split:

| File | Total | Code | Tests | Code % |
|------|-------|------|-------|--------|
| `llm.rs` | 2782 | 1728 | 1054 | 62% |
| `hooks.rs` | 1078 | 1050 | 28 | 97% |
| `store.rs` | 1014 | 485 | 528 | 48% |
| `tools.rs` | 1358 | 1270 | 88 | 94% |
| `agent.rs` | 1059 | 959 | 99 | 91% |

The critique's GAP-1 calls `store.rs` a "missed god file." It's 485 lines of code with 528 lines of tests — a **well-tested normal module**. Not a god file.

`hooks.rs` at 1050 code lines with only 28 lines of tests is the real concern — not for size, but for test coverage.

**Actual god files by code lines**: `llm.rs` (1728), `tools.rs` (1270), `agent.rs` (959). Only these three warrant splitting discussion.

---

## Challenge 2: Splitting `llm.rs` into 6 files is over-engineered

The report recommends: `mod.rs`, `builder.rs`, `chat.rs`, `streaming.rs`, `embedding.rs`, `decisions.rs`. Problems:

1. `LLMBuilder` and `LLM` share private fields — splitting means `pub(crate)` field leakage or accessor boilerplate
2. `decisions.rs` would be ~80 lines for two pure functions — fragmentation, not simplification
3. `streaming.rs` and `chat.rs` both use `LLM`'s internal state identically

**Counter-proposal**: Extract the 1054 lines of tests to `llm/tests.rs` (instantly halves the file, zero risk). If still too large, extract `EmbedInput` + embed methods (self-contained, 89-line function). Leave builder and chat together.

---

## Challenge 3: Splitting `tools.rs` by domain is the wrong cut

The report suggests `tools/fs.rs`, `tools/shell.rs`, `tools/web.rs`, `tools/git.rs`. But:

- 20 tool functions, each a self-contained factory (30-86 lines)
- No shared state between "tool groups"
- `builtin_tools()` at the top is a single registry — discoverable at a glance
- Splitting into 5 files means 5 places to look to understand what tools exist

The **actual** DRY issue in `tools.rs` is the 17 repetitions of `Tool::with_context("name", "desc", json!({...}), handler)` boilerplate, plus 35 uses of `require_str()`/`get_str()`/`get_bool()` parameter extraction. A macro or helper for tool definition would reduce more duplication than file splitting.

**Counter-proposal**: Leave as one file. If anything, extract shared helpers (`bash_exec`, `resolve_path`, `format_tape_entries`) to `tools_helpers.rs`.

---

## Challenge 4: E10 (`PromptInput`/`PromptValue` duplication) is a phantom issue

**`PromptInput` does not exist in the codebase.** Grep finds zero matches. Only `PromptValue` exists in `types.rs:45-49`. Both the original report and the critique treat this as a real issue (the critique even calls it "real bug risk" in GAP-3). It's not — the type was likely removed or never merged.

This is a credibility problem for both reports.

---

## Challenge 5: The `Envelope` duplicate is real but trivial

Confirmed: `types.rs:12` defines `pub type Envelope = Value;` and `channels/manager.rs:104` defines `pub type Envelope = serde_json::Value;`. Same type, different import style.

Both reports rate this HIGH. It's a **1-line fix**: delete the alias in `manager.rs`, import from `crate::types`. Zero behavioral change. Rating: **LOW**.

---

## Challenge 6: `hooks.rs` — the DRY issue is real, the split recommendation is wrong

11 `call_*` methods follow a near-identical pattern:
```rust
match result {
    Ok(Ok(Some(val))) => return Ok(Some(val)),
    Ok(Ok(None)) => continue,
    Ok(Err(e)) => { tracing + error wrapping; return Err(...) },
    Err(_) => { panic handling; return Err(HookError::Panic(...)) }
}
```

The report recommends splitting into `hooks/mod.rs` + `hooks/runtime.rs` + `hooks/spec.rs`. But the spec is a single trait, and the runtime is the only implementation. A 3-file split for one trait and one struct is over-engineering.

**Counter-proposal**: Write a `call_hook!` macro to deduplicate the 11 dispatch methods. This eliminates ~300 lines of repetition without changing file structure.

---

## Challenge 7: Both reports miss the crate boundary question

Neither report asks: **should `conduit` be a separate crate at all?**

`conduit` has exactly one consumer: `eli`. The crate description says "Core library for the eli AI assistant." The crate boundary creates real costs:
- Forces `pub` visibility where `pub(crate)` would suffice (46 re-exported items in `lib.rs`)
- Prevents `eli` types in `conduit` (no circular deps), causing workarounds like the duplicate `Envelope`
- `eli` wraps conduit types everywhere: `ForkTapeStore` adapts `AsyncTapeStore`, builtin tools wrap `conduit::Tool`

This is the most important structural question, and both reports dodge it by calling the two-crate split "clean."

**Counter-proposal**: Don't merge today. But acknowledge that the two-crate split has ongoing costs. If `conduit` is never published independently, the adapter boilerplate is paying for optionality that may never be exercised.

---

## Challenge 8: Dissolving `types.rs` and `utils.rs` would hurt

The report recommends scattering contents into other modules. But:

- `types.rs` (79 code lines, 5 type definitions) is imported by 5 modules — it's a **shared dependency root**
- `utils.rs` (59 code lines, 4 functions) is well-tested (119 test lines) and focused
- Moving `State` into `hooks.rs` (already 1050 lines) makes the "god file" problem worse
- Scattering creates circular import pressure between `hooks.rs`, `framework.rs`, and `envelope.rs`

**Counter-proposal**: Keep both. Rename `types.rs` → `primitives.rs` if desired (5 minutes, zero risk).

---

## Challenge 9: Action plans ignore project phase

Eli is v0.3.0. At this stage:
- Features and correctness matter more than file organization
- API stability should precede reorganization (splitting `llm.rs` before the API stabilizes means re-splitting later)
- The 16-item original plan and even the trimmed 5-item plan have negative expected value until scope is clearer

---

## Longest Functions Found (>15 line threshold)

| File | Function | Lines | Extraction candidate? |
|------|----------|-------|----------------------|
| `llm.rs:955` | `stream()` | 119 | Yes — extract retry/reconnect logic |
| `llm.rs:612` | `run_tools()` | 117 | Yes — extract tool loop from result assembly |
| `llm.rs:1135` | `embed()` | 89 | Yes — self-contained, easy extraction |
| `llm.rs:487` | `chat_async()` | 88 | Maybe — tightly coupled to LLM state |
| `tools.rs:277` | `tool_bash()` | 86 | No — schema definition, not decomposable |
| `tools.rs:1141` | `tool_sidecar()` | 86 | No — same reason |
| `llm.rs:1655` | `build_assistant_tool_call_message()` | 83 | Yes — pure transform function |
| `hooks.rs:344` | `call_run_model()` | 69 | Yes — via macro dedup |

---

## My Top 3 Recommendations (max value, min churn)

1. **Remove `Envelope` alias in `channels/manager.rs`** — 1-line fix, removes confusion. [LOW effort, real cleanup]
2. **Write `call_hook!` macro for `hooks.rs`** — eliminates ~300 lines of repetition across 11 methods. [MEDIUM effort, real DRY win]
3. **Extract `llm.rs` tests to separate file** — reduces file from 2782→1728 lines, zero functional risk. [LOW effort, high clarity win]

---

## Code Quality Ratings

| Dimension | Score | Notes |
|-----------|-------|-------|
| **DRY** | 6/10 | Hook dispatch boilerplate (11 methods), tool definition boilerplate (17 repetitions). `Envelope` alias duplicate. |
| **Readability** | 7/10 | Consistent naming, good module structure. Long functions in `llm.rs` hurt scanability. |
| **Architecture** | 7/10 | Clean hook-based extensibility, good crate boundary (even if debatable). Tape abstraction is solid. |
| **Test Coverage** | 5/10 | `store.rs` and `utils.rs` excellent. `hooks.rs` (2.6%), `tools.rs` (6.5%) severely under-tested. |

---

## Verdict Table

| Recommendation | Verdict | Reason |
|---------------|---------|--------|
| Split `llm.rs` into 6 files | **Over-engineered** | Extract tests only; optionally extract embedding |
| Split `tools.rs` by domain | **Churn** | 20 standalone functions; consider macro instead |
| Split `store.rs` | **Wrong** | 485 code lines, 528 test lines — not a god file |
| Split `hooks.rs` into 3 files | **Over-engineered** | Macro dedup is better than file split |
| Split `agent.rs` | **Marginal** | 959 code lines, but check complexity first |
| Dissolve `types.rs` | **Harmful** | Creates import complexity for 79 lines |
| Dissolve `utils.rs` | **Harmful** | 4 well-tested functions don't need 4 homes |
| Fix `PromptInput`/`PromptValue` | **Phantom** | `PromptInput` doesn't exist |
| Remove `Envelope` duplicate | **Agree** | Trivial 1-line fix |
| Rename `MessageHandler` | **Agree** | Low-cost clarity improvement |
| Add `missing_docs` | **Premature** | Would generate 200+ warnings |
| Question crate boundary | **Missing** | Most important structural question not asked |
