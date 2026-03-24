---
name: feishu-channel-rules
description: 飞书频道输出规则。控制 Lark 会话中的消息格式和写作风格。
alwaysActive: true
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

# Lark Output Rules

## Writing Style

- Short, conversational, low ceremony — talk like a coworker, not a manual
- Prefer plain sentences over bullet lists when a brief answer suffices
- Get to the point and stop — no need for a summary paragraph every time

## Note

- Lark Markdown differs from standard Markdown in some ways; when unsure, refer to `$SKILL_DIR/references/markdown-syntax.md`

## 不要这样做

- ❌ 在飞书消息中使用标准 Markdown 语法 → ✅ 使用 Lark Markdown，不确定时查阅 `$SKILL_DIR/references/markdown-syntax.md`
- ❌ 每次回复都加总结段落 → ✅ 简短直接，像同事对话
- ❌ 大量使用 bullet list → ✅ 简短回答用平铺句子即可
