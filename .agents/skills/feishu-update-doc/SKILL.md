---
name: feishu-update-doc
description: 更新飞书云文档。支持追加、覆盖、定位替换、全文替换、前后插入、删除 7 种模式。
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

# feishu_update_doc

更新飞书云文档，支持 7 种模式。优先局部更新，慎用 overwrite（会清空文档，可能丢失图片、评论等）。

## 模式速查

| mode | 用途 | 需要定位 | 需要 markdown |
|------|------|---------|--------------|
| append | 追加到末尾 | 否 | 是 |
| overwrite | 完全覆盖（危险） | 否 | 是 |
| replace_range | 定位替换（唯一匹配） | 是 | 是 |
| replace_all | 全文替换（多处匹配） | 是 | 是（可为空串=删除） |
| insert_before | 在匹配前插入 | 是 | 是 |
| insert_after | 在匹配后插入 | 是 | 是 |
| delete_range | 删除匹配内容 | 是 | 否 |

可选参数 `new_title`：纯文本，1-800 字符，可与任何 mode 配合。

---

## 定位方式（二选一）

### selection_with_ellipsis — 内容定位

- **范围匹配**：`开头内容...结尾内容` — 匹配从开头到结尾的所有内容，建议 10-20 字符确保唯一
- **精确匹配**：`完整内容`（不含 `...`）— 匹配完整文本
- **转义**：字面量 `...` 用 `\.\.\.` 表示

### selection_by_title — 标题定位

格式：`## 章节标题`（可带或不带 # 前缀）。自动定位整个章节（从该标题到下一个同级或更高级标题之前）。

---

## 核心约束

### 小粒度精确替换
定位范围越小越安全。表格、分栏等嵌套块应精确定位到需要修改的文本，避免影响其他内容。

### 保护不可重建的内容
图片、画板、电子表格、多维表格、任务等以 token 形式存储，无法读出后原样写入。替换时避开这些区域，精确定位到纯文本部分。

### insert 模式的边界
- `insert_after` → 插入在匹配范围的**结尾**之后
- `insert_before` → 插入在匹配范围的**开头**之前

扩大定位范围确保唯一性时，注意边界仍是期望的插入点。

### 分步优于整体
多处修改时用多次小范围替换。overwrite 会丢失媒体、评论、协作历史。

---

## 不要这样做

| 错误做法 | 正确做法 |
|---------|---------|
| 大范围替换包含图片/画板的区域 | 精确定位到纯文本部分，避免破坏 token 引用 |
| 随意使用 overwrite 模式 | overwrite 会丢失图片、评论，优先用局部更新 |
| selection_with_ellipsis 中真正的 `...` 不转义 | 字面量三个点用 `\.\.\.` |
| insert 时扩大定位范围但忽略边界变化 | insert_after 插入在匹配末尾之后，insert_before 在开头之前 |

---

> 📚 详细参考：使用 `fs.read` 读取
> - `$SKILL_DIR/references/examples.md` — 全部 7 种模式的使用示例
> - `$SKILL_DIR/references/appendix.md` — 返回值格式、new_title 参数详情
> - Markdown 语法参考见 feishu-create-doc 技能文档
