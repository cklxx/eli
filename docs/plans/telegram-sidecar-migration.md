# Telegram → Sidecar + Plugin Migration

## Goal

Remove `TelegramChannel` from the Rust codebase. Telegram runs as an **openclaw plugin** in the Node sidecar — same pattern as Feishu, Discord, Slack.

## Key Insight

The `openclaw/extensions/telegram` plugin **already exists** in `node_modules` with full support: polling, webhooks, media, groups, threads, access control. We don't need to write a Telegram plugin — just wire it up.

## Current Token Config

`ELI_TELEGRAM_TOKEN` → loaded in `TelegramSettings::from_env()`. The sidecar already supports env var overrides: `SIDECAR_TELEGRAM_BOT_TOKEN` or `channels.telegram.botToken` in `sidecar.json`.

**Migration**: Keep `ELI_TELEGRAM_TOKEN` working by forwarding it to the sidecar as `TELEGRAM_BOT_TOKEN` env var when spawning the sidecar process.

---

## Changes

### 1. Sidecar Config — Register Telegram Plugin

**File**: `sidecar/sidecar.example.json`

Add `"telegram"` to the plugins array (the openclaw runtime auto-discovers `openclaw/extensions/telegram` by its `openclaw.plugin.json` manifest, but it needs to be enabled):

```json
{
  "eli_url": "http://127.0.0.1:3100",
  "port": 3101,
  "plugins": ["@larksuite/openclaw-lark"],
  "channels": {
    "telegram": {
      "botToken": "${TELEGRAM_BOT_TOKEN}",
      "accounts": {
        "default": {
          "botToken": "${TELEGRAM_BOT_TOKEN}"
        }
      }
    }
  }
}
```

The openclaw runtime resolves `TELEGRAM_BOT_TOKEN` from env automatically. No code needed.

### 2. Rust — Forward `ELI_TELEGRAM_TOKEN` to Sidecar

**File**: `crates/eli/src/builtin/cli/gateway.rs`

In `start_sidecar()`, pass the token as an env var to the sidecar process:

```rust
// In the Command::new("node") builder, add:
.env("TELEGRAM_BOT_TOKEN", std::env::var("ELI_TELEGRAM_TOKEN").unwrap_or_default())
```

This preserves the existing `ELI_TELEGRAM_TOKEN` config — users don't need to change anything.

### 3. Rust — Remove TelegramChannel from Gateway

**File**: `crates/eli/src/builtin/cli/gateway.rs`

Remove the entire `// -- Telegram --` block (lines 228–243). The webhook channel + sidecar now handles Telegram.

Update the "no channels configured" error message to remove the `ELI_TELEGRAM_TOKEN` mention — webhook is now the only built-in channel.

Update imports to remove `TelegramChannel` and `TelegramSettings`.

### 4. Rust — Delete telegram.rs

**File**: `crates/eli/src/channels/telegram.rs` — **Delete entirely**

### 5. Rust — Remove Telegram from channels/mod.rs

**File**: `crates/eli/src/channels/mod.rs`

Remove:
```rust
pub mod telegram;
pub use telegram::{TelegramChannel, TelegramSettings};
```

### 6. Rust — Remove teloxide dependency

**File**: `crates/eli/Cargo.toml`

Remove `teloxide` from `[dependencies]`. Check if any other deps were telegram-only (e.g. `teloxide-core`).

### 7. Gateway — Auto-enable Webhook When Telegram Token Present

**File**: `crates/eli/src/builtin/cli/gateway.rs`

Currently, webhook only starts if `should_enable("webhook")`. After this change, if `ELI_TELEGRAM_TOKEN` is set, webhook should auto-start (since Telegram now runs through the sidecar which needs the webhook channel).

Update `gateway_command()`:
```rust
// Webhook is always needed when any sidecar channel is configured
let needs_sidecar = !std::env::var("ELI_TELEGRAM_TOKEN").unwrap_or_default().is_empty()
    || find_sidecar_dir().is_some();

if should_enable("webhook") || needs_sidecar {
    // ... existing webhook + sidecar startup ...
}
```

### 8. Update CLAUDE.md / Docs

Remove Telegram-specific references from channel docs. Update `ELI_TELEGRAM_TOKEN` docs to note it's forwarded to the sidecar.

---

## Migration Path for Users

1. **No config change needed** — `ELI_TELEGRAM_TOKEN` still works, forwarded to sidecar.
2. Users who want more control (multi-account, webhooks, proxy) can configure `sidecar.json` directly with the full openclaw telegram config.
3. `ELI_TELEGRAM_ALLOW_USERS` and `ELI_TELEGRAM_ALLOW_CHATS` move to `sidecar.json` under `channels.telegram.allowFrom` / `channels.telegram.groups`. Document this.

## File Checklist

| Action | File |
|--------|------|
| DELETE | `crates/eli/src/channels/telegram.rs` |
| EDIT | `crates/eli/src/channels/mod.rs` — remove telegram exports |
| EDIT | `crates/eli/src/builtin/cli/gateway.rs` — remove TG block, forward env, auto-enable webhook |
| EDIT | `crates/eli/Cargo.toml` — remove teloxide |
| EDIT | `sidecar/sidecar.example.json` — add telegram config example |
| EDIT | `CLAUDE.md` — update config docs |

## Verification

```bash
cargo build --release          # no telegram.rs, no teloxide
cargo clippy --workspace -- -D warnings
cargo test --workspace

# Manual test:
ELI_TELEGRAM_TOKEN=xxx eli gateway
# → webhook starts, sidecar starts, sidecar picks up TELEGRAM_BOT_TOKEN, telegram works
```
