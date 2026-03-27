<p align="center">
  <img src="assets/banner.png" alt="Eli Banner" width="100%">
</p>

<h1 align="center">Eli — Ease Lives Instantly</h1>

<p align="center">
  Open-source AI agent framework in Rust. Single binary. Hook-first architecture.<br>
  Drop it into WeChat, Feishu, Telegram, Slack, Discord — or just the terminal.
</p>

<p align="center">
  <a href="https://cklxx.github.io/eli">Website</a> ·
  <a href="#quick-start">Quick Start</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="https://github.com/cklxx/eli/issues">Issues</a>
</p>

---

I needed an AI agent in my team's group chat — not a chatbot, a teammate. Everything out there was Python, couldn't ship as a single binary, and broke under real concurrency. So I built eli in Rust.

```
$ eli chat
eli> summarize this repo

Eli is a hook-first AI agent framework in Rust. 7-stage turn pipeline,
22 builtin tools, tape-based history, provider-agnostic LLM layer (nexil).
Works across CLI, Telegram, and any channel via OpenClaw sidecar.
```

## Why Eli?

|   | Eli | Others (LangChain, CrewAI, AutoGen…) |
|---|-----|---------------------------------------|
| **Language** | Rust | Python |
| **Deploy** | Single binary | pip + deps |
| **Channels** | CLI, Telegram, WeChat, Feishu, Slack, Discord | — |

Rust performance + type safety + single-binary deploys. Smaller ecosystem, zero dependency hell.

## Quick Start

```bash
git clone https://github.com/cklxx/eli.git
cd eli && cargo build --release
cp env.example .env    # add your API key
```

```bash
eli chat                    # interactive REPL
eli run "summarize this"    # one-shot
eli gateway                 # channel listener (Telegram, Webhook, Sidecar)
```

## Features

| Feature | Detail |
|---------|--------|
| **Group chat native** | WeChat, Feishu, Telegram, Slack, Discord, DingTalk — one command |
| **Parallel tools** | Concurrent execution, faster complex tasks |
| **Auto-compact history** | Long conversations trimmed, key info preserved |
| **Progress streaming** | Reports mid-task, no silent waits |
| **Image support** | Any channel, CLI included |
| **Hot-swap models** | OpenAI, Claude, Copilot, DeepSeek, Gemini, Ollama |
| **Skill system** | Markdown-defined, project/global override |
| **MCP server** | Plug into Claude Code / Cursor |
| **Auto context fork** | Token limit → auto-branch, no interruption |

## Commands

| Command | What it does |
|---------|-------------|
| `eli chat` | Interactive REPL |
| `eli run "prompt"` | One-shot execution |
| `eli gateway` | Start channel listeners |
| `eli login` | Auth with OpenAI / Claude / Copilot |
| `eli model` | Switch or list models |
| `eli use` | Switch provider profile |
| `eli status` | Auth and config overview |
| `eli tape` | Tape viewer web UI |
| `eli decisions` | Manage persistent decisions |

## Gateway & Channels

```bash
ELI_TELEGRAM_TOKEN=xxx eli gateway          # Telegram (native)
eli gateway                                  # Feishu / WeChat / DingTalk / Discord (OpenClaw sidecar, auto-starts)
```

Sidecar = Node.js bridge loading OpenClaw channel plugins. First run prompts for credentials.

## Architecture

![Architecture](site/assets/architecture.png)

> [**Interactive diagram**](https://cklxx.github.io/eli/#architecture) &nbsp;|&nbsp; [**Video walkthrough**](https://github.com/cklxx/eli/blob/main/site/assets/architecture.mp4)

| Crate | Version | Role |
|-------|---------|------|
| **nexil** | 0.7.0 | Provider-agnostic LLM — streaming, tool calls, tape, embeddings, OAuth |
| **eli** | 0.4.2 | Hook-first agent — pipeline, channels, tools, skills, decisions |
| **eli-sidecar** | 0.2.0 | Node.js bridge — OpenClaw plugins over HTTP / MCP |

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `ELI_MODEL` | `openai:gpt-4o-mini` | `provider:model` identifier |
| `ELI_API_KEY` | — | Provider API key |
| `ELI_API_BASE` | — | Custom endpoint |
| `ELI_MAX_STEPS` | `50` | Max tool iterations per turn |
| `ELI_TELEGRAM_TOKEN` | — | Telegram bot token |
| `ELI_TELEGRAM_ALLOW_USERS` | — | User ID allowlist (comma-separated) |
| `ELI_TELEGRAM_ALLOW_CHATS` | — | Chat ID allowlist (comma-separated) |
| `ELI_WEBHOOK_PORT` | `3100` | Webhook port |
| `ELI_HOME` | `~/.eli` | Config / data directory |
| `ELI_TRACE` | — | Trace logging (`1` / `true`) |

Profiles: `~/.eli/config.toml`

## Skills

Markdown + YAML frontmatter. Discovery order:

1. `.agents/skills/<name>/SKILL.md` — project (highest)
2. `~/.eli/skills/<name>/SKILL.md` — global
3. Sidecar-synthesized (from plugin tool groups)

## License

[Apache-2.0](./LICENSE)
