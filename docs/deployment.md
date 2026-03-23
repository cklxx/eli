# Deployment Guide

This page covers practical Eli deployment paths based on the current repository behavior.

## 1) Prerequisites

- Python 3.12+
- `uv` installed
- a valid model provider key (for example `OPENROUTER_API_KEY`)

Bootstrap:

```bash
git clone https://github.com/eliagent/eli.git
cd eli
uv sync
cp env.example .env
```

Minimum `.env` example:

```bash
ELI_MODEL=openrouter:qwen/qwen3-coder-next
OPENROUTER_API_KEY=sk-or-...
```

## 2) Runtime Modes

Choose one command based on your operation target:

1. Interactive local operator: `uv run eli chat`
2. Channel listener service: `uv run eli gateway`
3. One-shot task execution: `uv run eli run "summarize this repo"`

## 3) Telegram Channel Setup

Telegram configuration and runtime behavior are documented in:

- `docs/channels/telegram.md`

Quick start:

```bash
ELI_TELEGRAM_TOKEN=123456:token uv run eli gateway --enable-channel telegram
```

## 4) Docker Compose

Repository assets:

- `Dockerfile`
- `docker-compose.yml`
- `entrypoint.sh`

Build and run:

```bash
docker compose up -d --build
docker compose logs -f app
```

Current entrypoint behavior:

- if `/workspace/startup.sh` exists, entrypoint tries to run `startup.sh`
- otherwise it starts `eli gateway`

Default mounts in `docker-compose.yml`:

- `${ELI_WORKSPACE_PATH:-.}:/workspace`
- `${ELI_HOME:-${HOME}/.eli}:/data`
- `${ELI_AGENT_HOME:-${HOME}/.agents}:/root/.agents`

Notes:

- Eli runtime data is written under `ELI_HOME` (container default: `/root/.eli`).
- In this compose file, `ELI_HOME` is used as the host bind source for `/data`.
- Do not set `ELI_HOME=/data` directly in `.env` with this compose file, or the host bind source will also become `/data`.
- If you want Eli runtime home to be `/data` in container, split variables first (for example `ELI_HOME_HOST` for host path) and then set `ELI_HOME=/data`.

## 5) Operational Checks

1. Verify process:
   `ps aux | rg "eli (chat|gateway|run)"`
2. Verify model config:
   `rg -n "ELI_MODEL|ELI_API_KEY|ELI_API_BASE|ELI_.*_API_KEY|ELI_.*_API_BASE|OPENROUTER_API_KEY" .env`
3. Verify Telegram settings:
   `rg -n "ELI_TELEGRAM_TOKEN|ELI_TELEGRAM_ALLOW_USERS|ELI_TELEGRAM_ALLOW_CHATS" .env`
4. Verify startup logs:
   `uv run eli gateway --enable-channel telegram`

## 6) Safe Upgrade

```bash
git fetch --all --tags
git pull
uv sync
uv run ruff check .
uv run mypy
uv run pytest -q
```

Then restart your service command.
