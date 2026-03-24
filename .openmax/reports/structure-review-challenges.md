# Structure Review Challenges

**Date**: 2026-03-24
**Challenging**: `structure-review.md` + `structure-review-critique.md`
**Verdict**: Both reports over-index on file length, under-index on cohesion. Several recommendations would add churn without improving the codebase.

---

## Challenge 1: "God files" are inflated by inline tests — the real problem is smaller

Both reports treat raw `wc -l` as the measure of complexity. But Rust's idiomatic `#[cfg(test)] mod tests` lives in the same file. Actual code vs test split:

| File | Total lines | Code lines | Test lines | Code % |
|------|------------|------------|------------|--------|
| `llm.rs` | 2782 | ~1729 | ~1053 | 62% |
| `hooks.rs` | 1078 | ~634 | ~444 | 59% |
| `store.rs` | 1014 | ~486 | ~528 | 48% |
| `tools.rs` | 1358 | ~1242 | ~116 | 91% |

`llm.rs` at 1729 lines of actual code is still large, but it's not the emergency that "2782 lines" implies. `store.rs` is ~486 lines of code — **not a god file at all**. The critique's GAP-1 ("missed god file") is wrong: it's a normal-sized module with good test coverage.

**Counter-proposal**: Only `llm.rs` and `tools.rs` genuinely warrant scrutiny based on code complexity. `hooks.rs` at 634 lines of code with a clean trait + runtime structure is fine. `store.rs` is fine. `agent.rs` should be checked for test ratio before recommending a split.

---

## Challenge 2: Splitting `llm.rs` into 6 files is over-engineered

The report recommends: `mod.rs`, `builder.rs`, `chat.rs`, `streaming.rs`, `embedding.rs`, `decisions.rs`. But `LLMBuilder` and `LLM` are tightly coupled — the builder constructs the struct, and every method on `LLM` uses its private fields. Splitting them across files means either:

1. Making fields `pub(crate)` (leaking implementation), or
2. Passing everything through accessor methods (boilerplate)

The `decisions.rs` suggestion (C6) is for two pure functions: `collect_active_decisions` and `inject_decisions_into_system_prompt`. These are ~80 lines total. Creating a new file for 80 lines is not simplification — it's fragmentation.

**Counter-proposal**: Extract the ~1053 lines of tests to `llm/tests.rs` using `#[path]` or a `tests` submodule (instantly halves the file, zero risk). If still too large, extract `EmbedInput` + embed methods only (self-contained). Leave builder and chat methods together — they share `LLM`'s internal state.

---

## Challenge 3: Splitting `builtin/tools.rs` by domain is the wrong axis

The report suggests `tools/fs.rs`, `tools/shell.rs`, `tools/web.rs`, `tools/git.rs`. But there are only 20 tool functions, most 30-60 lines of schema definition + handler closure. Each is a self-contained factory function. There's no shared state between "tool groups."

Splitting 20 standalone functions across 5 files doesn't reduce complexity — it means opening 5 files instead of 1 to understand what tools exist. The current flat structure is a **strength**: `builtin_tools()` at the top lists every tool, making the registry discoverable at a glance.

**Counter-proposal**: Leave `tools.rs` as-is. If anything, extract the shared helpers (`bash_exec`, `resolve_path`, `format_tape_entries`) to a `tools_helpers.rs` — those are the actual shared abstractions, not the tool definitions.

---

## Challenge 4: The "duplicate types" issue is overstated

The critique correctly identified that E5 (`MessageHandler`) isn't a real duplicate — the types have different signatures (`Envelope` vs `ChannelMessage` parameter). But E4 (`Envelope`) deserves scrutiny too.

`types.rs:12` — `type Envelope = Value;`
`channels/manager.rs:104` — `type Envelope = serde_json::Value;`

These are the **same type alias**. The channels module re-exports its own via `channels/mod.rs:18`. But since both resolve to `serde_json::Value`, this causes zero type errors, zero confusion at call sites — it's a 1-line local convenience alias.

**Counter-proposal**: Remove the duplicate in `channels/manager.rs` and import from `crate::types` — but rate this as **LOW**, not HIGH. It's a 1-line cleanup, not a structural crisis.

---

## Challenge 5: Both reports miss the actually important structural question

Neither report asks: **should `conduit` be a separate crate at all?**

`conduit` has exactly one consumer: `eli`. The workspace has 2 crates, not because they serve different audiences, but by historical design. The crate boundary creates real costs:

