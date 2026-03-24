# Structure Review — Challenges & Counter-Proposals

**Date**: 2026-03-24
**Challenging**: `structure-review.md` + `structure-review-critique.md`
**Method**: Read both reports, then verified claims against the actual codebase

---

## Challenge 1: Test lines inflate "god file" severity

Both reports count raw `wc -l` but never subtract inline tests. This changes the picture significantly:

| File | Total lines | Test lines | Actual code | Report verdict |
|------|------------|------------|-------------|----------------|
| `llm.rs` | 2782 | 1054 (38%) | **1728** | GOD FILE (overstated) |
| `store.rs` | 1014 | 528 (52%) | **486** | GOD FILE (wrong) |
| `hooks.rs` | 1078 | ~50 (5%) | **~1028** | GOD FILE (correct) |
| `agent.rs` | 1059 | ~20 (2%) | **~1039** | GOD FILE (correct) |
| `tools.rs` | 1358 | ~40 (3%) | **~1318** | GOD FILE (correct) |

**Counter-proposal**: `store.rs` is NOT a god file. The critique (GAP-1) says it "belongs in Phase 1 splits alongside C1-C3." Wrong — 486 lines of code implementing two related store types (`ForkTapeStore` + `FileTapeStore`) with shared test infrastructure is cohesive and well-scoped. Splitting it would separate two stores that share the same `AsyncTapeStore` trait for no benefit.

---

## Challenge 2: Splitting `llm.rs` into 6 files is over-engineered

The review recommends: `llm/mod.rs`, `llm/builder.rs`, `llm/chat.rs`, `llm/streaming.rs`, `llm/embedding.rs`, `llm/decisions.rs`.

Problems:
1. **Builder and struct are cohesive.** `LLMBuilder` exists solely to construct `LLM`. Separating them means cross-file `pub(crate)` fields or constructor gymnastics.
2. **`chat.rs` vs `streaming.rs` is a false split.** `stream()` and `chat_async()` share message-building, tape-recording, and provider-dispatch. Splitting forces duplication or a third "shared internals" file.
3. **Thin files.** `LLMBuilder` is ~180 lines, `EmbedInput` + embed methods ~80 lines. These become comically thin files that exist for line-count reasons.

**Counter-proposal (simpler)**: Extract only the **8 standalone functions** (lines 1374-1727): `build_messages`, `collect_active_decisions`, `inject_decisions_into_system_prompt`, `extract_content`, `extract_tool_calls`, `build_assistant_tool_call_message`, `slice_entries_by_anchor`, `build_full_context_from_entries`. These are pure transforms with no `&self` — they genuinely don't belong on `LLM`. Move them to `llm_helpers.rs` or `core/message_helpers.rs`. The remaining ~1350 lines (struct + builder + impl methods) are cohesive.

**Net result**: 1 new file instead of 5. Same benefit, 80% less churn.

---

## Challenge 3: `hooks.rs` split + macro suggestion would hurt readability

The review says the 12 `call_*` methods are "structurally identical" and suggests a macro.

- Each `call_*` has different signatures, return types, and error-handling. A macro handling `Option<String>`, `Vec<Envelope>`, `PromptValue`, and `Result<(), HookError>` would be complex, hard to debug, and invisible to `goto definition`.
- `hooks.rs` is the **framework contract** — the one file every plugin author reads. Macro-generated dispatch methods are hostile to that use case.

**Counter-proposal**: Leave `hooks.rs` as-is. 1028 lines of code for a 12-point hook system is proportionate. The repetition is a feature, not a bug — it makes the contract scannable.

---

## Challenge 4: The reports miss the biggest structural question

Neither report asks: **should `conduit` be one crate?**

`conduit` currently bundles 4 distinct concerns:
- **LLM client** (`llm.rs`, `core/`, `clients/`, `providers/`) — the actual "conduit"
- **Tape storage** (`tape/`) — append-only history, persistence, querying
- **Tool system** (`tools/`) — schema, execution, context
- **Auth/OAuth** (`auth/`) — 1669 lines of GitHub Copilot + OpenAI Codex OAuth flows

The tape system is used independently by `eli` (via `ForkTapeStore`, `FileTapeStore`). Auth has 1669 lines that nobody touches unless adding a new provider. Changes to OAuth flows trigger recompilation of the LLM client.

