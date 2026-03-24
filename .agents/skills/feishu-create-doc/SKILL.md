---
name: feishu-create-doc
description: 创建飞书云文档。从 Lark-flavored Markdown 创建新文档，支持指定文件夹或知识库。
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

# feishu_create_doc

从 Lark-flavored Markdown 创建飞书云文档。返回 `doc_id`、`doc_url`、`message`。

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| markdown | 是 | Lark-flavored Markdown 内容 |
| title | 否 | 文档标题 |
| folder_token | 否 | 父文件夹 token（`fldcnXXXX`），不提供则创建在个人空间根目录 |
| wiki_node | 否 | 知识库节点 token 或 URL（`wikcnXXXX`），与 folder_token/wiki_space 互斥 |
| wiki_space | 否 | 知识空间 ID，特殊值 `my_library` 表示个人知识库，与 wiki_node/folder_token 互斥 |

**参数优先级**：wiki_node > wiki_space > folder_token

---

## 内容规范

markdown 内容应当结构清晰、样式丰富、可读性高：
- **结构清晰**：标题层级 ≤ 4 层，用 Callout 突出关键信息
- **视觉节奏**：用分割线、分栏、表格打破大段纯文字
- **图文交融**：流程和架构优先用 Mermaid/PlantUML 可视化
- **克制留白**：Callout 不过度、加粗只强调核心词

用户有明确样式/风格需求时，以用户需求为准。

### 常用扩展语法速查

- 高亮块: `<callout emoji="💡" background-color="light-blue">内容</callout>`
- 分栏: `<grid cols="2"><column>左</column><column>右</column></grid>`
- 增强表格: `<lark-table header-row="true"><lark-tr><lark-td>内容</lark-td></lark-tr></lark-table>`
- 图片: `<image url="https://..." width="800" align="center" caption="说明"/>`
- 文件: `<file url="https://..." name="文档.pdf"/>`
- Mermaid 画板: ` ```mermaid ` 代码块
- 提及用户: `<mention-user id="ou_xxx"/>`
- 文字颜色: `<text color="red">红色</text>`

---

## 不要这样做

| 错误做法 | 正确做法 |
|---------|---------|
| markdown 开头写与 title 相同的一级标题 | title 已是文档标题，markdown 直接从正文开始 |
| 手动添加目录 | 飞书自动生成目录 |
| URL 图片用 doc_media insert | 用 `<image url="..."/>` 语法 |
| 一次性创建超长文档 | 配合 update-doc append 模式分段创建 |

---

> 📚 详细参考：使用 `fs.read` 读取
> - `$SKILL_DIR/references/examples.md` — 完整使用示例
> - `$SKILL_DIR/LARK_MARKDOWN_REFERENCE.md` — Lark-flavored Markdown 完整语法参考
