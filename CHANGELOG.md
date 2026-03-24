# Changelog

All notable changes to the **eli** workspace (eli + conduit crates) are documented here.

## [Unreleased]

### Added
- MCP server mode for external agent integration (`eli mcp`)
- Auto-handoff with grace period when context window approaches 70%
- Structured logging via `eli_trace`
- Scenario-based tool descriptions with bash `description` parameter
- User-facing sidecar tool notices

### Changed
- Command prefix changed from `,` to `/`
- Response parser extracted into composable per-transport functions
- `aggressive_trim` helpers extracted for DRY and testability
- `build_chat_entries` extracted to DRY sync/async `record_chat`
- Data-driven model spec table for context window and max output tokens
- Sidecar made plugin-agnostic with standard SKILL.md protocol
- Progressive disclosure: sidecar tools surfaced as skills

### Fixed
- OpenAI tool call deltas now merged by index instead of blindly appending
- SSE data buffered across chunk boundaries in `LLM::stream()`
- Bearer auth added to embedding client HTTP requests
- Lock ordering corrected in `InMemoryTapeStore::reset()` to prevent deadlock
- Sidecar process group killed on gateway shutdown
- Inbound context propagated to normal outbound for typing cleanup
- Consecutive assistant messages prevented after `aggressive_trim`
- `record_chat` reordered to place `response_text` before `tool_results`
- Tool errors fed back to LLM instead of aborting run
- Grace `remaining==0` no longer blocks future auto-handoffs permanently
- `eli_trace` suppressed from console output; UTF-8 safe truncation
- Sidecar auth, error classification, inbound sanitization hardened
- Tape persistence and hook runtime hardened
