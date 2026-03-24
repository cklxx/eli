# r/rust Post

<!--
Post to: https://reddit.com/r/rust
Flair: Show /r/rust (or equivalent)
Title: I built an AI agent framework in Rust — here's what I learned
-->

Hey r/rust,

I've been working on **[Eli](https://github.com/cklxx/eli)** — an AI agent framework in Rust. It started because I needed an AI teammate in group chats (Telegram, Feishu) that could actually run tools, remember context, and handle multiple conversations concurrently. Everything in this space is Python, and I kept hitting the limits.

## Why Rust was the right call

**1. Single binary deploy.** Our agents run on VPS boxes, Docker containers, developers' laptops. With Rust it's just `cargo install` — no Python environments, no dependency conflicts. This alone saved us hours of ops pain.

**2. Async concurrency without the GIL.** An agent in a group chat handles multiple humans talking simultaneously, tool calls running in parallel, and streaming LLM responses — all at once. Tokio makes this natural. In Python this was a constant fight.

**3. Type-safe tool schemas.** Every tool the agent can use (shell, filesystem, web fetch, etc.) has a typed schema checked at compile time. In Python frameworks, schema mismatches are runtime surprises.

**4. Performance.** The framework overhead is ~2ms per turn. The LLM call dominates everything, but when you're running 21 tools in an agent loop, the framework tax matters.

## Architecture

The core is a 7-stage hook pipeline. Every stage (session resolution, state loading, prompt building, model execution, etc.) is a hook point. Plugins register hooks — last-registered-wins. The builtins are just the default plugins that ship first.

Two crates:
- **conduit** — Provider-agnostic LLM toolkit (streaming, tool calls, tape storage, OAuth)
- **eli** — The agent framework (pipeline, channels, tools, skills)

LLM provider is swappable via env var: OpenAI, Claude, Copilot, DeepSeek, Ollama.

## What was hard

- **Async trait bounds everywhere.** Before RPITIT stabilized, this was painful. Edition 2024 helped a lot.
- **Streaming.** Properly handling SSE streams from different LLM providers with different formats, while keeping the trait generic, took multiple iterations.
- **Telegram shutdown.** teloxide doesn't have clean graceful shutdown. We use `CancellationToken` + `abort()`. Not elegant, works.

## Current state

v0.3.0. Used daily by a small team. 12 hook points, 21 tools, CLI + Telegram + webhook channels. It's not LangChain — it's smaller, more opinionated, and deploys as a single binary.

GitHub: https://github.com/cklxx/eli

Would love feedback on the architecture, the hook system, or anything that looks off. PRs welcome.
