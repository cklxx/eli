# Changelog

All notable changes to **eli-sidecar** are documented here.

## [Unreleased]

### Added
- Feishu multimedia message support: images, files, audio, video now forwarded to eli with local file paths resolved by openclaw-lark plugin
- MCP server mode (`createMcpSidecar()`) for external agent integration
- User-facing tool call notices sent to channel during execution
- QR login flow for channels that support it (e.g. WeChat)
- Channel lifecycle hooks: `initRuntime`, `onInboundMessage`, `onOutboundReply`, `wrapToolExecution`, `resolveOutboundTarget`, `renderToolCallText`
- `/send` endpoint for external agents
- `/notify` endpoint for tool progress notices
- `/setup/:channel/start` and `/setup/:channel/wait` for QR login
- Auto-generated SKILL.md for uncovered tool groups
- Sidecar token auth (`ELI_SIDECAR_TOKEN`)

### Changed
- `envelopeToEliMessage()` now forwards `media_paths` and `media_types` in context
- `dispatchReplyFromConfig` prefers `ctx.BodyForAgent` (media paths substituted) over `ctx.Body` (speaker-prefixed, no media)
- Plugin API constructor accepts runtime for lifecycle hook support
- Gateway start is fire-and-forget (non-blocking for long-poll gateways)
- Account resolution via `plugin.config.resolveAccount` before gateway start

### Fixed
- Feishu typing reaction cleanup: serialized per-session to prevent race conditions
- Typing indicator removed via lifecycle hooks or legacy fallback with reaction listing
- Sidecar tool name mismatch resolved; session_id passed for auth context
- Feishu reactions cleaned up on empty replies
- Oversized feishu-create-doc SKILL.md split to avoid context truncation
- Tool descriptions tightened across multiple review passes
