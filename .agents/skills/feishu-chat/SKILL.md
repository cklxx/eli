---
name: feishu-chat
description: "获取飞书群组的成员列表。当用户需要查看某个群里有哪些人时使用。"
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 快速索引

| 用户意图 | 工具 | 关键参数 |
|---------|------|---------|
| 查看群成员列表 | feishu_chat_members | chat_id |

## 工具说明

### feishu_chat_members
以用户的身份获取指定群组的成员列表。返回成员信息，包含成员 ID、姓名等。注意：不会返回群组内的机器人成员。

## 不要这样做

- ❌ 搜索群聊用 feishu_chat_members → ✅ 搜索群用 feishu skill 的 feishu_chat search
- ❌ 不知道 chat_id 就猜一个 → ✅ 先用 feishu skill 的 feishu_chat search 搜索群聊获取 chat_id
