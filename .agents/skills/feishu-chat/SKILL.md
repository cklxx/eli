---
name: feishu-chat
description: "Retrieve the member list of a Feishu group chat. Use when the user wants to see who is in a specific group."
---

# feishu-chat

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Fetches the member list for a specified Feishu group chat.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| View group member list | feishu_chat_members | chat_id |

## Tools

### feishu_chat_members
Retrieves the member list for a specified group chat, running as the user. Returns member information including member ID, name, etc. Note: bot members in the group are not included in the results.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Using feishu_chat_members to search for group chats | Use feishu_chat search from the feishu skill to search chats |
| Guessing a chat_id when you don't know it | First use feishu_chat search from the feishu skill to find the chat and obtain its chat_id |
