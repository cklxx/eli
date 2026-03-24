# feishu-im-read 常见错误与排查

| 错误现象 | 根本原因 | 解决方案 |
|---------|---------|---------|
| 消息结果太少 | 时间范围太窄或未传时间参数 | 根据用户意图推断合适的 `relative_time` |
| 消息不完整 | 没有检查 has_more 并翻页 | has_more=true 时用 page_token 翻页 |
| 话题讨论内容不完整 | 没有展开 thread_id | 发现 thread_id 时获取话题回复 |
| "open_id 和 chat_id 不能同时提供" | 同时传了两个参数 | 只传其中一个 |
| "relative_time 和 start_time/end_time 不能同时使用" | 时间参数冲突 | 选择一种时间过滤方式 |
| "未找到与 open_id=xxx 的单聊会话" | 没有单聊记录 | 改用 chat_id，或确认存在单聊 |
| 话题消息返回为空 | thread_id 格式不正确 | 确认为 `omt_xxx` 格式 |
| 图片/文件下载失败 | file_key 或 message_id 不匹配 | 确认 file_key 来自该 message_id |
| 权限不足 | 用户未授权或无权限 | 确认已完成 OAuth 授权且是会话成员 |
