# Telegram

Telegram is the builtin remote channel adapter in current core Eli.

## Configuration

Environment variables are read by `TelegramSettings` (`src/eli/channels/telegram.py`).

Required:

```bash
ELI_TELEGRAM_TOKEN=123456:token
```

Optional allowlists (comma-separated):

```bash
ELI_TELEGRAM_ALLOW_USERS=123456789,your_username
ELI_TELEGRAM_ALLOW_CHATS=123456789,-1001234567890
```

Optional proxy:

```bash
ELI_TELEGRAM_PROXY=http://127.0.0.1:7890
```

## Message Behavior

- Session id is `telegram:<chat_id>`.
- `/start` is handled by builtin channel logic.
- `/eli ...` is accepted and normalized to plain prompt content.
- Non-command messages are ingested; active/follow-up behavior is decided by channel filter metadata plus debounce handling.

## Outbound Behavior

- Outbound is sent back to Telegram chat via bot API.
- Empty outbound text is ignored.
- If outbound content is JSON, the `"message"` field is used when present.

## Access Control

- If `ELI_TELEGRAM_ALLOW_CHATS` is set, non-listed chats are ignored.
- If `ELI_TELEGRAM_ALLOW_USERS` is set, non-listed users are denied.
- In group chats, keep allowlists strict for production bots.
