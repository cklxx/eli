---
name: feishu-search
description: "飞书搜索能力：按关键词搜索员工信息，或搜索云文档和知识库 Wiki 内容。"
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 快速索引

| 用户意图 | 工具 | 关键参数 |
|---------|------|---------|
| 按姓名/手机号/邮箱搜索员工 | feishu_search_user | query |
| 搜索文档和 Wiki | feishu_search_doc_wiki | query, filter（可选） |

## 工具说明

### feishu_search_user
搜索员工信息（通过关键词搜索姓名、手机号、邮箱）。返回匹配的员工列表，包含姓名、部门、open_id 等信息。

### feishu_search_doc_wiki
【以用户身份】飞书文档与 Wiki 统一搜索工具。同时搜索云空间文档和知识库 Wiki。Actions: search。【重要】query 参数是搜索关键词（必填），filter 参数可选。【重要】filter 不传时，搜索所有文档和 Wiki；传了则同时对文档和 Wiki 应用相同的过滤条件。【重要】支持按文档类型、创建者、创建时间、打开时间等多维度筛选。【重要】返回结果包含标题和摘要高亮（<h>标签包裹匹配关键词）。

## 不要这样做

- ❌ 搜索群聊消息用 feishu_search_doc_wiki → ✅ 搜 IM 消息用 feishu-im 的 feishu_im_user_search_messages
- ❌ 查询已知 ID 的用户信息用 feishu_search_user → ✅ 已知 user_id 直接用 feishu-get 的 feishu_get_user
- ❌ 搜索群聊用 feishu_search_user → ✅ 搜索群用 feishu skill 的 feishu_chat search
