# 2026-03-25 · Elegant Rust Pass

## Goal

Refactor the current Rust hotspots to better match the project contract:

- shrink oversized functions into small, composable units
- remove duplicated streaming/SSE parsing paths
- preserve file content fidelity in `fs.edit`
- reduce lock scope and duplicated orchestration logic

## Failure Modes

- changing SSE parsing semantics and breaking provider streams
- rewriting files in a way that changes newline style or trailing newline behavior
- splitting tool registration into too many abstractions that are used once
- touching the dirty `main` worktree instead of an isolated branch

## Execution

1. Fix `fs.edit` to preserve raw file layout and add coverage for newline fidelity.
2. Unify streaming parsing around a shared SSE/event decoder path.
3. Replace stringly skill frontmatter reparsing with typed frontmatter parsing.
4. Narrow framework lock scope and reduce duplicated core request orchestration.
5. Run `fmt`, `clippy`, and `test`, then prune any low-signal refactor.

## Ownership

- Worker A: `crates/eli/src/builtin/tools.rs`
- Worker B: `crates/nexil/src/clients/chat.rs` and related parsing support
- Worker C: `crates/eli/src/skills.rs`
- Worker D: provider metadata consolidation
- Main thread: framework/core integration, verification, merge handling

## Result

Completed on 2026-03-25.

- `cargo fmt --all`
- `cargo check --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