This matters more than splitting `hooks.rs` into two files. But — the project is 28K lines with one developer. Crate splitting has real costs (workspace complexity, cross-crate visibility, longer cold builds). At this scale, the monolith works.

**Counter-proposal**: Don't split conduit now. But if the project doubles in size or gains contributors, split `tape` and `auth` into their own crates first — they have the cleanest boundaries. This is the structural insight both reports should have surfaced instead of counting lines.

---

## Challenge 5: Dissolving `types.rs` and `utils.rs` is pure churn

Both reports flag these as "ambiguous names" and recommend dissolving.

- **`types.rs`** (155 lines): 6 type definitions (`Envelope`, `State`, `MessageHandler`, `OutboundDispatcher`, `OutboundChannelRouter`, `TurnResult`, `PromptValue`). These are the framework's vocabulary, deliberately co-located. Moving `Envelope` to `envelope.rs` conflates the type alias with the envelope helper functions. Moving `State` to `hooks.rs` couples the type to one consumer. This file is a prelude, not a grab-bag.

- **`utils.rs`** (178 lines): 4 functions, well-tested. `exclude_none` is used by both `envelope.rs` and `builtin/` modules — no single "natural home." The "dissolve into relevant modules" recommendation touches 4 files to move 4 functions, creating git churn for naming-purity points.

**Counter-proposal**: Keep both. Rename `types.rs` → `primitives.rs` if the name truly bothers you (1-line change). Don't dissolve.

---

## Challenge 6: Library-quality standards applied to an application

The review benchmarks against a "10-star open source project" and recommends narrowing re-exports, adding `#![warn(missing_docs)]`, and tiered API surfaces.

But Eli is a **personal AI agent framework** with one developer and one consumer (`eli` consuming `conduit`).

- **Re-export breadth**: 30+ items re-exported from `conduit` means `eli` writes `use conduit::LLM` instead of `use conduit::llm::LLM`. For a single-consumer workspace, this is ergonomic.
- **`missing_docs`**: On a 28K-line workspace with zero doc coverage, this adds hundreds of warnings immediately. The critique catches this (X1 → LOW).
- **API tiering**: "A user who just wants `LLM::new().chat()` shouldn't see `GitHubCopilotOAuthTokens`" — there is no such user. The only consumer is `eli`.

**Counter-proposal**: Skip library-hygiene recommendations until conduit is published independently.

---

## Challenge 7: `PromptInput`/`PromptValue` — both reports miss the design intent

Both types exist with a behavioral difference:
- `PromptValue::is_empty()` → `s.is_empty()`
- `PromptInput::is_empty()` → `s.trim().is_empty()`

The critique correctly flags the behavioral gap (GAP-3), but both reports recommend eliminating one type. They don't ask: **why do two similar types exist?**

`PromptValue` is a framework-level type (pipeline messages). `PromptInput` is an agent-level type (user input). Trim behavior exists because user input should treat whitespace-only as empty; pipeline messages should not.

**Counter-proposal**: Keep both types. Make the relationship explicit:
- (a) `PromptInput` wraps `PromptValue` (newtype pattern) with trim behavior, or
- (b) Add `impl From<PromptInput> for PromptValue` to document the conversion

Cheaper and safer than merging.

---

## Summary: What would actually improve the project

| Action | Value | Effort | Verdict |
|--------|-------|--------|---------|
| Extract 8 standalone functions from `llm.rs` | HIGH | LOW | **Do this** |
| Split `builtin/tools.rs` by tool domain | MEDIUM | MEDIUM | **Do this** — tools are genuinely independent |
| Fix `Envelope` duplicate in `channels/manager.rs` | LOW | TRIVIAL | **Do this** |
| Make `PromptInput` wrap `PromptValue` | MEDIUM | LOW | **Do this** |
| Split `hooks.rs` into spec + runtime | LOW | MEDIUM | **Skip** — cosmetic |
| Dissolve `utils.rs`/`types.rs` | NEGATIVE | MEDIUM | **Skip** — churn |
| Split `store.rs` | NEGATIVE | MEDIUM | **Skip** — it's 486 lines of code |
| Narrow conduit re-exports | LOW | LOW | **Skip** — no external consumers |
| Add `missing_docs` | LOW | HIGH | **Skip** — not useful at 0% coverage |

**4 actions, not 16.** The rest is churn.
