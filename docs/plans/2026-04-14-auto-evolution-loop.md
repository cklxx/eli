# 2026-04-14 · Auto Evolution Loop
## Goal
- Add a governed background loop that distills, evaluates, and promotes low-risk evolution candidates without touching the prompt hot path.

## Decisions
- Trigger automation after `save_state`, asynchronously.
- Keep `SOUL.md` immutable; only materialize governed prompt rules.
- Use canary promotion with expiry and rollback instead of immediate permanent promotion.
- Record every automated action in a journal for inspection and replay.

## Steps
- Add evolution automation/policy/journal primitives.
- Wire framework + builtin state so the loop can resolve workspace and tapes.
- Add CLI/tool surfaces for `auto-run` and `history`.
- Verify with unit tests, workspace checks, and CLI end-to-end runs.
