---
name: reminder-scheduler
description: Schedule one-time and recurring reminders with create, list, cancel, and due-scan operations.
triggers:
  intent_patterns:
    - "提醒|remind|timer|定时|倒计时|schedule|cron|周期任务|闹钟|alarm"
  context_signals:
    keywords: ["提醒", "timer", "schedule", "cron", "周期"]
  confidence_threshold: 0.55
priority: 7
requires_tools: [bash]
max_tokens: 240
cooldown: 30
---

# reminder-scheduler

Unified reminder scheduler covering two capabilities: one-time reminders (delayed trigger) and recurring plans (long-term periodic reminders like weekly retros).

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| Set one-time reminder | `$PYTHON $SKILL_DIR/run.py set_once --delay 30m --task "..."` | `--delay`, `--task` |
| List one-time reminders | `$PYTHON $SKILL_DIR/run.py list_once` | none |
| Cancel one-time reminder | `$PYTHON $SKILL_DIR/run.py cancel_once --id timer-12345` | `--id` |
| Create/update plan | `$PYTHON $SKILL_DIR/run.py upsert_plan --name ... --schedule "..." --task "..."` | `--name`, `--schedule`, `--task` |
| List plans | `$PYTHON $SKILL_DIR/run.py list_plans` | none |
| Scan due plans | `$PYTHON $SKILL_DIR/run.py due_plans` | `--now` (optional) |
| Delete plan | `$PYTHON $SKILL_DIR/run.py delete_plan --name ...` | `--name` or `--id` |
| Advance plan | `$PYTHON $SKILL_DIR/run.py touch_plan --name ... --next-run-at ...` | `--name`/`--id`, `--next-run-at` |

## Usage

```bash
# One-time reminders
$PYTHON $SKILL_DIR/run.py set_once --delay 30m --task "Drink water"
$PYTHON $SKILL_DIR/run.py list_once
$PYTHON $SKILL_DIR/run.py cancel_once --id timer-12345

# Recurring plans
$PYTHON $SKILL_DIR/run.py upsert_plan --name weekly-retro --schedule "0 18 * * 5" --task "Send retro reminder" --next-run-at 2026-03-06T10:00:00Z
$PYTHON $SKILL_DIR/run.py list_plans
$PYTHON $SKILL_DIR/run.py due_plans --now 2026-03-06T10:00:00Z
$PYTHON $SKILL_DIR/run.py delete_plan --name weekly-retro
$PYTHON $SKILL_DIR/run.py touch_plan --name weekly-retro --next-run-at 2026-03-13T10:00:00Z
```

## Parameters

### set_once

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| delay | string | yes | Delay duration (`30s` / `5m` / `2h`) |
| task | string | yes | Reminder content |

### cancel_once

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| id | string | yes | One-time reminder ID |

### upsert_plan

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| name | string | yes | Plan name (unique key) |
| schedule | string | yes | Cron expression |
| task | string | yes | Reminder content |
| next_run_at | string | no | Next execution time (ISO 8601) |
| channel | string | no | Channel, default `lark` |
| enabled | bool | no | Whether enabled, default `true` |
| metadata | object | no | Extension metadata |

### list_plans

No parameters. Returns all recurring plans.

### delete_plan

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| name | string | no | Plan name; provide at least one of `name` or `id` |
| id | string | no | Plan ID; provide at least one of `name` or `id` |

When both `name` and `id` are provided, they must match the same record for deletion to proceed.

### due_plans

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| now | string | no | Current time (ISO 8601); defaults to system UTC time |

### touch_plan

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| name | string | no | Plan name; provide at least one of `name` or `id` |
| id | string | no | Plan ID; provide at least one of `name` or `id` |
| next_run_at | string | no | Update next execution time (ISO 8601) |

When both `name` and `id` are provided, they must match the same record for the update to proceed.
