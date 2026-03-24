---
name: feishu-search
description: "2 tools: feishu_search_user, feishu_search_doc_wiki"
---

Call tools via: sidecar(tool="<name>", params={...})

## feishu_search_user
搜索员工信息（通过关键词搜索姓名、手机号、邮箱）。返回匹配的员工列表，包含姓名、部门、open_id 等信息。

## feishu_search_doc_wiki
【以用户身份】飞书文档与 Wiki 统一搜索工具。同时搜索云空间文档和知识库 Wiki。Actions: search。【重要】query 参数是搜索关键词（必填），filter 参数可选。【重要】filter 不传时，搜索所有文档和 Wiki；传了则同时对文档和 Wiki 应用相同的过滤条件。【重要】支持按文档类型、创建者、创建时间、打开时间等多维度筛选。【重要】返回结果包含标题和摘要高亮（<h>标签包裹匹配关键词）。

