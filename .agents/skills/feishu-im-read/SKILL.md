---
name: feishu-im-read
description: Read Feishu IM messages. Supports conversation history, thread replies, cross-conversation search, and resource downloads.
---

# feishu-im-read

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Read and search Feishu IM messages using user identity. Supports conversation history retrieval, thread reply expansion, cross-conversation search, and image/file resource downloads.

## Prerequisites

- All tools use user identity — only conversations the user has access to can be read
- `open_id` and `chat_id` are mutually exclusive; prefer `chat_id`
- `relative_time` and `start_time/end_time` are mutually exclusive
- When a message contains `thread_id`, proactively use `feishu_im_user_get_thread_messages` to expand thread replies
- For resource downloads, use `feishu_im_user_fetch_resource` with `message_id` + `file_key` + `type`

## Quick Reference

| Intent | Tool | Required Params | Common Optional |
|--------|------|-----------------|-----------------|
| Get group/direct chat history | feishu_im_user_get_messages | chat_id or open_id (one of) | relative_time, start_time/end_time, page_size, sort_rule |
| Get thread replies | feishu_im_user_get_thread_messages | thread_id (omt_xxx) | page_size, sort_rule |
| Search messages across conversations | feishu_im_user_search_messages | at least one filter condition | query, sender_ids, chat_id, relative_time, start_time/end_time, page_size |
| Download image from message | feishu_im_user_fetch_resource | message_id, file_key (img_xxx), type="image" | - |
| Download file/audio/video from message | feishu_im_user_fetch_resource | message_id, file_key (file_xxx), type="file" | - |

## Constraints

### Time range

When the user does not specify a time range, infer a suitable `relative_time` based on intent. When the user specifies an explicit time, use their value directly.

### Pagination

- `page_size` range: 1-50, default 50
- When `has_more=true`, use `page_token` to continue fetching
- Paginate when complete results are needed; the first page usually suffices for browsing

### Thread replies

| Scenario | Behavior |
|----------|----------|
| Need to understand context (default) | Get latest 10 replies (`page_size: 10, sort_rule: "create_time_desc"`) |
| "Full conversation", "detailed discussion" | Get all replies (`page_size: 50, sort_rule: "create_time_asc"`), paginate as needed |
| Browsing overview / skip replies | Skip thread expansion |

Thread messages do not support time filtering (Feishu API limitation) — only pagination is available.

### open_id vs chat_id

| Param | Format | Use Case |
|-------|--------|----------|
| chat_id | `oc_xxx` | Known conversation ID (works for both group and direct chats) |
| open_id | `ou_xxx` | Known user ID — fetch direct chat messages with that user |

## Pitfalls

| Wrong | Right |
|-------|-------|
| Pass both open_id and chat_id | Use one or the other; prefer chat_id |
| Use both relative_time and start_time/end_time | Choose only one time filtering method |
| Ignore has_more=true without paginating | Check has_more and use page_token to paginate when complete results are needed |
| Ignore thread_id in messages | Proactively expand threads to get context |
| Download current conversation images with user_fetch_resource | Use feishu_im_bot_image (bot identity) |

---

> Detailed references: use `fs.read` to read
> - `$SKILL_DIR/references/examples.md` — Full usage examples
> - `$SKILL_DIR/references/errors.md` — Common errors and troubleshooting
> - `$SKILL_DIR/references/appendix.md` — Search parameters, resource markers, time filtering details
