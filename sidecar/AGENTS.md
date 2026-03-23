# eli-sidecar — Agent Integration

## One-line setup

```bash
npm install eli-sidecar @larksuite/openclaw-lark && npx eli-sidecar
```

This installs the Feishu channel plugin, starts the sidecar on `:3101`, and writes SKILL.md files to `.agents/skills/`. Any agent that reads `.agents/skills/` discovers them automatically.

## What happens

1. Sidecar loads OpenClaw channel plugins (auto-discovered from `node_modules`)
2. Writes `SKILL.md` files to `.agents/skills/` — standard skills protocol
3. Agent's `discover_skills()` finds them (feishu-calendar, feishu-task, feishu-im, ...)
4. User mentions `$feishu-calendar` → skill content expands into context
5. LLM reads the skill guide, picks a tool, calls `sidecar(tool="feishu_calendar_event", params={...})`
6. Bridge tool proxies to sidecar's `POST /tools/:name`

## When to trigger

Use when the user asks to connect to Feishu, Lark, DingTalk, or any messaging platform supported by OpenClaw plugins. Examples:

- "帮我接入飞书" → run setup, skills appear, agent can manage calendars/tasks/docs/IM
- "发消息给张三" → activate `$feishu-im` skill, call `feishu_im_user_message`
- "帮我创建一个会议" → activate `$feishu-calendar` skill, call `feishu_calendar_event`
- "查看我的任务" → activate `$feishu-task` skill, call `feishu_task_task`

## Config

Credentials in `sidecar.json` (or env vars `SIDECAR_FEISHU_APP_ID`, `SIDECAR_FEISHU_APP_SECRET`):

```json
{ "channels": { "feishu": { "appId": "cli_xxx", "appSecret": "xxx", "accounts": { "default": { "appId": "cli_xxx", "appSecret": "xxx" } } } } }
```

## HTTP API (for bridge tool)

| Endpoint | Purpose |
|----------|---------|
| `GET /health` | Status check |
| `GET /tools` | List tool schemas |
| `POST /tools/:name` | Execute tool with `{ "params": {...} }` |
