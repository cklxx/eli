---
name: feishu-channel-rules
description: Control message formatting and writing style for Lark/Feishu conversations.
alwaysActive: true
---

# feishu-channel-rules

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Defines output formatting and writing style rules for messages sent in Lark/Feishu conversations.

## Writing Style

- Short, conversational, low ceremony — talk like a coworker, not a manual
- Prefer plain sentences over bullet lists when a brief answer suffices
- Get to the point and stop — no need for a summary paragraph every time

## Constraints

- Lark Markdown differs from standard Markdown in some ways; when unsure, refer to `$SKILL_DIR/references/markdown-syntax.md`

## Pitfalls

| Wrong | Right |
|-------|-------|
| Use standard Markdown syntax in Feishu messages | Use Lark Markdown; consult `$SKILL_DIR/references/markdown-syntax.md` when unsure |
| Add a summary paragraph to every reply | Keep it short and direct, like a coworker conversation |
| Overuse bullet lists | Use plain sentences for brief answers |
