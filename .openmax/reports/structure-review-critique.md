# Structure Review Critique (Rewrite)

**Date**: 2026-03-24
**Reviewing**: `structure-review.md` (289 lines)
**Verdict**: Useful report with 3 factual errors and 4 gaps. Corrections below.

**Severity rubric** — shared across all pipeline reports:
- **HIGH**: Wrong recommendation that would break code, or missed >1000-line file
- **MEDIUM**: Inaccurate data that misleads prioritization
- **LOW**: Cosmetic, count-off-by-one, or debatable rating

---

## Errors (fix before acting)

**ERR-1 · HIGH — E5 is wrong: `MessageHandler` is NOT duplicated**
`types.rs:18` → `Arc<dyn Fn(Envelope) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>`
`handler.rs:12` → `Arc<dyn Fn(ChannelMessage) -> BoxFuture<'static, ()> + Send + Sync>`
Different parameter types, different future boxing. The recommendation to delete one would break compilation. Reclassify as a **naming collision** (MEDIUM) — rename to `ChannelMessageHandler` in `handler.rs`.

**ERR-2 · MEDIUM — File sizes are estimated, not measured**
| File | Report claim | Actual (`wc -l`) |
|------|-------------|------------------|
| `builtin/store.rs` | "300+" | **1014** — missed god file |
| `builtin/config.rs` | "~250" | **598** |
| `builtin/settings.rs` | "500+" | **681** |

**ERR-3 · LOW — File count table is self-contradictory**
">1000 lines" row says count=5 but lists 6 files. Actual count including `store.rs`: **7**.

---

## Gaps (add to report)

**GAP-1 · HIGH — `builtin/store.rs` (1014 lines) missing from god files**
Contains `ForkTapeStore` + `FileTapeStore` + context vars + ~400 lines of tests. Belongs in Phase 1 splits alongside C1-C3 and E1-E3.

**GAP-2 · MEDIUM — `channels/mod.rs` re-export count wrong**
Report says 17 items (E12). Actual: **19** (count each `pub use` import in `channels/mod.rs`). Also re-exports `Envelope` from `manager.rs`, which is the same duplicate flagged in E4.

**GAP-3 · MEDIUM — E10 behavior difference not noted**
`PromptInput::is_empty()` uses `s.trim().is_empty()`. `PromptValue::is_empty()` uses `s.is_empty()`. Direct replacement changes behavior. Migration must preserve trim semantics or audit all callers.

**GAP-4 · LOW — No test coverage map**
Report recommends splitting 7 god files but doesn't note which have tests. Splitting untested code is riskier — add a tested/untested annotation to the module tree.

---

## Rating corrections

| Issue | Report rating | Corrected | Reason |
|-------|--------------|-----------|--------|
| E5 | HIGH (duplicate) | **MEDIUM** (naming collision) | Not a duplicate — different types |
| E4 | HIGH | **MEDIUM** | Both are `Value` aliases — confusion, not breakage |
| X1 (`missing_docs`) | MEDIUM | **LOW** | Adding to a codebase with zero doc coverage = hundreds of warnings. Phase in per-module. |

---

## Action plan (trimmed to top 5)

The original 16-item plan is a wish list. These 5 deliver 80% of the value:

1. **Split `llm.rs`** (2782 lines → ~6 files) — highest line count, most mixed concerns
2. **Split `builtin/tools.rs`** (1358 lines → grouped by tool domain)
3. **Split `builtin/store.rs`** (1014 lines → fork/file/context) — missed by original report
4. **Fix `PromptInput`/`PromptValue` duplication** — behavioral difference requires careful migration
5. **Rename `handler::MessageHandler`** → `ChannelMessageHandler` to resolve naming collision

Everything else (narrow re-exports, dissolve `utils.rs`, rename `types.rs`) is polish — do after the above ships.

---

## Report quality: 7/10

Strengths: annotated module trees are excellent, phased plan is well-structured, crate boundary analysis is accurate.

Weaknesses: estimated file sizes eroded trust, E5 false positive is a credibility issue, action plan is too broad (16 items across 3 phases for a 28K-line workspace). Lead with the top 5 items.