- Can't use `eli` types in `conduit` (no circular deps), forcing workarounds like the duplicate `Envelope`
- Every change to shared concepts requires coordinating across crate boundaries
- `pub(crate)` doesn't cross crate lines, so `conduit` must expose more API surface than necessary
- `eli` wraps/adapts conduit types everywhere: `ForkTapeStore` adapts `AsyncTapeStore`, builtin tools wrap `conduit::Tool`, etc.

The standard justification — "conduit could be used by other projects" — doesn't hold for a 0.6.0 crate whose own description says "Core library for the eli AI assistant."

**Counter-proposal**: Don't merge them today. But acknowledge that the two-crate split is a **design decision with ongoing costs**, not an unqualified "strength." If `conduit` is never published independently, the adapter boilerplate in `eli` is paying for optionality that may never be exercised.

---

## Challenge 6: Dissolving `types.rs` and `utils.rs` would hurt, not help

The report recommends scattering `types.rs` contents into `envelope.rs`, `framework.rs`, `hooks.rs` and `utils.rs` contents similarly. But:

- `types.rs` is imported by 5 modules (`hooks.rs`, `framework.rs`, `envelope.rs`, `builtin/mod.rs`, `utils.rs`). It's a **shared dependency root** — scattering its contents creates circular import pressure
- `types.rs` is 155 lines with 5 type definitions + tests. That's a prelude, not a grab-bag
- `utils.rs` is 60 lines of code + 118 lines of tests. Four focused functions. Not the 2000-line `utils.py` anti-pattern

Moving `State` into `hooks.rs` (already the largest file) and `Envelope` into `envelope.rs` while `framework.rs` imports both creates exactly the tangled dependency graph the report claims to be fixing.

**Counter-proposal**: Keep both files. Rename `types.rs` → `primitives.rs` if the name bothers you (5 minutes, zero risk). Don't dissolve.

---

## Challenge 7: `#![warn(missing_docs)]` would be counterproductive

The critique correctly downgraded this to LOW. But neither report does the math: ~28K lines, 80+ `pub` items in `conduit` alone, near-zero doc coverage. Enabling `missing_docs` workspace-wide produces 200+ warnings that either:

1. Get suppressed with `#[allow(missing_docs)]` everywhere (worse than nothing), or
2. Require a massive docs pass before any feature work can land

**Counter-proposal**: Don't add this lint. Document types as you touch them. If you want enforcement, add `#[warn(missing_docs)]` on individual new modules only.

---

## Challenge 8: The action plans ignore project phase

The original report proposes 16 items across 3 phases. The critique trims to 5. Neither asks: **what is the project's current priority?**

Eli is v0.3.0 — small enough that one person holds the full architecture in their head. At this stage:

- Features and correctness matter more than file organization
- API stability should precede reorganization (splitting `llm.rs` before the facade API is stable means moving code between files again when the API changes)
- The refactoring has negative expected value until scope is clearer

**My top 3** (max value, min churn):

1. **Unify `PromptInput` → `PromptValue`** — real duplication with behavioral difference (trim semantics per GAP-3). Actual bug risk.
2. **Remove `Envelope` alias in `channels/manager.rs`** — 1-line fix, removes confusion
3. **Extract `llm.rs` tests to separate file** — instantly reduces perceived complexity, zero functional risk

Everything else is optional polish that doesn't improve correctness, performance, or developer experience enough to justify the refactoring cost at v0.3.0.

---

## Verdict table

| Recommendation | Verdict | Reason |
|---------------|---------|--------|
| Split `llm.rs` into 6 files | **Over-engineered** | Extract tests only; optionally extract embedding |
| Split `tools.rs` by domain | **Churn** | 20 standalone functions don't benefit from 5 files |
| Split `store.rs` | **Wrong** | Not a god file — half the lines are tests |
| Split `hooks.rs` | **Marginal** | 634 lines of code with clean structure |
| Split `agent.rs` | **Maybe** | Need to check test ratio first |
| Dissolve `types.rs` | **Harmful** | Would create import complexity |
| Dissolve `utils.rs` | **Harmful** | 4 functions don't need 4 homes |
| Fix `PromptInput`/`PromptValue` | **Agree** | Real issue, real bug risk |
| Remove `Envelope` duplicate | **Agree** | Trivial fix |
| Rename `MessageHandler` | **Agree** | Low-cost clarity improvement |
| Add `missing_docs` | **Premature** | Would block feature work |
| Narrow conduit re-exports | **Nice-to-have** | Low priority |
| Question crate boundary | **Missing** | Neither report asks the most important structural question |
