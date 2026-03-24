---
name: feishu-fetch-doc
description: 获取飞书云文档内容。返回 Markdown，支持处理文档中的图片、文件和画板。
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

# feishu_fetch_doc

获取飞书云文档的 Markdown 内容（Lark-flavored 格式）。

## 重要：图片、文件、画板的处理

**文档中的图片、文件、画板需要通过 `feishu_doc_media`（action: download）工具单独获取！**

### 识别格式

返回的 Markdown 中，媒体文件以 HTML 标签形式出现：

- **图片**：`<image token="..." width="..." height="..." align="..."/>`
- **文件**：`<view type="1"><file token="..." name="..."/></view>`
- **画板**：`<whiteboard token="..."/>`

### 获取步骤

1. 从 HTML 标签中提取 `token` 属性值
2. 调用 `feishu_doc_media`（action: download, resource_token: 提取的token, resource_type: media, output_path: 保存路径）

## 参数

- **`doc_id`**（必填）：支持直接传文档 URL 或 token
  - 直接传 URL：`https://xxx.feishu.cn/docx/Z1FjxxxxxxxxxxxxxxxxxxxtnAc`（系统自动提取 token）
  - 直接传 token：`Z1FjxxxxxxxxxxxxxxxxxxxtnAc`
  - 知识库 URL/token 也支持：`https://xxx.feishu.cn/wiki/Z1FjxxxxxxxxxxxxxxxxxxxtnAc` 或 `Z1FjxxxxxxxxxxxxxxxxxxxtnAc`

## Wiki URL 处理策略

知识库链接（`/wiki/TOKEN`）背后可能是云文档、电子表格、多维表格等不同类型的文档。**不能直接假设是云文档**，必须先查询实际类型。

### 处理流程

1. **先调用 `feishu_wiki_space_node`（action: get）解析 wiki token**
2. **从返回的 `node` 中获取 `obj_type` 和 `obj_token`**
3. **根据 `obj_type` 调用对应工具**：

| obj_type | 工具 | 传参 |
|----------|------|------|
| `docx` | `feishu_mcp_fetch_doc` | doc_id = obj_token |
| `sheet` | `feishu_sheet` | spreadsheet_token = obj_token |
| `bitable` | `feishu_bitable_*` 系列 | app_token = obj_token |
| 其他 | 告知用户暂不支持该类型 | — |

> 完整示例见 `$SKILL_DIR/references/examples.md`

## 工具组合

| 需求 | 工具 |
|------|------|
| 获取文档文本 | `feishu_mcp_fetch_doc` |
| 下载图片/文件/画板 | `feishu_doc_media`（action: download） |
| 解析 wiki token 类型 | `feishu_wiki_space_node`（action: get） |
| 读写电子表格 | `feishu_sheet` |
| 操作多维表格 | `feishu_bitable_*` 系列 |

## 不要这样做

- ❌ 拿到 wiki URL 直接当 docx fetch → ✅ 先用 feishu_wiki_space_node get 查 obj_type
- ❌ 期望 fetch_doc 返回图片内容 → ✅ 图片以 token 标签返回，需用 feishu_doc_media download 单独获取
- ❌ 忽略返回内容中的 `<image>`/`<file>`/`<whiteboard>` 标签 → ✅ 提取 token 并告知用户或按需下载
