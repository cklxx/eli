# feishu-update-doc 附录

## 返回值格式

### 成功

```json
{
  "success": true,
  "doc_id": "文档ID",
  "mode": "使用的模式",
  "message": "文档更新成功（xxx模式）",
  "warnings": ["可选警告列表"],
  "log_id": "请求日志ID"
}
```

### 异步模式（大文档超时）

```json
{
  "task_id": "async_task_xxxx",
  "message": "文档更新已提交异步处理，请使用 task_id 查询状态",
  "log_id": "请求日志ID"
}
```

使用返回的 `task_id` 再次调用 update-doc（仅传 task_id 参数）查询状态。

### 错误

```json
{
  "error": "[错误码] 错误消息\n💡 Suggestion: 修复建议\n📍 Context: 上下文信息",
  "log_id": "请求日志ID"
}
```

## 可选参数：new_title

更新文档标题。特性：
- 仅支持纯文本，不支持富文本格式
- 长度限制：1-800 字符
- 可以与任何 mode 配合使用
- 标题更新在内容更新之后执行
