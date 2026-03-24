# eli-sidecar

eli-sidecar 将 [OpenClaw](https://github.com/nicepkg/openclaw) 渠道插件（飞书、微信、钉钉等）的能力暴露给外部 AI agent。支持两种接入方式：

- **MCP (stdio)**：Claude Code、Cursor 等 MCP 客户端直接使用
- **HTTP**：自建 agent、脚本、任意语言调用

---

## 接入方式一：MCP（推荐）

适用于所有支持 [Model Context Protocol](https://modelcontextprotocol.io) 的客户端。

### 前置条件

1. Node.js >= 18
2. 飞书开发者后台创建应用，获取 App ID 和 App Secret
3. 应用开启所需权限（日历、任务、消息等）

### Claude Code

编辑 `~/.claude/settings.json`，在 `mcpServers` 中添加：

```json
{
  "mcpServers": {
    "eli-sidecar": {
      "command": "npx",
      "args": ["-y", "eli-sidecar", "--mcp"],
      "env": {
        "SIDECAR_FEISHU_APP_ID": "cli_a]xxx",
        "SIDECAR_FEISHU_APP_SECRET": "实际的 app secret"
      }
    }
  }
}
```

保存后重启 Claude Code。运行 `/mcp` 确认 `eli-sidecar` 状态为 running，工具列表中应包含 `feishu_calendar_event`、`send_message` 等。

### Cursor

在项目根目录创建 `.cursor/mcp.json`：

```json
{
  "mcpServers": {
    "eli-sidecar": {
      "command": "npx",
      "args": ["-y", "eli-sidecar", "--mcp"],
      "env": {
        "SIDECAR_FEISHU_APP_ID": "cli_xxx",
        "SIDECAR_FEISHU_APP_SECRET": "实际的 app secret"
      }
    }
  }
}
```

### 其他 MCP 客户端

启动命令为 `npx -y eli-sidecar --mcp`，transport 为 stdio。

### MCP 模式下可用的工具

启动后会自动暴露以下工具：

**内置元工具：**

| 工具 | 参数 | 说明 |
|------|------|------|
| `list_channels` | 无 | 列出已连接的渠道及工具分组。**首次使用时先调此工具**了解有哪些渠道和工具可用 |
| `send_message` | `channel`, `to`, `text`, `account_id?` | 向指定渠道发送消息 |

**飞书插件工具（安装 `@larksuite/openclaw-lark` 后自动注册）：**

| 工具 | 说明 |
|------|------|
| `feishu_get_user` | 获取用户信息 |
| `feishu_search_user` | 搜索员工 |
| `feishu_calendar_event` | 创建/查询/更新日程 |
| `feishu_calendar_freebusy` | 查询忙闲 |
| `feishu_task_task` | 管理任务 |
| `feishu_im_user_message` | 发送消息 |
| `feishu_chat` | 管理群聊 |
| `feishu_bitable_app_table_record` | 多维表格记录 |
| `feishu_fetch_doc` | 获取文档内容 |
| `feishu_create_doc` | 创建文档 |
| ...共 35 个工具 | 完整列表通过 `list_channels` 获取 |

### MCP 工具调用示例

#### send_message

```json
{
  "channel": "feishu",
  "to": "user:ou_xxxxxxxxxxxx",
  "text": "你好，这是来自 agent 的消息"
}
```

- `to` 的格式取决于渠道。飞书私聊用 `user:ou_xxx`（user open_id），群聊用 `oc_xxx`（chat_id）
- `account_id` 可选，默认 `"default"`，对应 sidecar 配置中的 account

#### 调用飞书工具（需要 OAuth 上下文的场景）

部分工具（如日历、任务）需要以应用身份执行，传 `_channel` 启用认证包裹：

```json
{
  "_channel": "feishu",
  "action": "list",
  "start_time": "2026-03-24T00:00:00+08:00",
  "end_time": "2026-03-25T00:00:00+08:00"
}
```

`_channel`、`_account_id`、`_session_id` 是元参数，传递给 sidecar 做认证路由，不会传入实际工具。

如果工具执行报权限错误，检查：
1. 飞书应用是否开启了对应 API 的权限
2. 是否传了 `_channel: "feishu"`

---

## 接入方式二：HTTP

适用于自建 agent、Python/Go/任意语言脚本。

### 启动

```bash
# 安装
npm install eli-sidecar @larksuite/openclaw-lark

# 启动（默认端口 3101）
npx eli-sidecar

# 或通过环境变量配置
SIDECAR_FEISHU_APP_ID=cli_xxx SIDECAR_FEISHU_APP_SECRET=xxx npx eli-sidecar
```

### API 端点

基础 URL：`http://localhost:3101`

#### GET /health

返回状态和已注册的渠道、工具列表。

```bash
curl http://localhost:3101/health
```

```json
{
  "status": "ok",
  "channels": ["feishu", "openclaw-weixin"],
  "tools": ["feishu_get_user", "feishu_calendar_event", "..."]
}
```

#### GET /tools

返回所有工具的名称、描述、JSON Schema 参数定义、工具分组。

```bash
curl http://localhost:3101/tools
```

```json
[
  {
    "name": "feishu_search_user",
    "description": "搜索员工信息（通过关键词搜索姓名、手机号、邮箱）。",
    "parameters": {
      "type": "object",
      "required": ["query"],
      "properties": {
        "query": { "type": "string", "description": "搜索关键词" },
        "page_size": { "type": "integer", "minimum": 1, "maximum": 200 }
      }
    },
    "group": "feishu-search"
  }
]
```

`group` 字段可用于按功能分组展示工具（如 `feishu-calendar`、`feishu-task`）。

#### POST /tools/:name

执行指定工具。

```bash
curl -X POST http://localhost:3101/tools/feishu_search_user \
  -H "Content-Type: application/json" \
  -d '{"params": {"query": "张三"}}'
```

请求体字段：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `params` | object | 是 | 工具参数，schema 由 `GET /tools` 返回 |
| `channel` | string | 否 | 渠道 ID（如 `"feishu"`）。传入后工具以该渠道的应用凭据执行，解决 OAuth 认证问题 |
| `session_id` | string | 否 | 使用已有用户会话的认证上下文（格式：`channel:accountId:chatId`） |
| `account_id` | string | 否 | 账户 ID，默认 `"default"` |
| `description` | string | 否 | 用户可见的进度文本（有会话时推送到渠道） |

响应：工具返回的 JSON（格式由具体工具决定）。

错误响应：

```json
// 404 — 工具不存在
{"error": "tool \"xxx\" not found"}

// 500 — 工具执行失败
{"content": [{"type": "text", "text": "Error: ..."}]}
```

#### POST /send

向渠道发送消息。比 `/outbound` 更简单，不需要了解内部路由。

```bash
curl -X POST http://localhost:3101/send \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "feishu",
    "to": "user:ou_xxxxxxxxxxxx",
    "text": "Hello!"
  }'
```

请求体字段：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `channel` | string | 是 | 渠道 ID |
| `to` | string | 是 | 接收方。飞书私聊 `user:ou_xxx`，群聊 `oc_xxx` |
| `text` | string | 是 | 消息内容（纯文本或 Markdown） |
| `account_id` | string | 否 | 账户 ID，默认 `"default"` |

成功返回 `{"ok": true, ...}`，失败返回 `{"error": "..."}` + 对应 HTTP 状态码。

#### POST /notify

向已有会话发送进度通知。需要 `session_id`（来自先前的入站消息）。

```bash
curl -X POST http://localhost:3101/notify \
  -H "Content-Type: application/json" \
  -d '{"session_id": "feishu:default:ou_xxx", "text": "正在处理..."}'
```

#### HTTP 认证

设置环境变量 `ELI_SIDECAR_TOKEN` 启用 Bearer token 认证（`/health` 除外）：

```bash
ELI_SIDECAR_TOKEN=my-secret npx eli-sidecar
```

请求时携带：

```
Authorization: Bearer my-secret
```

### Python 接入示例

```python
import requests

SIDECAR = "http://localhost:3101"

# 1. 查看有哪些工具
tools = requests.get(f"{SIDECAR}/tools").json()
for t in tools:
    print(f"{t['name']:40s} {t['group']:20s} {t['description'][:60]}")

# 2. 搜索用户
result = requests.post(f"{SIDECAR}/tools/feishu_search_user", json={
    "params": {"query": "张三"},
    "channel": "feishu",
}).json()

# 3. 发消息
requests.post(f"{SIDECAR}/send", json={
    "channel": "feishu",
    "to": "user:ou_xxxxxxxxxxxx",
    "text": "任务已完成",
})
```

---

## 接入方式三：Node.js API

### MCP 模式（嵌入到你的 agent 进程中）

```ts
import { createMcpSidecar } from "eli-sidecar";

// 启动 MCP server (stdio)。console 输出自动重定向到 stderr。
await createMcpSidecar();
```

### HTTP 模式

```ts
import { createSidecar } from "eli-sidecar";

const sidecar = await createSidecar({
  plugins: ["@larksuite/openclaw-lark"],
  channels: {
    feishu: {
      appId: "cli_xxx",
      appSecret: "xxx",
      domain: "feishu",
      accounts: { default: { appId: "cli_xxx", appSecret: "xxx", domain: "feishu" } },
    },
  },
});

// 直接调用工具（不走 HTTP）
const result = await sidecar.callTool("feishu_search_user", { query: "张三" });

await sidecar.stop();
```

---

## 配置

### 环境变量

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `SIDECAR_ELI_URL` | eli webhook 地址（HTTP 模式下的消息转发目标） | `http://127.0.0.1:3100` |
| `SIDECAR_PORT` | HTTP 服务端口 | `3101` |
| `ELI_SIDECAR_TOKEN` | HTTP Bearer token 认证 | 不启用 |

渠道凭据通过 `SIDECAR_{CHANNEL}_{KEY}` 模式传入，自动映射到 `channels.{channel}.accounts.default.{key}`：

| 变量 | 映射到 |
|------|--------|
| `SIDECAR_FEISHU_APP_ID` | `channels.feishu.accounts.default.app_id` |
| `SIDECAR_FEISHU_APP_SECRET` | `channels.feishu.accounts.default.app_secret` |
| `SIDECAR_FEISHU_DOMAIN` | `channels.feishu.accounts.default.domain` |
| `SIDECAR_WEIXIN_APP_ID` | `channels.weixin.accounts.default.app_id` |

### sidecar.json

在工作目录下放置 `sidecar.json` 可进行完整配置：

```json
{
  "eli_url": "http://127.0.0.1:3100",
  "port": 3101,
  "plugins": ["@larksuite/openclaw-lark"],
  "channels": {
    "feishu": {
      "appId": "cli_xxx",
      "appSecret": "xxx",
      "domain": "feishu",
      "accounts": {
        "default": {
          "appId": "cli_xxx",
          "appSecret": "xxx",
          "domain": "feishu"
        }
      }
    }
  }
}
```

各字段说明：

| 字段 | 类型 | 说明 |
|------|------|------|
| `eli_url` | string | eli 的 webhook 地址。仅 HTTP 模式下需要（MCP 模式不用） |
| `port` | number | HTTP 服务端口 |
| `plugins` | string[] | OpenClaw 插件包名列表。留空则自动扫描 node_modules |
| `channels` | object | 各渠道配置。key 为渠道 ID（如 `feishu`），value 包含凭据和账户 |

### 插件自动发现

不配置 `plugins` 数组时，sidecar 自动扫描 `node_modules` 中 `package.json` 含 `openclaw` 字段的包并加载。

---

## 添加渠道插件

```bash
npm install @larksuite/openclaw-lark        # 飞书
npm install @tencent-weixin/openclaw-weixin  # 微信
```

安装后 sidecar 自动加载，该插件注册的所有工具立即可用。

多插件配置：

```json
{
  "plugins": [
    "@larksuite/openclaw-lark",
    "@tencent-weixin/openclaw-weixin"
  ],
  "channels": {
    "feishu": { "appId": "...", "appSecret": "...", "accounts": { "default": { "..." } } },
    "weixin": { "accounts": { "default": { "..." } } }
  }
}
```

---

## 常见问题

**Q: 工具执行报 401 / 权限错误**
A: 检查飞书应用是否开启了对应 API 的权限范围，并确认 `appId` / `appSecret` 正确。HTTP 模式下调用 `/tools/:name` 时传 `"channel": "feishu"` 启用 OAuth 上下文。

**Q: MCP 模式下 Claude Code 看不到工具**
A: 运行 `/mcp` 查看状态。确认 `npx -y eli-sidecar --mcp` 可以在终端正常启动（stderr 应输出 `[mcp] loaded: N channel(s), M tool(s)`）。

**Q: 如何知道 `to` 字段该填什么**
A: 先调用 `feishu_search_user` 搜索用户获取 `open_id`（格式 `ou_xxx`），然后私聊时 `to` 填 `user:ou_xxx`。群聊时 `to` 填群的 `chat_id`（格式 `oc_xxx`）。

**Q: MCP 模式和 HTTP 模式可以同时用吗**
A: 不可以。`--mcp` 启动 stdio 模式，不启动 HTTP 服务。需要 HTTP 时去掉 `--mcp`。

---

## License

Apache-2.0
