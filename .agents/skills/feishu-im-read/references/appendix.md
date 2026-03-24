# feishu-im-read 附录

## 跨会话搜索参数详情

`feishu_im_user_search_messages` 支持以下过滤参数：

| 参数 | 说明 |
|------|------|
| `query` | 搜索关键词，匹配消息内容 |
| `sender_ids` | 发送者 open_id 列表 |
| `chat_id` | 限定搜索范围的会话 ID |
| `mention_ids` | 被@用户的 open_id 列表 |
| `message_type` | 消息类型：file / image / media |
| `sender_type` | 发送者类型：user / bot / all（默认 user） |
| `chat_type` | 会话类型：group / p2p |

搜索结果每条消息额外包含 `chat_id`、`chat_type`（p2p/group）、`chat_name`。单聊消息还有 `chat_partner`（对方 open_id 和名字）。

## 资源标记格式

消息内容中可能出现以下资源标记，用 `feishu_im_user_fetch_resource` 下载：

| 资源类型 | 内容中的标记格式 | fetch_resource 参数 |
|---------|-----------------|-------------------|
| 图片 | `![image](img_xxx)` | message_id=`om_xxx`, file_key=`img_xxx`, type=`"image"` |
| 文件 | `<file key="file_xxx" .../>` | message_id=`om_xxx`, file_key=`file_xxx`, type=`"file"` |
| 音频 | `<audio key="file_xxx" .../>` | message_id=`om_xxx`, file_key=`file_xxx`, type=`"file"` |
| 视频 | `<video key="file_xxx" .../>` | message_id=`om_xxx`, file_key=`file_xxx`, type=`"file"` |

文件大小限制 100MB，不支持下载表情包、卡片中的资源。

## 时间过滤详情

| 方式 | 参数 | 示例 |
|------|------|------|
| 相对时间 | `relative_time` | `today`、`yesterday`、`this_week`、`last_3_days`、`last_24_hours` |
| 精确时间 | `start_time` + `end_time` | ISO 8601 格式：`2026-02-27T00:00:00+08:00` |

可用的 relative_time 值：`today`、`yesterday`、`day_before_yesterday`、`this_week`、`last_week`、`this_month`、`last_month`、`last_{N}_{unit}`（unit: minutes/hours/days）
