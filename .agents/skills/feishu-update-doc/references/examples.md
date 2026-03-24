# feishu-update-doc 使用示例

## append - 追加到末尾

```json
{
  "doc_id": "文档ID或URL",
  "mode": "append",
  "markdown": "## 新章节\n\n追加的内容..."
}
```

## replace_range - 定位替换

使用 `selection_with_ellipsis`：
```json
{
  "doc_id": "文档ID或URL",
  "mode": "replace_range",
  "selection_with_ellipsis": "## 旧章节标题...旧章节结尾。",
  "markdown": "## 新章节标题\n\n新的内容..."
}
```

使用 `selection_by_title`（替换整个章节）：
```json
{
  "doc_id": "文档ID或URL",
  "mode": "replace_range",
  "selection_by_title": "## 功能说明",
  "markdown": "## 功能说明\n\n更新后的功能说明内容..."
}
```

## replace_all - 全文替换

```json
{
  "doc_id": "文档ID或URL",
  "mode": "replace_all",
  "selection_with_ellipsis": "张三",
  "markdown": "李四"
}
```

返回值包含 `replace_count` 字段，表示替换次数。`markdown` 可为空字符串表示删除所有匹配。

## insert_before - 前插入

```json
{
  "doc_id": "文档ID或URL",
  "mode": "insert_before",
  "selection_with_ellipsis": "## 危险操作...数据丢失风险。",
  "markdown": "> **警告**：以下操作需谨慎！"
}
```

## insert_after - 后插入

```json
{
  "doc_id": "文档ID或URL",
  "mode": "insert_after",
  "selection_with_ellipsis": "```python...```",
  "markdown": "**输出示例**：\n```\nresult = 42\n```"
}
```

## delete_range - 删除内容

使用 `selection_with_ellipsis`：
```json
{
  "doc_id": "文档ID或URL",
  "mode": "delete_range",
  "selection_with_ellipsis": "## 废弃章节...不再需要的内容。"
}
```

使用 `selection_by_title`（删除整个章节）：
```json
{
  "doc_id": "文档ID或URL",
  "mode": "delete_range",
  "selection_by_title": "## 废弃章节"
}
```

注意：delete_range 模式不需要 markdown 参数。

## 同时更新标题和内容

可以在任何更新模式中添加 `new_title` 参数：

```json
{
  "doc_id": "文档ID或URL",
  "mode": "append",
  "markdown": "## 更新日志\n\n2025-12-18: 新增功能...",
  "new_title": "项目文档（已更新）"
}
```

## overwrite - 完全覆盖

⚠️ 会清空文档后重写，可能丢失图片、评论等，仅在需要完全重建文档时使用。

```json
{
  "doc_id": "文档ID或URL",
  "mode": "overwrite",
  "markdown": "## 新文档\n\n全新的内容..."
}
```

## 修复画板语法错误

当 create-doc 或 update-doc 返回画板写入失败的 warning 时：
1. warning 中包含 whiteboard 标签（如 `<whiteboard token="xxx"/>`）
2. 分析错误信息，修正 Mermaid/PlantUML 语法
3. 用 `replace_range` 替换：`selection_with_ellipsis` 使用 warning 中的 whiteboard 标签，`markdown` 提供修正后的代码块
4. 重新提交验证
