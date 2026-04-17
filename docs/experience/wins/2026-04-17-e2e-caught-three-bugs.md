# 2026-04-17 · E2E testing caught 3 bugs that unit tests + clippy missed

## Context
After shipping `feat: support agent-infer as a local inference backend` (commit `3e3e0ad`) with `cargo fmt` + `cargo clippy -D warnings` + `cargo test --workspace` (435 passed) all green, user asked "端到端测试一下" (test end-to-end). I stubbed a minimal Python OpenAI-compatible server on :8000 and exercised the full CLI flow: `eli login agent-infer` → `eli use` → `eli run "ping"`.

## What Worked
Running the actual binary against a mock server uncovered three bugs invisible to the test suite:

1. **Wrong ApiFormat**: I registered agent-infer with `ApiFormat::Responses`, but that's the OpenAI Responses API, not Chat Completions. First real inference call errored with `responses format is not supported by this provider`. Unit tests only checked that *some* ApiFormat was set, not that it matched the wire protocol the server actually speaks.
2. **Prompt/behavior mismatch**: Login prompt said "Save as profile 'agent-infer' and set active? [y/N]" — but `save_profile_with_overrides` only set active when no prior active profile existed, so confirming `y` didn't flip active. Test covered save, not the promise.
3. **Leaky env override**: `AGENT_INFER_URL=http://typo:9999` silently fell through to default :8000 because the candidate list was additive. Tests asserted the ordering, not the exclusivity a user expects when they explicitly set the var.
4. (Plus a wrong script path in miss-path help text — trivially caught by invoking the help flow.)

A ~90-line Python HTTP stub that speaks `/v1/models` + `/v1/chat/completions` was enough to catch everything. Faster than spinning up the real inference server, good enough to expose wire-format mismatches.

## Rule
**For any feature that bridges systems (provider integrations, channel adapters, tool executors), stub the far side with the minimum viable protocol and run the real binary before declaring done.** Unit tests prove internal consistency; E2E proves the wire contract. Explicit prompt/behavior pairs ("and set active?") are a contract with the user — verify the behavior matches the prompt by running it, not by reading the code. For env overrides of a search list, default to *exclusive* semantics: explicit config should not silently mask itself by falling through to defaults.
