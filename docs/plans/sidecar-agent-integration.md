# Agent 接入 Sidecar 完整流程

## 架构总览

```
┌─────────────────────────────────────────────────────────────┐
│  OpenClaw Channel Plugin (Feishu / WeChat / DingTalk / …)   │
│  npm package, 实现 ChannelPlugin 接口                        │
└──────────┬──────────────────────────────────┬────────────────┘
           │ gateway.startAccount()           │ outbound.sendText()
           ▼                                  ▲
┌──────────────────────────────────────────────────────────────┐
│  Sidecar (Node.js, port 3101)                                │
│                                                              │
│  runtime.ts   — plugin 加载, channel 生命周期, SKILL.md 安装  │
│  bridge.ts    — HTTP 路由: /outbound, /tools/:name, /notify  │
│  api.ts       — OpenClawPluginApi 实现 (传给 plugin.register)│
│  envelope.ts  — InboundEnvelope ↔ EliChannelMessage 转换     │
└──────┬────────────────┬──────────────────────┬───────────────┘
       │ POST /inbound  │ POST /outbound       │ POST /tools/:name
       ▼                │                      ▲
┌──────────────────────────────────────────────────────────────┐
│  Eli (Rust, port 3100)                                       │
│                                                              │
│  webhook channel  — 接收 /inbound                            │
│  framework.rs     — turn pipeline 处理消息                    │
│  agent.rs         — LLM 调用, 发现 SKILL.md + 工具            │
│  tools.rs         — sidecar bridge tool (tool_sidecar)       │
│  gateway.rs       — 启动 sidecar 进程 + 等待 health check     │
└──────────────────────────────────────────────────────────────┘
```

---

## 一、消息入站流程 (Inbound)

### 1. Channel Plugin 收到消息

Plugin 的 `gateway.startAccount(ctx)` 启动后，平台消息通过 webhook/websocket 到达 plugin。
Plugin 内部调用 `runtime.channel.reply.dispatchReplyFromConfig(params)`。

### 2. Sidecar 拦截 → 转发给 Eli

`runtime.ts:buildPluginRuntime()` 中 `dispatchReplyFromConfig` 被替换：

```
runtime.ts:253  dispatchReplyFromConfig(params)
  → 提取 ctx 字段: SenderId, To, ChatType, Body, AccountId ...
  → 构建 SessionContext 存入 sessionContexts Map (30分钟TTL)
  → 构建 InboundEnvelope
  → 异步启动 typing indicator (beginPendingTyping)
  → sendToEli(envelope)
```

### 3. sendToEli → POST Eli webhook

```
bridge.ts:127  sendToEli(envelope)
  → envelopeToEliMessage(envelope)  // 转换格式
  → POST http://127.0.0.1:3100/inbound
```

`EliChannelMessage` 格式:
```json
{
  "session_id": "feishu:default:ou_xxx",
  "channel": "webhook",
  "content": "用户消息文本",
  "chat_id": "ou_xxx",
  "is_active": true,
  "kind": "normal",
  "context": {
    "source_channel": "feishu",
    "account_id": "default",
    "sender_id": "ou_xxx",
    "sender_name": "张三",
    "chat_type": "direct",
    "channel_target": "user:ou_xxx"
  },
  "output_channel": "webhook"
}
```

### 4. Eli Webhook Channel 接收

`webhook.rs` 的 axum handler 接收 JSON，封装为 `ChannelMessage`，通过 mpsc channel 发送给 `gateway.rs` 的主循环。

### 5. Framework 处理

```
gateway.rs:306  framework.process_inbound(inbound)
  → resolve_session → load_state → build_prompt → run_model → save_state
  → render_outbound → dispatch_outbound
```

---

## 二、消息出站流程 (Outbound)

### 1. Framework 输出

`gateway.rs:309` 遍历 `result.outbounds`，构建 `ChannelMessage`，通过 `channel.send(reply)` 发送。

### 2. Webhook Channel 发送

`webhook.rs` 的 `send()` 将 `ChannelMessage` POST 到 sidecar 的 `/outbound`。

### 3. Sidecar 路由回 Channel Plugin

```
bridge.ts:172  POST /outbound
  → parseOutboundTarget(msg) 提取 sourceChannel, accountId, chatId
  → registry.channels.get(sourceChannel) 查找 plugin
  → endPendingTyping() 清除 typing indicator
  → channelPlugin.outbound.sendText({ cfg, to, text, accountId })
```

