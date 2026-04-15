---
name: notebooklm-cli
description: Manage NotebookLM notebooks, sources, queries, reports, and audio studios via a local CLI wrapper.
triggers:
  intent_patterns:
    - "notebooklm|notebook lm|nlm|音频概览|podcast|research notebook"
  context_signals:
    keywords: ["notebooklm", "nlm", "notebook", "source", "podcast", "report"]
  confidence_threshold: 0.6
priority: 7
requires_tools: [bash]
max_tokens: 200
cooldown: 20
enabled: false
disabled_reason: "Depends on the external nlm CLI and NotebookLM auth state; not self-contained in this workspace."
---

# notebooklm-cli

Local CLI wrapper around `nlm` for managing NotebookLM resources. Unified `command/op` interface with structured JSON output.

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| Check auth | `auth check` | — |
| Create notebook | `notebook create` | `--title` |
| List notebooks | `notebook list` | — |
| Add URL source | `source add_url` | `--notebook_id`, `--url` |
| Query a notebook | `query` | `--notebook_id`, `--question` |
| Generate report | `report` | `--notebook_id`, `--confirm true` |
| Audio studio status | `studio status` | `--notebook_id` |
| Get help | `help` | `--topic` |

## Usage

```bash
$PYTHON $SKILL_DIR/run.py <command> [op] [--flag value ...]
```

## Progressive Help

Retrieve contract documentation for LLM or human consumption:

```bash
$PYTHON $SKILL_DIR/run.py help --topic overview      # entry point, env vars, command summary
$PYTHON $SKILL_DIR/run.py help --topic schema         # full command contracts (machine-readable)
$PYTHON $SKILL_DIR/run.py help --topic progressive    # overview + per-command contract chain
$PYTHON $SKILL_DIR/run.py help --topic source --include_cli true   # include raw CLI help
```

## Input Contract

- `command`: `help | auth | notebook | source | query | report | studio | raw`
- `op`: sub-operation within a command; omit for default op (see `help/schema`).
- Legacy aliases (`action`, `*_action`) still work; prefer `command/op` in new code.
- Unified response fields: `success, command, exit_code, stdout, stderr, hints, error?`

## Commands

| Command | Operations | Notes |
|---------|-----------|-------|
| `auth` | login, check, profile_delete | `profile_delete` requires `confirm=true` |
| `notebook` | list, create, get, describe, rename, query, delete | `delete` requires confirmation |
| `source` | list, add_*, get, describe, content, rename, delete | `delete` requires confirmation |
| `query` | — | Shortcut for notebook query |
| `report` | create (default) | Requires `--confirm true`; supports `format`, `prompt`, `language`, `source_ids` |
| `studio` | status, rename, delete | `delete` requires confirmation |
| `raw` | — | Pass-through to `nlm` argv; `nlm chat start` is forbidden |

## Minimal End-to-End Example

```bash
$PYTHON $SKILL_DIR/run.py auth check
$PYTHON $SKILL_DIR/run.py notebook create --title 'NLM E2E'
$PYTHON $SKILL_DIR/run.py source add_url --notebook_id '<nb-id>' --url https://example.com/article
$PYTHON $SKILL_DIR/run.py query --notebook_id '<nb-id>' --question 'Summarize 3 key conclusions'
$PYTHON $SKILL_DIR/run.py report --notebook_id '<nb-id>' --confirm true
$PYTHON $SKILL_DIR/run.py studio status --notebook_id '<nb-id>'
```

## Constraints

- Deletion requires explicit confirmation: `confirm=true`.
- Interactive `nlm chat start` is forbidden.
- On auth failure, run `auth login` first, then retry the business command.
