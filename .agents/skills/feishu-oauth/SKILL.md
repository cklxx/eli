---
name: feishu-oauth
description: "飞书批量授权工具。仅在用户明确要求一次性授权所有权限时使用。"
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 快速索引

| 用户意图 | 工具 | 关键参数 |
|---------|------|---------|
| 一次性授权所有权限 | feishu_oauth_batch_auth | （无需参数） |

## 工具说明

### feishu_oauth_batch_auth
飞书批量授权工具，一次性授权应用已开通的所有用户权限。仅在用户明确要求"授权所有权限"、"一次性授权"时使用。

## 不要这样做

- ❌ 用户授权失败/过期时调用 batch_auth → ✅ 仅在用户明确要求"授权所有权限"时使用，授权失败/过期由系统自动处理
- ❌ 用户说"授权一下"就调用 batch_auth → ✅ 普通的单项授权由系统自动弹出，batch_auth 仅用于"全部授权"场景
- ❌ 把 batch_auth 和 feishu skill 的 feishu_oauth revoke 搞混 → ✅ batch_auth 是授权，revoke 是撤销，完全相反的操作