---

## 三、工具调用流程 (Tool Call)

### 1. LLM 决定调用工具

Agent 在 `agent.rs` 中通过 SKILL.md 发现可用工具。LLM 输出:
```json
{ "name": "sidecar", "arguments": { "tool": "feishu_calendar_event", "params": {...} } }
```

### 2. Rust sidecar bridge tool

```
tools.rs:1141  tool_sidecar()
  → 从 ToolContext 取 session_id
  → POST http://127.0.0.1:3101/tools/feishu_calendar_event
    body: { params: {...}, session_id: "feishu:default:ou_xxx", description: "..." }
  → Bearer token auth (ELI_SIDECAR_TOKEN)
```

### 3. Sidecar 执行工具

```
bridge.ts:264  POST /tools/:name
  → registry.tools.get(name)
  → sessionContexts.get(session_id) → SessionContext
  → emitPluginEvent("before_tool_call", event)
  → notifyToolCall() (发送 typing 状态)
  → channelPlugin.lifecycle.wrapToolExecution(ctx, () => tool.execute(id, params))
    (包裹 LarkTicket 等 OAuth context)
  → emitPluginEvent("after_tool_call", event)
  → 返回 JSON result
```

---

## 四、Skill 发现流程

### 1. Sidecar 安装 SKILL.md

```
runtime.ts:505  installPluginSkills(pluginName)
  → 读取 plugin npm 包的 skills/ 目录
  → 复制 SKILL.md 到 $SIDECAR_SKILLS_DIR/.agents/skills/<skill-name>/SKILL.md
  → 注入 tool calling hint: `sidecar(tool="<tool_name>", params={...})`
  → generateMissingSkills() 为没有 SKILL.md 的 tool group 自动生成
```

### 2. Eli Agent 发现

```
skills.rs  discover_skills()
  → 扫描 .agents/skills/*/SKILL.md
  → 解析 YAML frontmatter (name, description)
  → 用户提到 $skill-name 时展开 SKILL.md 内容到 prompt
```

---

## 五、Gateway 启动流程

```
gateway.rs:212  gateway_command()
  1. dotenvy::dotenv() 加载环境变量
  2. 创建 mpsc channel (tx, rx) + CancellationToken
  3. 启动 Telegram channel (if ELI_TELEGRAM_TOKEN)
  4. find_sidecar_dir() → start_sidecar()
     → ensure_sidecar_config() (首次交互式配置)
     → spawn node start.cjs (设 SIDECAR_ELI_URL, SIDECAR_SKILLS_DIR)
  5. 启动 Webhook channel (port 3100)
  6. wait_for_sidecar("http://127.0.0.1:3101")
     → poll GET /health
     → 写入 SIDECAR_URL 全局变量 (tools.rs 使用)
  7. 主循环: rx.recv() → framework.process_inbound() → 路由 outbound
  8. Ctrl-C → cancel token → kill sidecar process group → stop channels
```

---

## 六、接入新 Agent 需要增加的代码

### A. 新增 Channel Plugin (npm 包)

如果要接入一个新的平台 (如 Discord, Slack), 需要创建一个 OpenClaw 兼容的 npm 包:

```typescript
// @your-org/openclaw-discord/index.ts
import type { OpenClawPluginDefinition } from "eli-sidecar/types";

const plugin: OpenClawPluginDefinition = {
  id: "discord",

  // 可选: 生命周期钩子
  lifecycle: {
    initRuntime(runtime, pluginName) {
      // 注入 runtime (如有需要)
    },
    wrapToolExecution(ctx, fn) {
      // 包裹 OAuth context (如有需要)
      return fn();
    },
    resolveOutboundTarget(context, chatId) {
      // 自定义出站路由
      return chatId;
    },
  },

  register(api) {
    // 注册 channel
    api.registerChannel({
      plugin: {
        meta: { id: "discord", label: "Discord" },
        config: {
          listAccountIds(cfg) { return Object.keys(cfg.channels?.discord?.accounts ?? {}); },
          resolveAccount(cfg, accountId) { return cfg.channels?.discord?.accounts?.[accountId]; },
        },
        capabilities: { chatTypes: ["direct", "group"] },

        // 出站: 发消息到平台
        outbound: {
          async sendText({ cfg, to, text, accountId }) {
            // 调用 Discord API 发送消息
            return { ok: true };
          },
        },

        // 入站: 从平台接收消息
        gateway: {
          async startAccount(ctx) {
            // 启动 Discord bot, 监听消息
            // 收到消息时调用:
            // ctx.runtime.channel.reply.dispatchReplyFromConfig({ ctx: {...}, cfg: ctx.cfg })
          },
          async stopAccount(ctx) {
            // 关闭连接
          },
        },
      },
    });

    // 注册工具 (可选)
    api.registerTool({
      name: "discord_send_embed",
      description: "Send a rich embed to a Discord channel",
      parameters: { /* JSON Schema */ },
      async execute(id, params) {
        return { content: [{ type: "text", text: "Done" }] };
      },
    });
  },
};

export default plugin;
```

