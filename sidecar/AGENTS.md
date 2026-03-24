# eli-sidecar — Agent Integration

## Quick start

### For Claude Code / Cursor (MCP)

Add to your MCP config (`~/.claude/settings.json` or `.mcp.json`):

```json
{
  "mcpServers": {
    "eli-sidecar": {
      "command": "npx",
      "args": ["-y", "eli-sidecar", "--mcp"],
      "env": {
        "SIDECAR_FEISHU_APP_ID": "cli_xxx",
        "SIDECAR_FEISHU_APP_SECRET": "xxx"
      }
    }
  }
}
```

That's it. All Feishu tools (calendar, tasks, docs, IM, ...) appear as MCP tools. Also available: `list_channels` and `send_message` for direct messaging.

### For eli (built-in bridge)

```bash
npm install eli-sidecar @larksuite/openclaw-lark && npx eli-sidecar
```

Starts on `:3101`, writes SKILL.md to `.agents/skills/`. Eli's `discover_skills()` finds them automatically.

### For custom agents (HTTP)

```python
import requests

SIDECAR = "http://localhost:3101"

# List tools
tools = requests.get(f"{SIDECAR}/tools").json()

# Call a tool
result = requests.post(f"{SIDECAR}/tools/feishu_calendar_event", json={
    "params": {"summary": "Meeting", "start_time": "..."},
    "channel": "feishu",  # optional: enables OAuth context
}).json()

# Send a message
requests.post(f"{SIDECAR}/outbound", json={
    "session_id": "feishu:default:ou_xxx",
    "channel": "webhook",
    "content": "Hello!",
    "chat_id": "ou_xxx",
    "context": {"source_channel": "feishu", "account_id": "default"},
    "output_channel": "webhook",
})
```

### Programmatic (Node.js)

```ts
import { createSidecar, createMcpSidecar } from "eli-sidecar";

// HTTP mode
const sidecar = await createSidecar();
const result = await sidecar.callTool("feishu_calendar_event", { summary: "Meeting" });

// MCP mode (stdio — for embedding in an agent)
await createMcpSidecar();
```

## How it works

1. Sidecar loads OpenClaw channel plugins (auto-discovered from `node_modules`)
2. Exposes tools via MCP (stdio) or HTTP (`/tools/:name`)
3. For eli: also writes `SKILL.md` to `.agents/skills/` for standard discovery

## Config

Credentials via `sidecar.json` or env vars:

```bash
# Feishu / Lark
SIDECAR_FEISHU_APP_ID=cli_xxx
SIDECAR_FEISHU_APP_SECRET=xxx

# WeChat (via openclaw-weixin)
SIDECAR_WEIXIN_APP_ID=xxx
SIDECAR_WEIXIN_APP_SECRET=xxx
```

Full `sidecar.json`:
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
        "default": { "appId": "cli_xxx", "appSecret": "xxx", "domain": "feishu" }
      }
    }
  }
}
```

## MCP tools

In `--mcp` mode, the following meta-tools are always available:

| Tool | Description |
|------|-------------|
| `list_channels` | Show connected channels and available tool groups |
| `send_message` | Send a message to any connected channel |

Plus all plugin-registered tools (e.g. `feishu_calendar_event`, `feishu_im_user_message`, ...).

## HTTP API

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/health` | GET | Status + channel/tool list |
| `/tools` | GET | Tool schemas with group info |
| `/tools/:name` | POST | Execute tool: `{ params, session_id?, channel? }` |
| `/outbound` | POST | Route a message to a channel |
| `/notify` | POST | Send a progress notice: `{ text, session_id }` |
