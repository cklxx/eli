# eli-sidecar

OpenClaw plugin bridge for [eli](https://github.com/cklxx/eli). Load any [OpenClaw](https://github.com/nicepkg/openclaw) channel plugin (Feishu, DingTalk, Discord, Slack, ...) and bridge channels + tools to eli over HTTP.

## How it works

```
Feishu/DingTalk/Slack/...
    ↕ (OpenClaw plugin)
┌────────────────────────┐
│  eli-sidecar (:3101)   │
│  ├ Plugin loader (jiti) │
│  ├ Channel registry     │
│  ├ Tool registry        │
│  └ HTTP bridge          │
└────────┬───────────────┘
         │ HTTP JSON
         ▼
┌────────────────────────┐
│  eli (:3100)           │
│  Turn pipeline → LLM   │
└────────────────────────┘
```

Tools from plugins are exposed as **skills** with progressive disclosure — the LLM sees grouped summaries, not 35 individual tool schemas. When needed, it activates a skill to get tool details, then calls via the `sidecar` bridge tool.

## Usage

### With eli (automatic)

```bash
eli gateway --enable-channel webhook
```

Eli auto-starts the sidecar. First run prompts for credentials interactively.

### Standalone

```bash
npm install eli-sidecar @larksuite/openclaw-lark
npx eli-sidecar
```

### Programmatic

```ts
import { createSidecar } from "eli-sidecar";

const sidecar = await createSidecar({
  eli_url: "http://localhost:3100",
  port: 3101,
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

// later:
await sidecar.stop();
```

### Auto-discovery

If no `plugins` array is specified, the sidecar scans `node_modules` for packages with an `openclaw` field in their `package.json`.

## Config

Create `sidecar.json` (or set env vars):

| Field | Env | Default | Description |
|-------|-----|---------|-------------|
| `eli_url` | `SIDECAR_ELI_URL` | `http://127.0.0.1:3100` | Eli webhook endpoint |
| `port` | `SIDECAR_PORT` | `3101` | Outbound server port |
| `plugins` | — | auto-discover | OpenClaw plugin packages |
| `channels` | — | `{}` | Per-channel credentials |

## HTTP API

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Status + registered channels/tools |
| `GET /tools` | List tool schemas with group info |
| `POST /tools/:name` | Execute a tool (with LarkTicket context) |
| `POST /outbound` | Receive eli replies (internal) |

## Adding plugins

Install any OpenClaw channel plugin as a dependency:

```bash
npm install @larksuite/openclaw-lark     # Feishu/Lark
npm install openclaw-channel-dingtalk    # DingTalk
npm install openclaw-channel-discord     # Discord
```

The sidecar loads them automatically — no per-plugin adaptation needed.

## License

Apache-2.0
