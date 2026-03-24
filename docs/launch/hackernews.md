# Show HN: Eli — Hook-first AI agent framework in Rust

<!--
Post to: https://news.ycombinator.com/submit
Title: Show HN: Eli – A hook-first AI agent framework in Rust
URL: https://github.com/cklxx/eli
-->

I've been building AI agents that live in group chats — not as chatbots, but as teammates that can run tools, track tasks, and follow up autonomously. Everything in the agent framework space is Python. That's fine for prototyping, but I kept hitting walls: no real concurrency, hard to deploy (Python environments on every server), and fragile when multiple humans and agents share the same conversation.

So I built Eli in Rust.

**What it does:**

Every message goes through a 7-stage hook pipeline: resolve_session → load_state → build_prompt → run_model → save_state → render_outbound → dispatch_outbound. Each stage is a hook point. Builtins register first, your plugins override them — last-registered-wins, no special cases.

It ships with 21 builtin tools, a tape-based conversation history (append-only, forkable), and works across CLI, Telegram, and any IM channel (Feishu, Slack, Discord) via an OpenClaw sidecar.

The LLM layer (conduit) is provider-agnostic — switch between OpenAI, Claude, GitHub Copilot, DeepSeek, or Ollama with one env var.

**Why Rust:**

- Single binary deploy. `cargo install` and you're done. No virtualenv, no pip, no dependency hell
- Real async concurrency via tokio. Multiple conversations, multiple channels, no GIL
- Type-safe tool schemas — tool definitions are checked at compile time
- Memory safety matters when your agent runs shell commands and reads files

**What it's NOT:**

- Not a LangChain replacement. If you need 500 integrations, use LangChain
- Not multi-agent orchestration (yet). It's one agent, multiple channels
- Not production-hardened at scale. It's v0.3, used by a small team daily

**Try it:**

```
git clone https://github.com/cklxx/eli.git
cd eli && cargo build --release
cp env.example .env
eli chat
```

GitHub: https://github.com/cklxx/eli

Happy to answer questions about the architecture, the hook system, or why I chose Rust for this.
