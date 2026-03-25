# Eli — Ease Lives Instantly

> Hook-first AI agent framework in Rust. One pipeline for CLI, Telegram, and any channel.

I needed an AI agent that could live in my team's group chat — not a chatbot, a teammate. Everything in the space was Python, couldn't deploy as a single binary, and fell apart the moment you needed real concurrency. So I built eli in Rust.

<!-- TODO: replace with actual recording via `vhs demo.tape` -->
```
$ eli chat
eli> summarize this repo in one paragraph

Eli is a hook-first AI agent framework written in Rust. It runs a 7-stage
turn pipeline (resolve_session → load_state → build_prompt → run_model →
save_state → render_outbound → dispatch_outbound) where every stage is a
hook point that plugins can override. It ships with 21 builtin tools, a
tape-based conversation history, and works across CLI, Telegram, and any
channel via an OpenClaw sidecar. The LLM layer (nexil) is provider-
agnostic — switch between OpenAI, Claude, Copilot, or Ollama with one
env var. The LLM layer is called nexil.

eli> /quit
```

## Why Eli?

|   | Eli | LangChain | CrewAI | AutoGen |
|---|-----|-----------|--------|---------|
| **Language** | Rust | Python | Python | Python |
| **Binary** | Single static binary | pip install + deps | pip install + deps | pip install + deps |
| **Architecture** | Hook pipeline (12 points) | Chain/graph | Role-based crew | Multi-agent conversation |
| **Extensibility** | Last-registered-wins hooks | Callbacks + chains | Custom agents | Custom agents |
| **Channels** | CLI, Telegram, Feishu, WeChat, Slack, Discord | None (library) | None (library) | None (library) |
| **Memory** | Tape (append-only, forkable) | Various memory classes | Shared memory | Chat history |
| **Deploy** | `cargo install` or Docker | Python environment | Python environment | Python environment |

Eli is smaller and younger. The tradeoff: you get Rust's performance, type safety, and single-binary deploys. If you want a mature ecosystem with hundreds of integrations, use LangChain. If you want a fast, self-contained agent that deploys anywhere and handles real concurrency, try eli.

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

- **Hook-based pipeline** — 12 hook points, last-registered-wins. Builtins register first, your plugins override
- **Multi-channel** — CLI REPL, Telegram bot, Feishu/WeChat/DingTalk/Discord/Slack via OpenClaw sidecar
- **Provider-agnostic LLM** — OpenAI, Anthropic Claude, GitHub Copilot, DeepSeek, Ollama, custom endpoints
- **21 builtin tools** — shell, filesystem, web fetch, subagent, tape operations, decisions
- **Skills** — Markdown-defined capabilities (`SKILL.md`) with project/global precedence
- **Tape system** — Append-only conversation history with anchors, search, and forking
- **MCP server mode** — Expose sidecar tools over stdio for Claude Code / Cursor integration
- **Auto-handoff** — Context-aware tape branching when approaching token limits

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

# Feishu / WeChat / DingTalk / Discord (via OpenClaw sidecar — auto-starts)
eli gateway
```

The sidecar launches a Node.js bridge that loads any OpenClaw channel plugin. First run prompts for credentials interactively.

## Architecture

```
                    ┌──────────────────────────────────────┐
  CLI / Telegram ──>│             eli (Rust)                │
                    │                                      │
                    │  resolve_session -> load_state ->    │
  Feishu / Slack ──>│  build_prompt -> run_model ->        │<── nexil (LLM)
  (via sidecar)     │  save_state -> render_outbound ->    │
                    │  dispatch_outbound                   │
                    └──────────────────────────────────────┘
```

| Crate | Version | Role |
|-------|---------|------|
| **nexil** | 0.6.2 | Provider-agnostic LLM toolkit — streaming, tool calls, tape storage, embeddings, OAuth |
| **eli** | 0.3.2 | Hook-first agent framework — pipeline, channels, tools, skills, decisions |
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

## Skills

Skills are Markdown files with YAML frontmatter, discovered from:

1. `.agents/skills/<name>/SKILL.md` (project — highest priority)
2. `~/.eli/skills/<name>/SKILL.md` (global)
3. Sidecar-synthesized skills (from plugin tool groups)

## License

[Apache-2.0](./LICENSE)
