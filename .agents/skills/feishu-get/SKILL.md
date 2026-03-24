---
name: feishu-get
description: "查询已知用户的详细信息。通过 user_id 查指定用户，或不传参查当前用户自己。"
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 快速索引

| 用户意图 | 工具 | 关键参数 |
|---------|------|---------|
| 查看自己的信息 | feishu_get_user | （不传参） |
| 查看指定用户信息 | feishu_get_user | user_id |

## 工具说明

### feishu_get_user
获取用户信息。不传 user_id 时获取当前用户自己的信息；传 user_id 时获取指定用户的信息。返回用户姓名、头像、邮箱、手机号、部门等信息。

## 不要这样做

- ❌ 搜索用户用 feishu_get_user → ✅ 搜索用 feishu-search 的 feishu_search_user，get_user 只能查已知 ID 或查自己
- ❌ 不知道 user_id 就让用户提供 → ✅ 先用 feishu-search 的 feishu_search_user 按姓名搜索获取 open_id
