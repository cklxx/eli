---
name: feishu-im
description: "飞书消息发送与机器人资源下载。用户身份发消息/回复，机器人身份下载当前对话图片文件。"
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 快速索引

| 用户意图 | 工具 | 关键参数 |
|---------|------|---------|
| 以用户身份发消息 | feishu_im_user_message | action=send, receive_id, msg_type, content |
| 以用户身份回复消息 | feishu_im_user_message | action=reply, message_id, msg_type, content |
| 下载当前对话中的图片/文件（机器人身份） | feishu_im_bot_image | message_id, image_key/file_key |

> 消息读取/搜索/资源下载 → 使用 **feishu-im-read** skill

## 工具说明

### feishu_im_user_message
飞书用户身份 IM 消息工具。**有且仅当用户明确要求以自己身份发消息、回复消息时使用，当没有明确要求时优先使用 message 系统工具**。

Actions:
- send（发送消息）：发送消息到私聊或群聊。私聊用 receive_id_type=open_id，群聊用 receive_id_type=chat_id
- reply（回复消息）：回复指定 message_id 的消息，支持话题回复（reply_in_thread=true）

【重要】content 必须是合法 JSON 字符串，格式取决于 msg_type。最常用：text 类型 content 为 '{"text":"消息内容"}'。

【安全约束】此工具以用户身份发送消息，发出后对方看到的发送者是用户本人。调用前必须先向用户确认：1) 发送对象（哪个人或哪个群）2) 消息内容。禁止在用户未明确同意的情况下自行发送消息。

### feishu_im_bot_image
【以机器人身份】下载飞书 IM 消息中的图片或文件资源到本地。

适用场景：用户直接发送给机器人的消息、用户引用的消息、机器人收到的群聊消息中的图片/文件。即当前对话上下文中出现的 message_id 和 image_key/file_key，应使用本工具下载。
引用消息的 message_id 可从上下文中的 [message_id=om_xxx] 提取，无需向用户询问。

文件自动保存到 /tmp/openclaw/ 下，返回值中的 saved_path 为实际保存路径。

### 消息读取相关工具
feishu_im_user_get_messages、feishu_im_user_get_thread_messages、feishu_im_user_search_messages、feishu_im_user_fetch_resource 的详细用法见 **feishu-im-read** skill。

## 不要这样做

- 未经用户确认就以用户身份发消息 → 必须先确认发送对象和内容，用户明确同意后才能调用 feishu_im_user_message
- 当前对话中的图片用 feishu_im_user_fetch_resource 下载 → 用 feishu_im_bot_image（机器人身份，无需用户授权）
- 需要完整的消息历史阅读和分析时用 feishu-im skill → 用 feishu-im-read skill
- content 参数传纯文本字符串 → content 必须是 JSON 字符串，如 '{"text":"你好"}'
