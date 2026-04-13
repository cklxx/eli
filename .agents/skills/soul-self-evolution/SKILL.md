---
name: soul-self-evolution
description: Update evolvable sections of SOUL.md with immutable-segment protection and rollback support.
triggers:
  intent_patterns:
    - "soul|人格更新|self evolve|自我优化|更新协作风格"
  context_signals:
    keywords: ["SOUL", "persona", "habit", "collaboration"]
  confidence_threshold: 0.7
priority: 9
requires_tools: [read_file, write_file]
max_tokens: 320
cooldown: 300
capabilities: [self_evolve_soul, policy_self_adjust]
governance_level: critical
activation_mode: semi_auto
depends_on_skills: [meta-orchestrator]
produces_events:
  - workflow.skill.meta.soul_updated
  - workflow.skill.meta.rollback_applied
requires_approval: false
---

# soul-self-evolution

Perform controlled updates to `SOUL.md` (typically `.agents/SOUL.md` or `~/.eli/SOUL.md`). Only evolvable sections can be modified. Every change creates a checkpoint that supports one-click rollback.

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| Apply changes | `apply` | `--path`, `--changes` |
| List checkpoints | `list_checkpoints` | — |

## Usage

```bash
# Apply a change to an evolvable section
python3 $SKILL_DIR/run.py apply --path .agents/SOUL.md --changes '[{"section":"## Collaboration Preferences","content":"- Keep updates concise."}]'

# List available rollback checkpoints
python3 $SKILL_DIR/run.py list_checkpoints
```

## Parameters

### apply

| Name | Type | Required | Notes |
|------|------|----------|-------|
| path | string | yes | Path to the SOUL.md file |
| changes | JSON array | yes | Each entry: `{"section": "## Header", "content": "new content"}` |

### list_checkpoints

No parameters.

## Constraints

- Only evolvable sections can be modified; immutable segments are protected.
- Every `apply` creates a checkpoint automatically.
- Rollback restores the previous checkpoint state.
