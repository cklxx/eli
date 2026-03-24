---
name: feishu-wiki
description: "2 tools: feishu_wiki_space, feishu_wiki_space_node"
---

Call tools via: sidecar(tool="<name>", params={...})

## feishu_wiki_space
飞书知识空间管理工具。当用户要求查看知识库列表、获取知识库信息、创建知识库时使用。Actions: list（列出知识空间）, get（获取知识空间信息）, create（创建知识空间）。【重要】space_id 可以从浏览器 URL 中获取，或通过 list 接口获取。【重要】知识空间（Space）是知识库的基本组成单位，包含多个具有层级关系的文档节点。

## feishu_wiki_space_node
飞书知识库节点管理工具。操作：list（列表）、get（获取）、create（创建）、move（移动）、copy（复制）。节点是知识库中的文档，包括 doc、bitable(多维表表格)、sheet(电子表格) 等类型。node_token 是节点的唯一标识符，obj_token 是实际文档的 token。可通过 get 操作将 wiki 类型的 node_token 转换为实际文档的 obj_token。

