---
name: feishu-im
description: "6 tools: feishu_im_user_message, feishu_im_user_fetch_resource, feishu_im_user_get_messages, feishu_im_user_get_thread_messages, feishu_im_user_search_messages, feishu_im_bot_image"
---

Call tools via: sidecar(tool="<name>", params={...})

## feishu_im_user_message
飞书用户身份 IM 消息工具。**有且仅当用户明确要求以自己身份发消息、回复消息时使用，当没有明确要求时优先使用message系统工具**。

Actions:
- send（发送消息）：发送消息到私聊或群聊。私聊用 receive_id_type=open_id，群聊用 receive_id_type=chat_id
- reply（回复消息）：回复指定 message_id 的消息，支持话题回复（reply_in_thread=true）

【重要】content 必须是合法 JSON 字符串，格式取决于 msg_type。最常用：text 类型 content 为 '{"text":"消息内容"}'。

【安全约束】此工具以用户身份发送消息，发出后对方看到的发送者是用户本人。调用前必须先向用户确认：1) 发送对象（哪个人或哪个群）2) 消息内容。禁止在用户未明确同意的情况下自行发送消息。

## feishu_im_user_fetch_resource
【以用户身份】下载飞书 IM 消息中的文件或图片资源到本地文件。需要用户 OAuth 授权。

适用场景：当你以用户身份调用了消息列表/搜索等 API 获取到 message_id 和 file_key 时，应使用本工具以同样的用户身份下载资源。
注意：如果 message_id 来自当前对话上下文（用户发给机器人的消息、引用的消息），请使用 feishu_im_bot_image 工具以机器人身份下载，无需用户授权。

参数说明：
- message_id：消息 ID（om_xxx），从消息事件或消息列表中获取
- file_key：资源 Key，从消息体中获取。图片用 image_key（img_xxx），文件用 file_key（file_xxx）
- type：图片用 image，文件/音频/视频用 file

文件自动保存到 /tmp/openclaw/ 下，返回值中的 saved_path 为实际保存路径。
限制：文件大小不超过 100MB。不支持下载表情包、合并转发消息、卡片中的资源。

## feishu_im_user_get_messages
【以用户身份】获取群聊或单聊的历史消息。

用法：
- 通过 chat_id 获取群聊/单聊消息
- 通过 open_id 获取与指定用户的单聊消息（自动解析 chat_id）
- 支持时间范围过滤：relative_time（如 today、last_3_days）或 start_time/end_time（ISO 8601 格式）
- 支持分页：page_size + page_token

【参数约束】
- open_id 和 chat_id 必须二选一，不能同时提供
- relative_time 和 start_time/end_time 不能同时使用
- page_size 范围 1-50，默认 50

返回消息列表，每条消息包含 message_id、msg_type、content（AI 可读文本）、sender、create_time 等字段。

## feishu_im_user_get_thread_messages
【以用户身份】获取话题（thread）内的消息列表。

用法：
- 通过 thread_id（omt_xxx）获取话题内的所有消息
- 支持分页：page_size + page_token

【注意】话题消息不支持时间范围过滤（飞书 API 限制）

返回消息列表，格式同 feishu_im_user_get_messages。

## feishu_im_user_search_messages
【以用户身份】跨会话搜索飞书消息。

用法：
- 按关键词搜索消息内容
- 按发送者、被@用户、消息类型过滤
- 按时间范围过滤：relative_time 或 start_time/end_time
- 限定在某个会话内搜索（chat_id）
- 支持分页：page_size + page_token

【参数约束】
- 所有参数均可选，但至少应提供一个过滤条件
- relative_time 和 start_time/end_time 不能同时使用
- page_size 范围 1-50，默认 50

返回消息列表，每条消息包含 message_id、msg_type、content、sender、create_time 等字段。
每条消息还包含 chat_id、chat_type（p2p/group）、chat_name（群名或单聊对方名字）。
单聊消息额外包含 chat_partner（对方 open_id 和名字）。
搜索结果中的 chat_id 和 thread_id 可配合 feishu_im_user_get_messages / feishu_im_user_get_thread_messages 查看上下文。

## feishu_im_bot_image
【以机器人身份】下载飞书 IM 消息中的图片或文件资源到本地。

适用场景：用户直接发送给机器人的消息、用户引用的消息、机器人收到的群聊消息中的图片/文件。即当前对话上下文中出现的 message_id 和 image_key/file_key，应使用本工具下载。
引用消息的 message_id 可从上下文中的 [message_id=om_xxx] 提取，无需向用户询问。

文件自动保存到 /tmp/openclaw/ 下，返回值中的 saved_path 为实际保存路径。