### B. 添加 SKILL.md (npm 包内)

```
your-plugin/
  skills/
    discord-embed/
      SKILL.md          ← sidecar 自动复制到 .agents/skills/
    discord-manage/
      SKILL.md
```

SKILL.md 格式:
```markdown
---
name: discord-embed
description: |
  Create and manage Discord rich embeds and messages.
---

## discord_send_embed

Send a rich embed message to a channel.

**Parameters:**
- `channel_id` (string, required): Target channel ID
- `title` (string): Embed title
- `description` (string): Embed body
- `color` (number): Hex color

**Example:**
```json
sidecar(tool="discord_send_embed", params={"channel_id": "123", "title": "Hello"})
```
```

### C. 配置 sidecar.json

```json
{
  "eli_url": "http://127.0.0.1:3100",
  "port": 3101,
  "plugins": [
    "@larksuite/openclaw-lark",
    "@your-org/openclaw-discord"
  ],
  "channels": {
    "feishu": {
      "appId": "...", "appSecret": "...", "domain": "feishu",
      "accounts": { "default": { "appId": "...", "appSecret": "...", "domain": "feishu" } }
    },
    "discord": {
      "accounts": { "default": { "botToken": "..." } }
    }
  }
}
```

### D. Eli 侧不需要改动

关键: **Eli (Rust) 侧零改动**。所有 agent 接入都通过 sidecar:

- **消息**: sidecar → POST /inbound → webhook channel → framework (已有)
- **工具**: agent → `sidecar()` bridge tool → POST /tools/:name (已有)
- **技能**: sidecar 写 SKILL.md → agent 自动发现 (已有)
- **出站**: framework → webhook channel → POST /outbound → sidecar 路由 (已有)

唯一可能需要 Rust 改动的场景:
1. 新增内置 channel (不走 sidecar), 如直接接入 Slack API — 需要在 `channels/` 新增实现
2. 修改 `sidecar` bridge tool 的参数/行为

---

## 七、关键文件索引

| 文件 | 职责 |
|------|------|
| `sidecar/src/runtime.ts` | Plugin 加载, channel 生命周期, typing indicator, SKILL.md 安装 |
| `sidecar/src/bridge.ts` | HTTP 路由 (/outbound, /tools, /notify, /health, /setup) |
| `sidecar/src/api.ts` | `SidecarPluginApi` — 传给 plugin.register() |
| `sidecar/src/types.ts` | 所有 TypeScript 接口定义 |
| `sidecar/src/envelope.ts` | InboundEnvelope ↔ EliChannelMessage 格式转换 |
| `crates/eli/src/builtin/cli/gateway.rs` | Sidecar 进程管理, channel 启动, 主消息循环 |
| `crates/eli/src/builtin/tools.rs:1141` | `tool_sidecar()` — Rust 侧 bridge tool |
| `crates/eli/src/channels/webhook.rs` | Webhook channel (接收 /inbound, 发送 /outbound) |
| `crates/eli/src/skills.rs` | SKILL.md 发现与渲染 |
| `crates/eli/src/builtin/agent.rs` | LLM agent loop, prompt 组装, tool/skill 过滤 |

---

## 八、Session 与认证

- **Session ID 格式**: `{channel}:{accountId}:{chatId}` (e.g. `feishu:default:ou_xxx`)
- **SessionContext**: 存在 sidecar `sessionContexts` Map 中, 30分钟 TTL
- **Tool 认证**: 通过 `lifecycle.wrapToolExecution()` 包裹 (如 Feishu LarkTicket)
- **Sidecar Token**: `ELI_SIDECAR_TOKEN` env var, 用于 Eli↔Sidecar 之间的 Bearer auth
- **跨 session 隔离**: tool 调用必须传 explicit session_id, 不会 fallback 到其他 session
