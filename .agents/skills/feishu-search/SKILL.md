---
name: feishu-search
description: "Search Feishu for employees by keyword, or search cloud documents and Wiki content."
---

# feishu-search

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Keyword-based search across Feishu employees, cloud documents, and Wiki pages.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| Search employees by name/phone/email | feishu_search_user | query |
| Search documents and Wiki | feishu_search_doc_wiki | query, filter (optional) |

## Tools

### feishu_search_user
Searches for employees by keyword (matches against name, phone number, email). Returns a list of matching employees including name, department, open_id, and more.

### feishu_search_doc_wiki
Unified Feishu document and Wiki search tool (runs as the user). Searches both Drive documents and Wiki pages simultaneously. Action: search.

**Important:** The query parameter (search keyword) is required. The filter parameter is optional.

**Important:** When filter is omitted, all documents and Wiki pages are searched. When provided, the same filter conditions are applied to both document and Wiki results.

**Important:** Supports filtering by document type, creator, creation time, last opened time, and other dimensions.

**Important:** Results include title and excerpt highlighting (`<h>` tags wrap matching keywords).

## Pitfalls

| Wrong | Right |
|-------|-------|
| Using feishu_search_doc_wiki to search chat messages | For IM message search, use feishu_im_user_search_messages from the feishu-im skill |
| Using feishu_search_user to look up a user whose ID you already know | Use feishu_get_user from the feishu-get skill for known user IDs |
| Using feishu_search_user to search for group chats | Use feishu_chat search from the feishu skill to search chats |
