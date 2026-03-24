---
name: feishu-im-read
description: 飞书 IM 消息读取。支持会话消息获取、话题回复、跨会话搜索、资源下载。
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

# 飞书 IM 消息读取

## 执行前必读

- 所有工具以用户身份调用，只能读取用户有权限的会话
- `open_id` 和 `chat_id` **二选一**，优先 `chat_id`
- `relative_time` 和 `start_time/end_time` **互斥**
- 消息中出现 `thread_id` 时，主动用 `feishu_im_user_get_thread_messages` 展开话题回复
- 资源下载用 `feishu_im_user_fetch_resource`，需要 `message_id` + `file_key` + `type`

---

## 意图 → 工具

| 用户意图 | 工具 | 必填参数 | 常用可选 |
|---------|------|---------|---------|
| 获取群聊/单聊历史消息 | feishu_im_user_get_messages | chat_id 或 open_id（二选一） | relative_time, start_time/end_time, page_size, sort_rule |
| 获取话题内回复消息 | feishu_im_user_get_thread_messages | thread_id（omt_xxx） | page_size, sort_rule |
| 跨会话搜索消息 | feishu_im_user_search_messages | 至少一个过滤条件 | query, sender_ids, chat_id, relative_time, start_time/end_time, page_size |
| 下载消息中的图片 | feishu_im_user_fetch_resource | message_id, file_key（img_xxx）, type="image" | - |
| 下载消息中的文件/音频/视频 | feishu_im_user_fetch_resource | message_id, file_key（file_xxx）, type="file" | - |

---

## 核心约束

### 时间范围
用户没有明确指定时间时，根据意图推断合适的 `relative_time`。用户明确指定时间时直接用用户的值。

### 分页
- `page_size` 范围 1-50，默认 50
- `has_more=true` 时用 `page_token` 继续获取
- 需要完整结果时翻页，浏览概览时第一页通常够用

### 话题回复

| 场景 | 行为 |
|------|------|
| 需要理解上下文（默认） | 对 thread_id 获取最新 10 条回复（`page_size: 10, sort_rule: "create_time_desc"`） |
| "完整对话"、"详细讨论" | 获取全部回复（`page_size: 50, sort_rule: "create_time_asc"`），需要时翻页 |
| 浏览概览 / 不看回复 | 跳过话题展开 |

话题消息不支持时间过滤（飞书 API 限制），只能通过分页获取。

### open_id 与 chat_id

| 参数 | 格式 | 适用场景 |
|------|------|---------|
| chat_id | `oc_xxx` | 已知会话 ID（群聊或单聊均可） |
| open_id | `ou_xxx` | 已知用户 ID，获取与该用户的单聊消息 |

---

## 不要这样做

| 错误做法 | 正确做法 |
|---------|---------|
| open_id 和 chat_id 同时传 | 二选一，优先 chat_id |
| relative_time 和 start_time/end_time 同时用 | 只能选一种时间过滤方式 |
| 忽略 has_more=true 不翻页 | 需要完整结果时检查 has_more 并用 page_token 翻页 |
| 忽略消息中的 thread_id | 主动展开话题获取上下文 |
| 当前对话的图片用 user_fetch_resource | 用 feishu_im_bot_image（机器人身份） |

---

> 📚 详细参考：使用 `fs.read` 读取
> - `$SKILL_DIR/references/examples.md` — 完整使用示例
> - `$SKILL_DIR/references/errors.md` — 常见错误与排查
> - `$SKILL_DIR/references/appendix.md` — 搜索参数、资源标记、时间过滤详情
