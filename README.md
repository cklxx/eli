# Eli

A hook-first agent framework in Rust. CLI, Telegram, Feishu, and any OpenClaw channel plugin — one pipeline.

## Quick Start

```bash
cargo install --path crates/eli
```

```bash
eli chat                    # interactive REPL
eli run "summarize this"    # one-shot
eli gateway                 # multi-channel listener
```

## Gateway + Channels

```bash
# Telegram
ELI_TELEGRAM_TOKEN=xxx eli gateway

# Feishu (via OpenClaw sidecar — auto-starts)
eli gateway --enable-channel webhook
```

The webhook channel auto-launches a Node sidecar that loads any [OpenClaw](https://github.com/nicepkg/openclaw) channel plugin (Feishu, DingTalk, Discord, Slack, ...). First run prompts for credentials interactively.

## Architecture

```
                    ┌─────────────────────────────────┐
  CLI / Telegram ──▶│          eli (Rust)              │
                    │  resolve_session → build_prompt  │
  Feishu / Slack ──▶│  → run_model → save_state →     │◀── conduit (LLM)
  (via sidecar)     │  render_outbound → dispatch      │
                    └─────────────────────────────────┘
```

**`conduit`** — Provider-agnostic LLM toolkit. Streaming, tool calls, tape storage, OAuth.

**`eli`** — Hook-first agent. Every stage is a hook. Builtins register first, plugins override.

**`@anthropic-ai/eli-sidecar`** — Node.js bridge that loads OpenClaw plugins and exposes their channels + tools to eli over HTTP.

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `ELI_MODEL` | `anthropic:claude-sonnet-4-6` | Model identifier |
| `ELI_API_KEY` | — | Provider API key |
| `ELI_API_BASE` | — | Custom endpoint |
| `ELI_MAX_STEPS` | `50` | Max tool-use iterations |

## License

[Apache-2.0](./LICENSE)
