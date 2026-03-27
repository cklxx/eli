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

I needed an AI agent that could live in my team's group chat — not a chatbot, a teammate. Everything in the space was Python, couldn't deploy as a single binary, and fell apart the moment you needed real concurrency. So I built eli in Rust.

```
$ eli chat
eli> summarize this repo in one paragraph

Eli is a hook-first AI agent framework written in Rust. It runs a 7-stage
turn pipeline where every stage is a hook point that plugins can override.
It ships with 22 builtin tools, tape-based conversation history, and works
across CLI, Telegram, and any channel via an OpenClaw sidecar. The LLM
layer (nexil) is provider-agnostic — switch between OpenAI, Claude, Copilot,
DeepSeek, or Ollama with one env var.

eli> /quit
```

## Why Eli?

|   | Eli | LangChain | CrewAI | AutoGen |
|---|-----|-----------|--------|---------|
| **Language** | Rust | Python | Python | Python |
| **Deploy** | Single static binary | pip + deps | pip + deps | pip + deps |
| **Architecture** | Hook pipeline (12 points) | Chain / graph | Role-based crew | Multi-agent chat |
| **Channels** | CLI, Telegram, WeChat, Feishu, Slack, Discord | None (library) | None (library) | None (library) |
| **Memory** | Tape (append-only, forkable) | Various classes | Shared memory | Chat history |
| **Extensibility** | Last-registered-wins hooks | Callbacks + chains | Custom agents | Custom agents |

Eli is smaller and younger. You get Rust's performance, type safety, and single-binary deploys. If you want a mature ecosystem with hundreds of integrations, use LangChain. If you want a fast, self-contained agent that deploys anywhere and handles real concurrency, try eli.

## Quick Start

```bash
git clone https://github.com/cklxx/eli.git
cd eli && cargo build --release
cp env.example .env    # add your API key
```

```bash
eli chat                    # interactive REPL
eli run "summarize this"    # one-shot execution
eli gateway                 # multi-channel listener
```

## Features

- **Drop into any group chat** — WeChat, Feishu, Telegram, Slack, Discord, DingTalk — one command to connect, reads context, calls tools, replies in-thread
- **Parallel tool execution** — multiple tools run concurrently, complex tasks finish faster
- **Auto-compacting history** — conversations that grow too long get trimmed automatically, key info preserved
- **Progress streaming** — reports progress mid-task instead of going silent until done
- **Image support** — sends images on any channel, CLI and group chat alike
- **Hot-swap models** — OpenAI, Claude, Copilot, DeepSeek, Gemini, Ollama — switch with one command
- **Skill system** — define capabilities in Markdown, override at project or global level
- **Single binary** — `cargo install` or Docker, no dependency hell
- **MCP server** — plug into Claude Code / Cursor as a tool provider
- **Auto context branching** — approaching token limit? Auto-forks the conversation, no interruption

## Commands

| Command | Description |
|---------|-------------|
| `eli chat` | Interactive REPL with streaming |
| `eli run "prompt"` | One-shot execution |
| `eli gateway` | Start channel listeners (Telegram, Webhook, Sidecar) |
| `eli login` | Authenticate with a provider (OpenAI, Claude, Copilot) |
| `eli model` | Switch model or list available models |
| `eli use` / `eli profile` | Switch provider profile |
| `eli status` | Show auth and config status |
| `eli tape` | Open tape viewer web UI |
| `eli decisions` | Manage persistent decisions |

## Gateway & Channels

```bash
# Telegram — native channel
ELI_TELEGRAM_TOKEN=xxx eli gateway

# Feishu / WeChat / DingTalk / Discord — via OpenClaw sidecar (auto-starts)
eli gateway
```

The sidecar launches a Node.js bridge that loads any OpenClaw channel plugin. First run prompts for credentials interactively.

## Architecture

![Architecture](site/assets/architecture.png)

> [**Interactive diagram**](https://cklxx.github.io/eli/#architecture) — click any module to explore &nbsp;|&nbsp; [**Video walkthrough**](https://github.com/cklxx/eli/blob/main/site/assets/architecture.mp4)

| Crate | Version | Role |
|-------|---------|------|
| **nexil** | 0.7.0 | Provider-agnostic LLM toolkit — streaming, tool calls, tape storage, embeddings, OAuth |
| **eli** | 0.4.2 | Hook-first agent framework — pipeline, channels, tools, skills, decisions |
| **eli-sidecar** | 0.2.0 | Node.js bridge — loads OpenClaw plugins, exposes channels + tools over HTTP or MCP |

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `ELI_MODEL` | `openai:gpt-4o-mini` | Model identifier (`provider:model`) |
| `ELI_API_KEY` | — | Provider API key |
| `ELI_API_BASE` | — | Custom API endpoint |
| `ELI_MAX_STEPS` | `50` | Max tool-use iterations per turn |
| `ELI_TELEGRAM_TOKEN` | — | Telegram bot token |
| `ELI_TELEGRAM_ALLOW_USERS` | — | Comma-separated user ID allowlist |
| `ELI_TELEGRAM_ALLOW_CHATS` | — | Comma-separated chat ID allowlist |
| `ELI_WEBHOOK_PORT` | `3100` | Webhook listener port |
| `ELI_HOME` | `~/.eli` | Config and data directory |
| `ELI_TRACE` | — | Enable structured trace logging (`1` / `true`) |

Profiles: `~/.eli/config.toml` — per-provider API keys and defaults.

## Skills

Skills are Markdown files with YAML frontmatter, discovered from:

1. `.agents/skills/<name>/SKILL.md` (project — highest priority)
2. `~/.eli/skills/<name>/SKILL.md` (global)
3. Sidecar-synthesized skills (from plugin tool groups)

## License

[Apache-2.0](./LICENSE)
