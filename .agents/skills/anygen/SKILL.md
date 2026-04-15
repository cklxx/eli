---
name: anygen
description: Generate documents, slides, websites, and diagrams via the AnyGen API with progressive discovery and task execution.
triggers:
  intent_patterns:
    - "anygen|ppt|slides|生成文档|docx|storyboard|website|data analysis|smart_draw|diagram"
  context_signals:
    keywords: ["anygen", "slide", "ppt", "doc", "website", "data_analysis", "smart_draw", "storyboard"]
  confidence_threshold: 0.6
priority: 7
requires_tools: [bash]
max_tokens: 260
cooldown: 20
output:
  format: markdown
  artifacts: true
  artifact_type: file
---

# anygen

Wraps the repository-bundled AnyGen CLI adapter into a unified entry point with two commands: `help` for progressive discovery and `task` for task execution via the task-manager module.

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| Top-level overview | `$PYTHON $SKILL_DIR/run.py help` | none |
| List modules | `$PYTHON $SKILL_DIR/run.py help --topic modules` | `--topic` |
| Module details | `$PYTHON $SKILL_DIR/run.py help --topic module --module task-manager` | `--module` |
| Action details | `$PYTHON $SKILL_DIR/run.py help --topic action --module task-manager --action_name create` | `--action_name` |
| One-shot generate | `$PYTHON $SKILL_DIR/run.py task run --operation slide --prompt '...' --output ./out` | `--operation`, `--prompt` |

## Prerequisites

- Environment variable: `ANYGEN_API_KEY=sk-xxx`
- Entry command: `$PYTHON $SKILL_DIR/run.py <command> [subcommand] [--flag value ...]`

## Usage

### Progressive Discovery (recommended order)

```bash
# 1) Top-level overview
$PYTHON $SKILL_DIR/run.py help

# 2) Module list
$PYTHON $SKILL_DIR/run.py help --topic modules

# 3) Module details
$PYTHON $SKILL_DIR/run.py help --topic module --module task-manager

# 4) Action parameters and examples
$PYTHON $SKILL_DIR/run.py help --topic action --module task-manager --action_name create
```

### Task Execution

Supported operations: `chat|slide|doc|storybook|data_analysis|website|smart_draw`

```bash
# Create a task
$PYTHON $SKILL_DIR/run.py task create --operation slide --prompt 'Q2 roadmap deck' --style business

# Check status (one-shot)
$PYTHON $SKILL_DIR/run.py task status --task_id task_xxx

# Poll until complete (with optional auto-download)
$PYTHON $SKILL_DIR/run.py task poll --task_id task_xxx --output ./output

# Download completed task files
$PYTHON $SKILL_DIR/run.py task download --task_id task_xxx --output ./output

# One-shot: create + poll + optional download
$PYTHON $SKILL_DIR/run.py task run --operation doc --prompt 'Technical design for notification service' --output ./output
```

## Constraints

- `task-manager`: fully executable within this skill.
- `finance-report`: guidance available via `help`, but not directly executable in this skill.
- Input accepts `command` or `action` as the top-level command name.
- When `action=create/status/poll/download/run`, it auto-routes to the `task` command.
