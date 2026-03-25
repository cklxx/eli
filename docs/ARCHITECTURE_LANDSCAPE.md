# Eli — Architecture Landscape

> Generated 2026-03-25. Function-level detail, critical issues prioritized.

---

## Table of Contents

1. [System Overview](#1-system-overview)
2. [Crate: conduit — LLM Toolkit](#2-crate-conduit--llm-toolkit)
3. [Crate: eli — Agent Framework](#3-crate-eli--agent-framework)
4. [Critical Issues (P0/P1)](#4-critical-issues-p0p1)
5. [Major Issues (P2)](#5-major-issues-p2)
6. [Minor Issues (P3)](#6-minor-issues-p3)
7. [Metrics & Health](#7-metrics--health)

---

## 1. System Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                          eli (v0.3.0)                               │
│  Hook-first agent framework · 14 hook points · plugin architecture │
│                                                                     │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐  ┌──────────────────┐  │
│  │ CLI REPL │  │ Telegram │  │  Webhook  │  │   Sidecar (TS)   │  │
│  │ channel  │  │ channel  │  │  channel  │  │  OpenClaw bridge │  │
│  └────┬─────┘  └────┬─────┘  └─────┬─────┘  └────────┬─────────┘  │
│       └──────────────┴──────────────┴─────────────────┘            │
│                          ▼                                          │
│              ┌──────────────────────┐                               │
│              │   ChannelManager     │  debounce · task tracking     │
│              └──────────┬───────────┘                               │
│                         ▼                                           │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │                    EliFramework                               │  │
│  │  process_inbound() — 7-stage turn pipeline                   │  │
│  │                                                               │  │
│  │  [0] classify_inbound  → greet short-circuit                 │  │
│  │  [1] resolve_session   → session_id                          │  │
│  │  [2] load_state        → State (merged from all plugins)     │  │
│  │  [3] build_user_prompt → PromptValue (text or multimodal)    │  │
│  │  [4] run_model         → model_output string                 │  │
│  │  [5] save_state        → persist (fire-and-forget)           │  │
│  │  [6] render_outbound   → Vec<Envelope>                       │  │
│  │      dispatch_outbound → deliver to channels                 │  │
│  └──────────────────────────┬───────────────────────────────────┘  │
│                              │                                      │
│  ┌───────────────────────────┴──────────────────────────────────┐  │
│  │                    BuiltinImpl (plugin)                       │  │
│  │  Per-session Agent · SkillMatcher · SmartRouter              │  │
│  │  ToolMiddleware (CircuitBreaker + Metrics)                   │  │
│  │  TapeService · ShellManager · PromptBuilder                  │  │
│  └──────────────────────────┬───────────────────────────────────┘  │
│                              ▼                                      │
│              ┌──────────────────────────┐                           │
│              │     conduit (v0.6.0)     │                           │
│              │  Provider-agnostic LLM   │                           │
│              └──────────────────────────┘                           │
└─────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────┐
│                       conduit (v0.6.0)                              │
│  LLM toolkit · streaming · tools · tape · OAuth                    │
│                                                                     │
│  ┌─────────┐   ┌──────────────┐   ┌────────────┐   ┌───────────┐  │
│  │   LLM   │──▶│   LLMCore    │──▶│  Providers │──▶│ HTTP/SSE  │  │
│  │ Builder │   │ retry+fallbk │   │  Adapters  │   │  Client   │  │
│  └─────────┘   └──────────────┘   └────────────┘   └───────────┘  │
│       │                                                             │
│  ┌────┴────┐   ┌──────────────┐   ┌────────────┐                  │
│  │  Tools  │   │    Tape      │   │    Auth    │                   │
│  │Executor │   │  Manager     │   │  OAuth     │                   │
│  └─────────┘   └──────────────┘   └────────────┘                  │
└─────────────────────────────────────────────────────────────────────┘
```

**LOC breakdown**: conduit ~7,500 · eli ~11,300 · total Rust ~18,800

---

## 2. Crate: conduit — LLM Toolkit

### 2.1 Module Map

```
conduit/src/
├── lib.rs                          Public API re-exports
├── adapter.rs                      ProviderAdapter trait
├── llm/
│   ├── mod.rs                      LLMBuilder, LLM facade, chat/tool loops
│   ├── decisions.rs                Decision injection into system prompt
│   └── embedding.rs                EmbedInput type
├── core/
│   ├── execution.rs                LLMCore — retry loop, model candidates
│   ├── errors.rs                   ErrorKind (7 variants), ConduitError
│   ├── results.rs                  StreamEvent, TextStream, ToolExecution
│   ├── api_format.rs               ApiFormat enum (Auto/Completion/Responses/Messages)
│   ├── client_registry.rs          HTTP client cache by (provider, key, base)
│   ├── error_classify.rs           Text-heuristic error classification
│   ├── provider_runtime.rs         Transport selection, API base resolution
│   ├── request_builder.rs          TransportCallRequest, kwargs per transport
│   ├── response_parser.rs          SSE collection, response assembly
│   ├── message_norm.rs             Orphan tool-call pruning, image normalization
│   ├── tool_calls.rs               Tool call canonicalization (Anthropic↔OpenAI)
│   ├── provider_policies.rs        Per-provider toggles (usage-in-stream, etc.)
│   ├── anthropic_messages.rs       Anthropic message merging
│   └── request_adapters.rs         Request-building helpers
├── clients/
│   ├── chat.rs                     PreparedChat, ToolCallAssembler
│   ├── embedding.rs                EmbeddingClient
│   ├── text.rs                     TextClient
│   ├── internal.rs                 InternalOps (low-level)
│   └── parsing/
│       ├── types.rs                BaseTransportParser trait, ToolCallDelta
│       ├── completion.rs           OpenAI Completion parser
│       ├── responses.rs            OpenAI Responses parser
│       ├── messages.rs             Anthropic Messages parser
│       └── common.rs               Shared helpers
├── tools/
│   ├── schema.rs                   Tool, ToolSet, ToolAction
│   ├── executor.rs                 ToolExecutor (resolve + call)
│   └── context.rs                  ToolContext (tape, run_id, meta, state)
├── tape/
│   ├── entries.rs                  TapeEntry (8 kinds)
│   ├── context.rs                  TapeContext, AnchorSelector, build_messages()
│   ├── manager.rs                  TapeManager, AsyncTapeManager
│   ├── store.rs                    InMemoryTapeStore, TapeStore/AsyncTapeStore traits
│   ├── session.rs                  TapeSession (convenience wrapper)
│   └── query.rs                    TapeQuery builder (search, anchor, date, kind)
├── providers/
│   ├── openai.rs                   OpenAIAdapter (Completion + Responses)
│   └── anthropic.rs                AnthropicAdapter (Messages)
└── auth/
    ├── mod.rs                      APIKeyResolver, multi_api_key_resolver
    ├── github_copilot.rs           Copilot OAuth flow + token cache
    └── openai_codex.rs             Codex OAuth flow + token cache
```

### 2.2 Key Types — Full Function Listing

#### `LLMBuilder` / `LLM` (`llm/mod.rs`)

| Function | Signature | Purpose |
|----------|-----------|---------|
| `LLMBuilder::new` | `() → Self` | Empty builder |
| `.model` | `(&str) → Self` | Set primary model |
| `.provider` | `(&str) → Self` | Set provider |
| `.fallback_models` | `(Vec<String>) → Self` | Fallback model list |
| `.max_retries` | `(u32) → Self` | Retry count |
| `.api_key` | `(&str) → Self` | Single API key |
| `.api_key_map` | `(HashMap) → Self` | Per-provider keys |
| `.api_key_resolver` | `(APIKeyResolver) → Self` | Dynamic key resolution |
| `.api_base` | `(&str) → Self` | API base URL |
| `.api_base_map` | `(HashMap) → Self` | Per-provider bases |
| `.api_format` | `(ApiFormat) → Self` | Wire format override |
| `.verbose` | `(u32) → Self` | Verbosity 0–2 |
| `.context` | `(TapeContext) → Self` | Default tape context |
| `.tape_store` | `(impl AsyncTapeStore) → Self` | Tape backend |
| `.stream_filter` | `(StreamEventFilter) → Self` | Event filter |
| `.build` | `() → Result<LLM>` | Construct LLM |
| `LLM::chat_async` | `(&mut, ChatRequest) → Result<String>` | Non-streaming chat |
| `LLM::chat_sync` | `(&mut, ChatRequest) → Result<String>` | Blocking chat |
| `LLM::tool_calls` | `(&mut, ChatRequest) → Result<Vec<Value>>` | Extract tool calls |
| `LLM::run_tools` | `(&mut, ChatRequest) → Result<ToolAutoResult>` | Auto tool loop (≤250 rounds) |
| `LLM::append_tape_entry` | `(&self, &str, &TapeEntry) → Result<()>` | Write tape entry |
| `LLM::handoff_tape` | `(&self, tape, name, state, meta) → Result<Vec<TapeEntry>>` | Create anchor |
| `LLM::session` | `(&mut, tape) → TapeSession` | Convenience wrapper |

#### `LLMCore` (`core/execution.rs`)

| Function | Purpose |
|----------|---------|
| `resolve_model_provider(model, provider)` | Parse "provider:model" format |
| `model_candidates(override_model, override_provider)` | Build [primary] + [fallbacks] |
| `resolve_api_key(provider)` | Key lookup: explicit > resolver > map |
| `resolve_api_base(provider)` | Base URL lookup |
| `get_client(provider)` | Cached HTTP client by (provider, key, base) |
| `run_chat<T, F>(messages, tools, ...)` | **Core retry loop**: iterate candidates × retries |

#### Data Flow: `run_chat` internals

```
run_chat(messages, tools, system_prompt, ...)
  │
  ├─ model_candidates() → [(provider, model), ...]
  │
  └─ FOR EACH (provider, model):
      └─ FOR EACH attempt ≤ max_retries:
          │
          ├─ ProviderRuntime::selected_transport()
          │   → Messages | Responses | Completion
          │
          ├─ normalize_messages_for_api(messages, transport)
          │   ├─ prune_orphan_tool_messages()
          │   ├─ normalize_image_content_blocks()
          │   └─ merge_consecutive_same_role() [Anthropic only]
          │
          ├─ ProviderAdapter::build_*_body(model, messages, tools, ...)
          │   ├─ OpenAIAdapter::build_completion_body()
          │   ├─ OpenAIAdapter::build_responses_body()
          │   └─ AnthropicAdapter::build_request_body()
          │
          ├─ HTTP POST → provider API
          │
          ├─ response_parser → extract content/tool_calls/usage
          │   └─ parser_for_transport(transport) → BaseTransportParser
          │
          └─ On error:
              ├─ classify_error() → ErrorKind
              └─ should_retry() → RetrySameModel | TryNextModel
```

#### Tool System (`tools/`)

| Type/Function | Purpose |
|---------------|---------|
| `Tool::new(name, desc, params, handler)` | Tool with handler |
| `Tool::with_context(...)` | Tool receiving ToolContext |
| `Tool::schema_only(...)` | Schema-only (no handler) |
| `Tool::run(args, context)` | Execute handler |
| `ToolSet::empty()` | Empty set |
| `ToolSet::payload()` | JSON schemas for API |
| `ToolExecutor::execute_async(calls, tools, ctx)` | Run all calls, collect results |
| `ToolContext::new(run_id)` | Create context |
| `ToolAction::{Keep, Remove, Replace}` | Hook middleware actions |

#### Tape System (`tape/`)

| Type/Function | Purpose |
|---------------|---------|
| `TapeEntry::message(msg, meta)` | User/assistant message |
| `TapeEntry::anchor(name, state, meta)` | Checkpoint |
| `TapeEntry::tool_call(calls, meta)` | Tool invocations |
| `TapeEntry::tool_result(results, meta)` | Tool outputs |
| `TapeEntry::decision(text, meta)` | Active decision |
| `TapeEntry::decision_revoked(text, meta)` | Revoke decision |
| `TapeEntry::event(name, data, meta)` | Generic event |
| `TapeContext::build_query(query)` | Apply anchor selector |
| `build_messages(entries, context)` | Entries → LLM messages |
| `apply_context_budget(messages)` | Truncate: tool results 16KB, messages 400KB |
| `TapeQuery::new(tape).after_anchor().kinds().limit()` | Builder-pattern queries |
| `TapeManager::read_messages(tape, ctx)` | Load conversation history |
| `TapeManager::handoff(tape, name, state, meta)` | Create named anchor |
| `AsyncTapeManager::record_chat(...)` | Record full turn to tape |
| `InMemoryTapeStore` | In-memory backend |
| `TapeStore` / `AsyncTapeStore` | Trait for custom backends |

#### Auth (`auth/`)

| Function | Purpose |
|----------|---------|
| `multi_api_key_resolver(resolvers)` | Chain multiple resolvers |
| `login_github_copilot_oauth()` | Browser-based Copilot login |
| `github_copilot_oauth_resolver()` | Auto-refresh Copilot tokens |
| `login_openai_codex_oauth()` | Browser-based Codex login |
| `openai_codex_oauth_resolver()` | Auto-refresh Codex tokens |

#### Error System (`core/errors.rs`, `core/error_classify.rs`)

| Type | Variants/Purpose |
|------|-----------------|
| `ErrorKind` | `InvalidInput, Config, Provider, Tool, Temporary, NotFound, Unknown` |
| `ConduitError` | `{ kind, message, cause: Option<Box<ConduitError>> }` |
| `classify_error(status, body)` | HTTP status + text-heuristic → ErrorKind |
| `should_retry(kind)` | `Temporary` → retry; `Provider` → next model |

#### Parsers (`clients/parsing/`)

| Parser | Transport | Key Methods |
|--------|-----------|-------------|
| `CompletionParser` | OpenAI `/v1/chat/completions` | `extract_text`, `extract_tool_calls`, `extract_chunk_text` |
| `ResponsesParser` | OpenAI `/v1/responses` | Same interface, `response.completed` event |
| `MessagesParser` | Anthropic `/v1/messages` | Same interface, content blocks |
| `ToolCallAssembler` | All | Reconstruct tool calls from streaming deltas |

---

## 3. Crate: eli — Agent Framework

### 3.1 Module Map

```
eli/src/
├── main.rs                         Binary entry: clap, tracing init
├── lib.rs                          Public API re-exports
├── framework.rs                    EliFramework — 7-stage turn pipeline
├── hooks.rs                        EliHookSpec (14 hooks), HookRuntime
├── types.rs                        Envelope, State, PromptValue, TurnResult
├── envelope.rs                     field_of, content_of, OutboundMessage
├── prompt_builder.rs               PromptBuilder (identity/skills/runtime sections)
├── smart_router.rs                 SmartRouter — greeting detection (15 triggers)
├── skill_matcher.rs                Multi-signal skill activation (intent/tool/keyword)
├── skills.rs                       SkillMetadata discovery (project > global)
├── tools.rs                        REGISTRY, model_tools(), render_tools_prompt()
├── tool_middleware.rs              CircuitBreaker, MetricsCollector, MiddlewareChain
├── utils.rs                        exclude_none, workspace_from_state
├── channels/
│   ├── base.rs                     Channel trait (name, start, stop, send)
│   ├── cli.rs                      CliChannel — REPL with crossterm colored output
│   ├── telegram.rs                 TelegramChannel — teloxide polling, media extraction
│   ├── webhook.rs                  WebhookChannel — axum HTTP bridge for sidecar
│   ├── handler.rs                  BufferedMessageHandler — debounce/batching
│   ├── manager.rs                  ChannelManager — routing, task tracking, shutdown
│   └── message.rs                  ChannelMessage, MediaItem, MediaType
└── builtin/
    ├── mod.rs                      BuiltinImpl — primary EliHookSpec implementation
    ├── agent.rs                    Per-session Agent runtime (LLM loop, tape fork)
    ├── tools.rs                    20 builtin tools (bash, fs, tape, web, decision, ...)
    ├── store.rs                    ForkTapeStore, FileTapeStore, TapeFile
    ├── tape.rs                     TapeService (info, anchors, reset, handoff)
    ├── settings.rs                 AgentSettings, EnvConfig (ELI_* vars)
    ├── config.rs                   EliConfig, Profile (config.toml + legacy migration)
    ├── context.rs                  Tape entry → LLM message conversion
    ├── model_specs.rs              87-entry model capability table
    ├── shell_manager.rs            Background process manager
    ├── tape_viewer.rs              Axum web UI for tape inspection
    └── cli/
        ├── mod.rs                  CliCommand dispatch, strip_fake_tool_calls()
        ├── chat.rs                 /chat REPL mode
        ├── run.rs                  /run one-shot mode
        ├── gateway.rs              /gateway — multi-channel + sidecar
        ├── login.rs                /login — OAuth flows
        ├── model.rs                /model — model selection
        ├── profile.rs              /use, /status
        ├── tape.rs                 /tape — web viewer
        └── decisions.rs            /decisions — list/manage
```

### 3.2 Framework Core

#### `EliFramework` (`framework.rs`)

| Function | Purpose |
|----------|---------|
| `new()` | Default framework (cwd workspace) |
| `with_workspace(PathBuf)` | Custom workspace |
| `register_plugin(name, Arc<dyn EliHookSpec>)` | Register hook impl |
| `load_hooks(Vec<(name, Arc<dyn EliHookSpec>)>)` | Batch register |
| `process_inbound(Envelope) → Result<TurnResult>` | **Main pipeline** (7 stages) |
| `bind_outbound_router(Arc<dyn OutboundChannelRouter>)` | Set delivery target |
| `dispatch_via_router(&Envelope) → bool` | Route outbound |
| `quit_via_router(session_id)` | Cancel session tasks |
| `get_channels(MessageHandler) → HashMap<String, Box<dyn ChannelHook>>` | Collect channels |
| `get_tape_store() → Option<TapeStoreKind>` | Get tape backend |
| `get_system_prompt(&PromptValue, &State) → String` | Build system prompt |
| `hook_report() → HashMap<String, Vec<String>>` | Which plugins → which hooks |
| `plugin_status() → HashMap<String, PluginStatus>` | Plugin health |

#### 14 Hook Points (`hooks.rs`)

| # | Hook | Pattern | Panic Policy |
|---|------|---------|-------------|
| 0 | `classify_inbound(&Envelope) → Option<RouteDecision>` | First-result | Caught, skip |
| 1 | `resolve_session(&Envelope) → Result<Option<String>>` | First-result | **Aborts chain** |
| 2 | `load_state(&Envelope, &str) → Result<Option<State>>` | Collect-all, merge | **Aborts chain** |
| 3 | `build_user_prompt(&Envelope, &str, &State) → Option<PromptValue>` | First-result | Caught, skip |
| 4 | `run_model(&PromptValue, &str, &State) → Result<Option<String>>` | First-result | **Aborts chain** |
| 5 | `build_system_prompt(&str, &State) → Option<String>` | First-result (sync) | Caught |
| 6 | `save_state(&str, &State, &Envelope, &str)` | Notify-all | Caught, skip |
| 7 | `render_outbound(...) → Option<Vec<Envelope>>` | Collect-all | Caught, skip |
| 8 | `dispatch_outbound(&Envelope) → Option<bool>` | Notify-all | Caught, skip |
| 9 | `on_error(&str, &Error, Option<&Envelope>)` | Notify-all | Caught |
| 10 | `wrap_tool(Tool) → ToolAction` | All-pass (sync) | Caught |
| 11 | `provide_tape_store() → Option<TapeStoreKind>` | First-result (sync) | Caught |
| 12 | `register_cli_commands(&mut Command)` | Sync collect | Caught |
| 13 | `provide_channels(MessageHandler) → Vec<Box<dyn ChannelHook>>` | Collect-all (sync) | Caught |

**Precedence rule**: last-registered plugin wins for first-result hooks (reverse iteration).

### 3.3 Channel System

#### Channel Trait (`channels/base.rs`)

```rust
trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self, cancel: CancellationToken) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    async fn send(&self, message: ChannelMessage) -> Result<()>;  // default no-op
    fn needs_debounce(&self) -> bool;  // default false
}
```

#### Implementations

| Channel | Transport | Debounce | Media | Access Control |
|---------|-----------|----------|-------|----------------|
| `CliChannel` | stdin/stdout (crossterm) | No | No | N/A |
| `TelegramChannel` | teloxide long-polling | **Yes** | Photo/Audio/Video/Doc/Sticker | allow_users + allow_chats |
| `WebhookChannel` | axum HTTP + reqwest POST | No | Via JSON | Bearer token |

#### `ChannelManager` (`channels/manager.rs`)

| Function | Purpose |
|----------|---------|
| `new(channels, settings, enabled)` | Build manager |
| `on_receive(message)` | Route: debounce → buffer, else → direct |
| `listen_and_run(processor, cancel)` | Start channels + run loop |
| `run_loop(processor, cancel)` | Receive → spawn task per message |
| `dispatch(&Envelope) → bool` | Route outbound to correct channel |
| `quit(session_id)` | Abort all in-flight tasks for session |
| `shutdown()` | Abort all tasks, stop all channels |

#### `BufferedMessageHandler` (`channels/handler.rs`)

| Function | Purpose |
|----------|---------|
| `handle(message)` | Buffer with debounce timer |
| `schedule_flush(delay)` | Spawn delayed merge+send; new message resets timer |

**Behavior**: Commands bypass buffer (sent immediately). Inactive messages outside `active_time_window` silently dropped. Active messages debounced at `debounce_seconds`.

#### `ChannelMessage` (`channels/message.rs`)

```rust
struct ChannelMessage {
    session_id, channel, content, chat_id: String,
    is_active: bool,
    kind: MessageKind,  // Normal | Error | Command
    context: Map<String, Value>,
    media: Vec<MediaItem>,
    output_channel: String,
}
```

| Function | Purpose |
|----------|---------|
| `new(session_id, channel, content)` | Constructor |
| `.with_chat_id()` / `.with_is_active()` / `.with_media()` | Builder methods |
| `from_batch(&[Self])` | Merge N messages into 1 (content joined by `\n`) |

### 3.4 Builtin Plugin

#### `BuiltinImpl` (`builtin/mod.rs`)

| Function | Purpose |
|----------|---------|
| `new()` | Creates plugin, registers 20 tools |
| `get_or_create_agent(session_id)` | Per-session Agent cache |
| Implements all 14 `EliHookSpec` hooks | Primary hook implementation |

#### `Agent` (`builtin/agent.rs`)

| Function | Purpose |
|----------|---------|
| `new()` | Load settings from env |
| `run(prompt, session_id, state, ...)` | **Main execution**: fork tape → detect slash cmd → agent_loop → merge |
| `system_prompt(state, skills, ...)` | PromptBuilder + SkillMatcher |
| `create_llm(model, state)` | Construct conduit::LLM with API key chain |
| `resolve_stored_api_key()` | Check ~/.codex/auth.json, ~/.eli/auth.json, Copilot |
| `build_tool_context(tape, state)` | ToolContext with tape + state |
| `lookup_registered_tool(name)` | Resolve with . ↔ _ alias fallback |
| `run_command(input, session_id)` | Execute slash-command or bash |

#### 20 Builtin Tools (`builtin/tools.rs`)

| Tool | Context | Purpose |
|------|---------|---------|
| `bash` | Yes | Shell execution (timeout default 30s) |
| `bash.output` | No | Poll background shell by ID |
| `bash.kill` | No | Terminate background shell |
| `fs.read` | Yes | Read file (offset/limit by line) |
| `fs.write` | Yes | Write file (auto-mkdir) |
| `fs.edit` | Yes | Find-and-replace (first occurrence) |
| `skill` | Yes | Load skill by name (allowlist check) |
| `tape.info` | Yes | Tape stats: entries, anchors, tokens |
| `tape.search` | Yes | Keyword search in tape |
| `tape.reset` | Yes | Wipe tape (optional archive) |
| `tape.handoff` | Yes | Create named checkpoint anchor |
| `tape.anchors` | Yes | List all anchors |
| `decision.set` | Yes | Record decision (max 500 chars) |
| `decision.list` | Yes | List active decisions |
| `decision.remove` | Yes | Revoke by index |
| `web.fetch` | Yes | HTTP GET (timeout 10s) |
| `subagent` | Yes | **Stub** — acknowledges only |
| `help` | No | List internal commands |
| `quit` | Yes | Placeholder ("ok") |
| `sidecar` | Yes | Proxy to external sidecar plugin |

#### Tape Storage (`builtin/store.rs`)

| Type | Purpose |
|------|---------|
| `FileTapeStore` | JSONL files under `~/.eli/tapes/` |
| `ForkTapeStore` | In-memory fork with merge-on-exit |
| `TapeFile` | Lazy-read cache + incremental append |

| Function | Purpose |
|----------|---------|
| `ForkTapeStore::fork(tape, callback)` | Scoped in-memory fork |
| `FileTapeStore::new(dir)` | Create directory + store |
| `TapeFile::read()` | Lazy load, cache, handle growth |
| `TapeFile::append(entry)` | Write JSONL line |

#### Other Subsystems

| Module | Key Functions |
|--------|--------------|
| **PromptBuilder** | `build(settings, text, state, skills, ...)` — assemble system prompt with priority-based truncation (cap 32K, identity never removed) |
| **SmartRouter** | `classify(content) → Option<RouteDecision>` — 15 greeting triggers × 5 responses |
| **SkillMatcher** | `match_skills(skills, context)` — score = intent×0.6 + tool×0.25 + keyword×0.15; threshold 0.3 |
| **ShellManager** | `start(cmd)`, `get(id)`, `terminate(id)`, `wait_closed(id)` — background process manager |
| **TapeViewer** | axum server: `GET /`, `/api/tapes`, `/api/tapes/{name}`, `/api/tapes/{name}/info`, `/api/tapes/{name}/context` |
| **ModelSpecs** | 87-entry table: `infer_context_window(model)`, `infer_max_output_tokens(model)` |
| **Config** | `EliConfig::load()` — config.toml + legacy JSON migration; `AgentSettings::from_env()` |

### 3.5 CLI Entry Points

| Command | Mode | Description |
|---------|------|-------------|
| `eli chat` | Interactive | REPL via CliChannel |
| `eli run "prompt"` | One-shot | Single message, exit |
| `eli gateway` | Multi-channel | Telegram + Webhook + Sidecar |
| `eli login` | Setup | OAuth flows (Copilot, Codex) |
| `eli model` | Config | Select model |
| `eli use` | Config | Switch profile |
| `eli status` | Info | Show current config |
| `eli hooks` | Debug | List registered hooks |
| `eli tape` | Debug | Web UI for tape inspection |
| `eli decisions` | Manage | List/manage active decisions |

### 3.6 Configuration

| Source | Priority | Location |
|--------|----------|----------|
| Env vars | Highest | `ELI_MODEL`, `ELI_API_KEY`, `ELI_{PROVIDER}_API_KEY`, etc. |
| `.env` file | High | Project root (via dotenvy) |
| config.toml | Medium | `~/.eli/config.toml` (profiles) |
| Model specs | Lowest | Built-in 87-entry table |

### 3.7 Workspace Dependencies

| Dep | Version | Used By | Notes |
|-----|---------|---------|-------|
| tokio | 1 (full) | Both | **Overly broad** — compiles all 35+ features |
| serde/serde_json | 1 | Both | Core serialization |
| reqwest | 0.12 | Both | HTTP client |
| teloxide | 0.13 | eli | Telegram (heavy) |
| axum | 0.8 | eli | HTTP server |
| clap | 4 | eli | CLI |
| ratatui | 0.29 | eli | Terminal UI |
| crossterm | 0.28 | eli | Terminal control |

**Potentially unused deps**: `fuzzy-matcher` 0.3, `glob` 0.3, `which` 8.0.2 (eli); `schemars` 0.8 (conduit)

---

## 4. Critical Issues (P0/P1)

### P0 — Production Panics

#### 4.1 `panic!()` in conduit provider adapters

**Location**: `crates/conduit/src/providers/openai.rs:30`
```rust
panic!("openai adapter does not support messages transport")
```

**Location**: `crates/conduit/src/core/execution.rs:880, 898`
```rust
panic!("Expected Single key config")
panic!("Expected Single base config")
```

**Impact**: Process crash on invalid internal state. These are reachable if a provider is misconfigured.
**Fix**: Replace with `Err(ConduitError { kind: Config, ... })`.

#### 4.2 Unsafe pointer cast in MiddlewareChain

**Location**: `crates/eli/src/tool_middleware.rs:127-131`
```rust
unsafe { &*(self as *const Self) }
```

**Impact**: Unsound lifetime extension across await points. If the Arc holding the CircuitBreaker is dropped while a tool execution is in-flight, this is undefined behavior.
**Fix**: Clone the Arc instead of raw pointer cast.

### P1 — Data Safety

#### 4.3 `.env` secrets in repository

**Location**: `/Users/bytedance/code/eli/.env`
Contains real `ELI_TELEGRAM_TOKEN` and API keys. Must verify `.gitignore` blocks this.

#### 4.4 No session ID validation

**Location**: `crates/eli/src/framework.rs` (resolve_session fallback)
Accepts arbitrary strings as session IDs — potential log injection or path traversal (tape files derived from session ID via MD5).

#### 4.5 Tape viewer has no authentication

**Location**: `crates/eli/src/builtin/tape_viewer.rs`
Anyone with network access can read all conversation tapes. No auth, no bind-to-localhost enforcement.

#### 4.6 No response size limits on web.fetch / fs.read

**Location**: `crates/eli/src/builtin/tools.rs`
- `web.fetch`: No max response body size — OOM on large responses
- `fs.read`: No file size limit — could load multi-GB files

#### 4.7 InMemoryTapeStore unbounded growth

**Location**: `crates/conduit/src/tape/store.rs`
No eviction policy. Long-running agents accumulate entries forever → memory leak.

---

## 5. Major Issues (P2)

### 5.1 Inconsistent panic safety across hooks

Hooks 1, 2, 4 (resolve_session, load_state, run_model) abort the chain on panic. All others catch and skip. This inconsistency is confusing and undocumented.

### 5.2 State merge priority is surprising

`load_state` iterates plugins forward but inserts with `.or_insert()` — meaning **first** plugin wins, not last-registered. This contradicts the "last-registered wins" pattern used by first-result hooks.

### 5.3 ChannelManager busy-poll for task cleanup

**Location**: `crates/eli/src/channels/manager.rs:364-405`
Polls `task.is_finished()` in a loop with `yield_now()`. Under high concurrency, this wastes CPU. Should use `tokio::select!` on JoinHandle.

### 5.4 Orphan message pruning drops entire assistant messages

**Location**: `crates/conduit/src/core/message_norm.rs:134`
If any tool_call in an assistant message lacks a matching result, the **entire** assistant message is dropped (including text content and other valid tool calls).

### 5.5 Subagent tool is a stub

**Location**: `crates/eli/src/builtin/tools.rs` — `subagent` tool only echoes prompt, no actual sub-agent isolation.

### 5.6 Shell manager never cleans up exited shells

**Location**: `crates/eli/src/builtin/shell_manager.rs`
Exited shell entries remain in HashMap forever → memory leak in long-running gateway.

### 5.7 Unused dependencies

4 deps appear unused: `fuzzy-matcher`, `glob`, `which` (eli); `schemars` (conduit). Adds compile time and attack surface.

### 5.8 No `.env.example` template

Users have no reference for required environment variables.

### 5.9 Telegram polling can hang

**Location**: `crates/eli/src/channels/telegram.rs`
`update_listeners::polling_default()` has no timeout. If Telegram API hangs, the dispatcher won't respond to CancellationToken until the request completes.

### 5.10 Dead code: sync TapeManager

**Location**: `crates/conduit/src/llm/mod.rs:291` — `#[allow(dead_code)] tape: TapeManager`
Stored but never used; only AsyncTapeManager is active.

---

## 6. Minor Issues (P3)

| # | Issue | Location |
|---|-------|----------|
| 6.1 | `ChannelMessage::from_batch()` panics on empty batch | `channels/message.rs:215` |
| 6.2 | Hard-coded Anthropic beta headers will go stale | `core/client_registry.rs:169` |
| 6.3 | Tool result truncation thresholds hard-coded (16KB/400KB) | `tape/context.rs:101-158` |
| 6.4 | No `fsync()` on tape writes — data loss on crash | `builtin/store.rs` |
| 6.5 | `tokio::full` feature compiles unnecessary subsystems | `Cargo.toml` |
| 6.6 | No feature flags in eli — can't build without teloxide/ratatui | `crates/eli/Cargo.toml` |
| 6.7 | Error classification uses fragile text-signature heuristics | `core/error_classify.rs` |
| 6.8 | Model specs table is hand-maintained (87 entries) | `builtin/model_specs.rs` |
| 6.9 | Silent media download failure in Telegram | `channels/telegram.rs:320-348` |
| 6.10 | `debug_assert_eq!` in Anthropic adapter (no-op in release) | `providers/anthropic.rs:14` |
| 6.11 | No exponential backoff for sidecar health check (fixed 1s × 15) | `cli/gateway.rs:199-209` |
| 6.12 | API keys may appear in verbose error logs | `core/error_classify.rs:93` |
| 6.13 | No cancellation token integration in conduit (requests run to completion) | `core/execution.rs` |
| 6.14 | Responses parser (OpenAI) may be incomplete for tool calls | `clients/parsing/responses.rs` |
| 6.15 | Config file has no locking — concurrent read/write race | `builtin/config.rs` |
| 6.16 | `experience/` directory is empty (no error/win logs captured yet) | `docs/experience/` |

---

## 7. Metrics & Health

### Code Size

| Component | Rust LOC | Files |
|-----------|----------|-------|
| conduit | ~7,500 | 30 |
| eli framework | ~4,900 | 12 |
| eli builtin | ~6,400 | 13 |
| eli channels | ~2,500 | 7 |
| eli CLI | ~1,500 | 9 |
| **Total** | **~18,800** | **71** |

### Test Coverage

| Crate | Unit Tests | Integration Tests |
|-------|-----------|------------------|
| conduit | 345 | 0 |
| eli | ~125 | 0 |
| **Total** | **~470** | **0** |

### Dependency Footprint

| Metric | Value |
|--------|-------|
| Direct deps (conduit) | 21 |
| Direct deps (eli) | 35 |
| Transitive deps | ~350 |
| Cargo.lock entries | 3,608 lines |

### Architecture Health Score

| Dimension | Score | Notes |
|-----------|-------|-------|
| Modularity | **8/10** | Clean two-crate separation, hook-based extensibility |
| Error handling | **5/10** | 3 production panics, inconsistent hook panic policy |
| Safety | **6/10** | Unsafe code in middleware, no size limits on I/O tools |
| Testability | **7/10** | Good unit test count, but 0 integration tests |
| Configuration | **8/10** | Clean env var hierarchy, profile system |
| Documentation | **6/10** | Good CLAUDE.md/AGENTS.md, but empty experience logs |
| Dependency hygiene | **6/10** | 4 unused deps, overly broad tokio features |
| **Overall** | **6.6/10** | Solid architecture with specific safety/correctness gaps |

---

*End of landscape. Priority: fix P0 panics + unsafe code first, then P1 data safety issues.*
