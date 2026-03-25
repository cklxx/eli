---
name: feishu-im
description: Send Feishu messages as user identity and download images/files from the current conversation as bot identity.
---

# feishu-im

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Send messages and replies as the user's Feishu identity, and download images/files from the current conversation using bot identity.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| Send message as user | feishu_im_user_message | action=send, receive_id, msg_type, content |
| Reply to message as user | feishu_im_user_message | action=reply, message_id, msg_type, content |
| Download image/file from current conversation (bot identity) | feishu_im_bot_image | message_id, image_key/file_key |

> For message reading/searching/resource downloading, use the **feishu-im-read** skill.

## Tools

### feishu_im_user_message

Send or reply to messages using the user's Feishu identity. **Use only when the user explicitly requests sending a message as themselves; otherwise prefer the system message tools.**

Actions:
- **send**: Send a message to a direct chat or group chat. Use `receive_id_type=open_id` for direct chats, `receive_id_type=chat_id` for group chats.
- **reply**: Reply to a specific `message_id`. Supports thread replies (`reply_in_thread=true`).

**Important**: `content` must be a valid JSON string. Format depends on `msg_type`. Most common: text type content is `'{"text":"message content"}'`.

**Safety constraint**: This tool sends messages as the user — the recipient sees the user as the sender. Before calling, you must confirm with the user: 1) the recipient (which person or group), 2) the message content. Never send messages without the user's explicit consent.

### feishu_im_bot_image

Download images or file resources from Feishu IM messages to local storage, using bot identity.

Use for: messages sent directly to the bot, quoted messages, or images/files in group messages the bot received — i.e., any `message_id` and `image_key`/`file_key` from the current conversation context.

The `message_id` for quoted messages can be extracted from `[message_id=om_xxx]` in context — no need to ask the user.

Files are saved to `/tmp/openclaw/`; the `saved_path` in the response is the actual file path.

### Message reading tools

`feishu_im_user_get_messages`, `feishu_im_user_get_thread_messages`, `feishu_im_user_search_messages`, `feishu_im_user_fetch_resource` — see the **feishu-im-read** skill for detailed usage.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Send message as user without confirming first | Always confirm recipient and content with the user before calling feishu_im_user_message |
| Download current conversation images with feishu_im_user_fetch_resource | Use feishu_im_bot_image (bot identity, no user authorization needed) |
| Use feishu-im skill for full message history reading/analysis | Use feishu-im-read skill |
| Pass plain text string as content | content must be a JSON string, e.g. `'{"text":"hello"}'` |
