# Eli

Hook-first AI agent framework in Rust. One pipeline for CLI, Telegram, Feishu, and any [OpenClaw](https://github.com/nicepkg/openclaw) channel plugin.

## Features

- **Hook-based pipeline** — 12 hook points, last-registered-wins. Builtins register first, your plugins override
- **Multi-channel** — CLI REPL, Telegram bot, Feishu/DingTalk/Discord/Slack via OpenClaw sidecar
- **Provider-agnostic LLM** — OpenAI, Anthropic Claude, GitHub Copilot, custom endpoints
- **21 builtin tools** — shell, filesystem, web fetch, subagent, tape operations, decisions
- **Skills** — Markdown-defined capabilities (`SKILL.md`) with project/global precedence
- **Tape system** — Append-only conversation history with anchors, search, and forking
- **MCP server mode** — Expose sidecar tools over stdio for Claude Code / Cursor integration
- **Auto-handoff** — Context-aware tape branching when approaching token limits

## Quick Start

```bash
cargo install --path crates/eli
```

```bash
eli chat                    # interactive REPL
eli run "summarize this"    # one-shot execution
eli gateway                 # multi-channel listener
```

## Commands

| Command | Description |
|---------|-------------|
| `eli chat` | Interactive REPL with streaming output |
| `eli run "prompt"` | One-shot pipeline execution |
| `eli gateway` | Start channel listeners (Telegram, Webhook) |
| `eli login` | Authenticate with a provider (OpenAI, Claude, Copilot) |
| `eli model` | Switch model or list available models |
| `eli use` / `eli profile` | Switch provider profile |
| `eli status` | Show auth and config status |
| `eli tape` | Open tape viewer web UI |
| `eli decisions` | Manage persistent decisions |

## Gateway & Channels

```bash
# Telegram
ELI_TELEGRAM_TOKEN=xxx eli gateway

# Feishu / DingTalk / Discord (via OpenClaw sidecar — auto-starts)
eli gateway --enable-channel webhook
```

The webhook channel launches a Node.js sidecar that loads any OpenClaw channel plugin. First run prompts for credentials interactively.

## Architecture

```
                    ┌──────────────────────────────────────┐
  CLI / Telegram ──>│             eli (Rust)                │
                    │                                      │
                    │  resolve_session -> load_state ->    │
  Feishu / Slack ──>│  build_prompt -> run_model ->        │<── conduit (LLM)
  (via sidecar)     │  save_state -> render_outbound ->    │
                    │  dispatch_outbound                   │
                    └──────────────────────────────────────┘
```

| Crate | Version | Role |
|-------|---------|------|
| **conduit** | 0.6.0 | Provider-agnostic LLM toolkit — streaming, tool calls, tape storage, embeddings, OAuth |
| **eli** | 0.3.0 | Hook-first agent framework — pipeline, channels, tools, skills, decisions |
| **eli-sidecar** | 0.2.0 | Node.js bridge — loads OpenClaw plugins, exposes channels + tools over HTTP or MCP |

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `ELI_MODEL` | `openai:gpt-4o-mini` | Model identifier (`provider:model`) |
| `ELI_API_KEY` | — | Provider API key |
| `ELI_API_BASE` | — | Custom API endpoint |
| `ELI_MAX_STEPS` | `50` | Max tool-use iterations per turn |
| `ELI_TELEGRAM_TOKEN` | — | Telegram bot token |
| `ELI_TELEGRAM_ALLOW_USERS` | — | Comma-separated user ID whitelist |
| `ELI_TELEGRAM_ALLOW_CHATS` | — | Comma-separated chat ID whitelist |
| `ELI_WEBHOOK_PORT` | `3100` | Webhook listener port |
| `ELI_HOME` | `~/.eli` | Config and data directory |
| `ELI_TRACE` | — | Enable structured trace logging (`1` / `true`) |

Profiles: `~/.eli/config.toml` — per-provider API keys and defaults.

## Project Structure

```
crates/
  conduit/          LLM toolkit (streaming, tools, tape, embeddings)
  eli/              Agent framework (pipeline, channels, builtins)
sidecar/            Node.js bridge for OpenClaw plugins
.agents/skills/     Project-level skill definitions
```

## Skills

Skills are Markdown files with YAML frontmatter, discovered from:

1. `.agents/skills/<name>/SKILL.md` (project — highest priority)
2. `~/.eli/skills/<name>/SKILL.md` (global)
3. Sidecar-synthesized skills (from plugin tool groups)

## License

[Apache-2.0](./LICENSE)
