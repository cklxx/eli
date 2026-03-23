# CLI

`eli` exposes four main commands (`run`, `gateway`, `chat`, `login`) plus two hidden ones (`hooks` for diagnostics, `message` as a compatibility alias for `gateway`).

## `eli run`

Run one inbound message through the full framework pipeline and print outbounds.

```bash
uv run eli run "hello" --channel cli --chat-id local
```

Common options:

- `--workspace/-w`: workspace root, declared once on the top-level CLI and shared by all subcommands
- `--channel`: source channel (default `cli`)
- `--chat-id`: source endpoint id (default `local`)
- `--sender-id`: sender identity (default `human`)
- `--session-id`: explicit session id (default is `<channel>:<chat_id>`)

Comma-prefixed input enters internal command mode:

```bash
uv run eli run ",help"
uv run eli run ",skill name=my-skill"
uv run eli run ",fs.read path=README.md"
```

Unknown comma commands fall back to shell execution:

```bash
uv run eli run ",echo hello-from-shell"
```

## `eli hooks`

Print hook-to-plugin bindings discovered at startup.

```bash
uv run eli hooks
```

`hooks` remains available for diagnostics, but it is hidden from the top-level help.

## `eli gateway`

Start channel listener mode (defaults to all non-`cli` channels).

```bash
uv run eli gateway
```

Enable only selected channels:

```bash
uv run eli gateway --enable-channel telegram
```

`eli message` is kept as a hidden compatibility alias and forwards to the same command implementation.

## `eli chat`

Start an interactive REPL session via the `cli` channel.

```bash
uv run eli chat
uv run eli chat --chat-id local --session-id cli:local
```

## `eli login`

Authenticate with OpenAI Codex OAuth and persist the resulting credentials under `CODEX_HOME` (default `~/.codex`).

```bash
uv run eli login openai
```

Manual callback mode is useful when the local redirect server is unavailable:

```bash
uv run eli login openai --manual --no-browser
```

After login, you can use an OpenAI model without setting `ELI_API_KEY`:

```bash
ELI_MODEL=openai:gpt-5-codex uv run eli chat
```

If the upstream endpoint expects a specific OpenAI-compatible request shape, set `ELI_API_FORMAT`:

- `completion`: legacy completion-style format; default
- `responses`: OpenAI Responses API format
- `messages`: chat-completions-style messages format

```bash
ELI_MODEL=openai:gpt-5-codex ELI_API_FORMAT=responses uv run eli chat
```

## Notes

- `--workspace` is parsed before the subcommand, for example `uv run eli --workspace /repo chat`.
- `run` prints each outbound as:

```text
[channel:chat_id]
content
```
