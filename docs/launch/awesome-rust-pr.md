# awesome-rust PR

<!--
Target repo: https://github.com/rust-unofficial/awesome-rust
File to edit: README.md
Section: Artificial Intelligence
PR title: Add eli - hook-first AI agent framework
-->

## Entry to add

Under **Artificial Intelligence** section:

```markdown
* [cklxx/eli](https://github.com/cklxx/eli) — Hook-first AI agent framework with a 7-stage pipeline, 21 builtin tools, and multi-channel support (CLI, Telegram, Feishu). [![build badge](https://github.com/cklxx/eli/actions/workflows/main.yml/badge.svg)](https://github.com/cklxx/eli/actions)
```

## PR description

Adds **eli**, a hook-first AI agent framework in Rust.

- 12 hook points with last-registered-wins plugin system
- Provider-agnostic LLM layer (OpenAI, Claude, Copilot, DeepSeek, Ollama)
- Multi-channel: CLI, Telegram, Feishu/Slack/Discord via OpenClaw sidecar
- Tape-based conversation history with anchors and forking
- 21 builtin tools (shell, filesystem, web fetch, subagent, etc.)
- Single binary deploy via `cargo install`

GitHub: https://github.com/cklxx/eli
