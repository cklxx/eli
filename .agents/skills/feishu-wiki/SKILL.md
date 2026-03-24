---
name: feishu-wiki
description: "飞书知识库管理。支持查看/创建知识空间，以及知识库节点的增删改查和类型解析。"
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 快速索引

| 用户意图 | 工具 | 关键参数 |
|---------|------|---------|
| 列出知识空间 | feishu_wiki_space | action=list |
| 查看知识空间信息 | feishu_wiki_space | action=get, space_id |
| 创建知识空间 | feishu_wiki_space | action=create |
| 列出知识库节点 | feishu_wiki_space_node | action=list, space_id |
| 查看节点信息（wiki→obj_token 转换） | feishu_wiki_space_node | action=get, node_token |
| 创建/移动/复制节点 | feishu_wiki_space_node | action=create/move/copy |

## 工具说明

### feishu_wiki_space
飞书知识空间管理工具。当用户要求查看知识库列表、获取知识库信息、创建知识库时使用。Actions: list（列出知识空间）, get（获取知识空间信息）, create（创建知识空间）。【重要】space_id 可以从浏览器 URL 中获取，或通过 list 接口获取。【重要】知识空间（Space）是知识库的基本组成单位，包含多个具有层级关系的文档节点。

### feishu_wiki_space_node
飞书知识库节点管理工具。操作：list（列表）、get（获取）、create（创建）、move（移动）、copy（复制）。节点是知识库中的文档，包括 doc、bitable(多维表表格)、sheet(电子表格) 等类型。node_token 是节点的唯一标识符，obj_token 是实际文档的 token。可通过 get 操作将 wiki 类型的 node_token 转换为实际文档的 obj_token。

## 不要这样做

- ❌ 拿到 wiki URL 直接当 docx 读 → ✅ 先用 feishu_wiki_space_node get 查 obj_type，再按类型调对应工具（doc 用 read-doc，sheet 用 feishu_sheet 等）
- ❌ 搜索知识库文档用 feishu_wiki_space_node list → ✅ 搜索文档用 feishu-search 的 feishu_search_doc_wiki，list 只列出层级结构
- ❌ 混淆 node_token 和 obj_token → ✅ node_token 是知识库节点 ID，obj_token 是实际文档 ID，用 get 操作做转换
