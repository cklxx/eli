---
name: feishu-create-doc
description: |
  创建飞书云文档。从 Lark-flavored Markdown 内容创建新的飞书云文档，支持指定创建位置（文件夹/知识库/知识空间）。
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

# feishu_create_doc

通过 MCP 调用 `create-doc`，从 Lark-flavored Markdown 内容创建一个新的飞书云文档。

# 返回值

工具成功执行后，返回一个 JSON 对象，包含以下字段：

- **`doc_id`**（string）：文档的唯一标识符（token），格式如 `doxcnXXXXXXXXXXXXXXXXXXX`
- **`doc_url`**（string）：文档的访问链接，可直接在浏览器中打开，格式如 `https://www.feishu.cn/docx/doxcnXXXXXXXXXXXXXXXXXXX`
- **`message`**（string）：操作结果消息，如"文档创建成功"


# 参数

## markdown（必填）
文档的 Markdown 内容，使用 Lark-flavored Markdown 格式。

调用本工具的markdown内容应当尽量结构清晰,样式丰富, 有很高的可读性. 合理的使用callout高亮块, 分栏,表格等能力,并合理的运用插入图片与mermaid的能力,做到图文并茂..
你需要遵循以下原则:

- **结构清晰**：标题层级 ≤ 4 层，用 Callout 突出关键信息
- **视觉节奏**：用分割线、分栏、表格打破大段纯文字
- **图文交融**：流程和架构优先用 Mermaid/PlantUML 可视化
- **克制留白**：Callout 不过度、加粗只强调核心词

当用户有明确的样式,风格需求时,应当以用户的需求为准!!

**重要提示**：
- **禁止重复标题**：markdown 内容开头不要写与 title 相同的一级标题！title 参数已经是文档标题，markdown 应直接从正文内容开始
- **目录**：飞书自动生成，无需手动添加
- Markdown 语法必须符合 Lark-flavored Markdown 规范，详见下方"内容格式"章节
- 创建较长的文档时,强烈建议配合update-doc中的append mode, 进行分段的创建,提高成功率.

## title（可选）
文档标题。

## folder_token（可选）
父文件夹的 token。如果不提供，文档将创建在用户的个人空间根目录。

folder_token 可以从飞书文件夹 URL 中获取，格式如：`https://xxx.feishu.cn/drive/folder/fldcnXXXX`，其中 `fldcnXXXX` 即为 folder_token。

## wiki_node（可选）
知识库节点 token 或 URL（可选，传入则在该节点下创建文档，与 folder_token 和 wiki_space 互斥）

wiki_node 可以从飞书知识库页面 URL 中获取，格式如：`https://xxx.feishu.cn/wiki/wikcnXXXX`，其中 `wikcnXXXX` 即为 wiki_node token。

## wiki_space（可选）
知识空间 ID（可选，传入则在该空间根目录下创建文档。特殊值 `my_library` 表示用户的个人知识库。与 wiki_node 和 folder_token 互斥）

wiki_space 可以从知识空间设置页面 URL 中获取，格式如：`https://xxx.feishu.cn/wiki/settings/7448000000000009300`，其中 `7448000000000009300` 即为 wiki_space ID。

**参数优先级**：wiki_node > wiki_space > folder_token

# 示例

## 示例 1：创建简单文档

```json
{
  "title": "项目计划",
  "markdown": "# 项目概述\n\n这是一个新项目。\n\n## 目标\n\n- 目标 1\n- 目标 2"
}
```

## 示例 2：创建到指定文件夹

```json
{
  "title": "会议纪要",
  "folder_token": "fldcnXXXXXXXXXXXXXXXXXXXXXX",
  "markdown": "# 周会 2025-01-15\n\n## 讨论议题\n\n1. 项目进度\n2. 下周计划"
}
```

## 示例 3：使用飞书扩展语法

使用高亮块、表格等飞书特有功能：

```json
{
  "title": "产品需求",
  "markdown": "<callout emoji=\"💡\" background-color=\"light-blue\">\n重要需求说明\n</callout>\n\n## 功能列表\n\n<lark-table header-row=\"true\">\n| 功能 | 优先级 |\n|------|--------|\n| 登录 | P0 |\n| 导出 | P1 |\n</lark-table>"
}
```

## 示例 4：创建到知识库节点下

```json
{
  "title": "技术文档",
  "wiki_node": "wikcnXXXXXXXXXXXXXXXXXXXXXX",
  "markdown": "# API 接口说明\n\n这是一个知识库文档。"
}
```

## 示例 5：创建到知识空间根目录

```json
{
  "title": "项目概览",
  "wiki_space": "7448000000000009300",
  "markdown": "# 项目概览\n\n这是知识空间根目录下的一级文档。"
}
```

## 示例 6：创建到个人知识库

```json
{
  "title": "学习笔记",
  "wiki_space": "my_library",
  "markdown": "# 学习笔记\n\n这是创建在个人知识库中的文档。"
}
```

# 内容格式

文档内容使用 **Lark-flavored Markdown** 格式（标准 Markdown 的超集，支持飞书特有的 XML 标签扩展）。

常用扩展语法速查：
- 高亮块: `<callout emoji="💡" background-color="light-blue">内容</callout>`
- 分栏: `<grid cols="2"><column>左</column><column>右</column></grid>`
- 增强表格: `<lark-table header-row="true"><lark-tr><lark-td>内容</lark-td></lark-tr></lark-table>`
- 图片: `<image url="https://..." width="800" align="center" caption="说明"/>`
- 文件: `<file url="https://..." name="文档.pdf"/>`
- Mermaid 画板: ` ```mermaid ` 代码块
- 提及用户: `<mention-user id="ou_xxx"/>`
- 文字颜色: `<text color="red">红色</text>`

**完整语法参考**：使用 `fs.read` 读取 `$SKILL_DIR/LARK_MARKDOWN_REFERENCE.md`
