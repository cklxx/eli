# eli-sidecar

OpenClaw plugin bridge for [eli](https://github.com/cklxx/eli). Load any OpenClaw channel plugin (Feishu, DingTalk, Discord, Slack, ...) and bridge it to eli over HTTP.

## Usage

### With eli (automatic)

```bash
eli gateway --enable-channel webhook
```

Eli auto-starts the sidecar. First run prompts for credentials.

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
| `GET /tools` | List tool schemas (JSON) |
| `POST /tools/:name` | Execute a tool |
| `POST /outbound` | Receive eli replies (internal) |

## License

Apache-2.0
