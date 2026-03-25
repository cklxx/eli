# Changelog

All notable changes to the **eli** workspace are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/).

---

## [Unreleased]

### Changed
- Command prefix changed from `,` to `/`

---

## [0.3.1] — 2026-03-25

eli 0.3.1 · conduit 0.6.1

Full P0-P2 architecture hardening across both crates. 20 fixes, 8 new tests, 603 total passing.

### Fixed
- **Production panic removed** — `panic!()` in OpenAI adapter replaced with `Result<Value, ConduitError>`
- **Unsafe code eliminated** — `unsafe` pointer cast in CircuitBreaker middleware replaced with `Arc<Mutex>` clone
- **OOM protection** — 10MB response limit on `web.fetch`, 50MB file limit on `fs.read`
- **Tape memory cap** — `InMemoryTapeStore` capped at 10K entries per tape with oldest-first eviction
- **Orphan tool-call pruning** — strips individual orphaned calls instead of dropping entire assistant messages with valid content
- **ChannelManager CPU waste** — busy-poll loop (50ms `is_finished()`) replaced with direct `JoinHandle` await
- **Shell memory leak** — finished shells auto-cleaned from `ShellManager` HashMap on output read
- **Telegram shutdown hang** — 5-second poll timeout for responsive cancellation
- **`from_batch()` panic** — returns `Option<ChannelMessage>` instead of panicking on empty input
- **Anthropic release-mode guard** — `debug_assert_eq!` replaced with real transport validation returning `Result`
- **Media download silence** — failed Telegram media downloads now surface error messages in conversation
- **API key leakage** — `mask_sensitive()` sanitizer strips Bearer tokens and key prefixes from error logs
- **Sidecar startup speed** — exponential backoff (200ms→3s) replaces fixed 1-second health check delays

### Changed
- Removed 4 unused dependencies: `fuzzy-matcher`, `glob`, `which` (eli); `schemars` (conduit)
- Removed dead sync `TapeManager` field from `LLM` struct — only `AsyncTapeManager` is active
- Documented hook panic safety policy (chain-aborting vs best-effort)
- Subagent tool marked as `[EXPERIMENTAL]`

---

## [0.3.0] — 2026-03-20

eli 0.3.0 · conduit 0.6.0 · eli-sidecar 0.2.0

### Added
- **MCP server mode** — sidecar exposes tools over stdio JSON-RPC for Claude Code / Cursor (`--mcp` flag)
- **Auto-handoff** — automatic tape branching with grace period at 70% context window
- **Structured trace logging** — `ELI_TRACE=1` writes structured logs to `~/.eli/logs/`
- **Scenario-based tool descriptions** — concrete use cases in tool help text, bash `description` parameter
- **User-facing sidecar tool notices** — visibility into sidecar tool execution
- **Progressive disclosure** — sidecar tools surfaced as discoverable skills
- **Feishu multimedia** — photo, audio, video, document support in sidecar channel

### Fixed
- SSE data buffering across chunk boundaries in `LLM::stream()`
- Bearer auth missing from embedding client requests
- Lock ordering in `InMemoryTapeStore::reset()` preventing deadlock
- Sidecar process group cleanup on gateway shutdown
- OpenAI tool call delta merging by index instead of appending
- Consecutive assistant messages after aggressive tape trim
- Inbound context propagation for typing cleanup
- Feishu typing reaction cleanup on empty replies
- Tool errors fed back to LLM instead of aborting run
- `remaining==0` no longer permanently blocks future auto-handoffs
- UTF-8 safe truncation in trace output
- Sidecar auth, error classification, inbound sanitization hardened
- Tape persistence and hook runtime hardened

### Changed
- Response parser extracted into composable per-transport functions
- `build_chat_entries` extracted for DRY sync/async `record_chat`
- `aggressive_trim` helpers extracted for testability
- Data-driven model spec table for context window and max output tokens
- Sidecar made plugin-agnostic with standard SKILL.md protocol
- Hardened abstractions across conduit and eli crates

---

## [0.2.0] — 2026-03-10

### Added
- **Webhook channel** — generic HTTP bridge for external services
- **Node.js sidecar** — loads OpenClaw plugins (Feishu, DingTalk, Discord, Slack)
- **One-command gateway** — `eli gateway` starts all enabled channels + sidecar
- **Sidecar tool bridge** — external plugin tools callable from eli pipeline
- **Tape system** — append-only history with anchors, search, fork, and handoff
- **Decision system** — persistent cross-turn memory (`eli decisions`)
- **Skills system** — `SKILL.md` discovery with project/global/sidecar precedence
- **Subagent tool** — spawn subprocess agents for parallel work
- **Embedding support** — `LLM::embed()` for vector operations
- **GitHub Copilot provider** — OAuth-based authentication

### Fixed
- Anthropic OAuth adaptive thinking + Claude Code identity
- Telegram shutdown via `CancellationToken` + `abort()`

### Changed
- Crate renamed from `republic` to `conduit`
- Provider runtime centralized in conduit
- System prompt loaded from Markdown files
- Message normalization layer for cross-provider compatibility

---

## [0.1.0] — 2026-03-01

### Added
- **eli** — hook-first agent framework with 12-point pipeline
- **conduit** — provider-agnostic LLM toolkit (OpenAI, Anthropic)
- **CLI channel** — interactive REPL with streaming output
- **Telegram channel** — bot with user/chat whitelisting
- **21 builtin tools** — bash, filesystem, web fetch, tape operations
- **Profile system** — `eli login`, `eli use`, `eli status`
- **Tape storage** — file-based and in-memory stores
