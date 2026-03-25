---
name: feishu
description: "Manage Feishu group chats, read/write spreadsheets, and revoke user authorization. Use for searching/viewing chat info, operating on Sheets, or revoking OAuth."
---

# feishu

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Core Feishu capabilities: group chat management, spreadsheet read/write, and user authorization revocation.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| Search group chats | feishu_chat | action=search, keyword |
| View chat details | feishu_chat | action=get, chat_id |
| Read spreadsheet | feishu_sheet | action=read, url/spreadsheet_token |
| Write/append spreadsheet data | feishu_sheet | action=write/append, url/spreadsheet_token, data |
| Create spreadsheet | feishu_sheet | action=create |
| Export spreadsheet | feishu_sheet | action=export, format |
| Revoke Feishu authorization | feishu_oauth | action=revoke |

## Tools

### feishu_chat
Calls the Feishu group chat management API as the user. Actions: search (search chat list, supports keyword matching on chat name and members), get (retrieve detailed chat info including name, description, avatar, owner, permission settings).

### feishu_sheet
Feishu spreadsheet tool (runs as the user). Supports creating, reading, writing, searching, and exporting spreadsheets.

Spreadsheets (Sheets) are similar to Excel/Google Sheets and are a different product from Bitable (multi-dimensional tables / Airtable-like).

All actions (except create) accept either a url or spreadsheet_token — the tool parses automatically. Wiki URLs are also supported and automatically resolved to spreadsheet tokens.

Actions:
- info: Get spreadsheet info + full list of worksheets (replaces separate get_info + list_sheets calls)
- read: Read data. Omit range to read all data from the first worksheet
- write: Overwrite data. **Destructive — use with caution.** Omit range to write to the first worksheet starting at A1
- append: Append rows after existing data
- find: Search for cells in a worksheet
- create: Create a new spreadsheet. Supports creating with headers + data in one step
- export: Export as xlsx or csv (csv requires specifying sheet_id)

### feishu_oauth
Feishu user authorization revocation tool. Only invoke revoke when the user explicitly says "revoke authorization", "cancel authorization", "log out", or "clear authorization". No need to pass user_open_id — the system automatically obtains the current user from message context.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Calling feishu_oauth revoke when user says "re-authorize" | Authorization flow is handled automatically by the system; "re-authorize" does not mean "revoke" |
| Using feishu_sheet for Bitable (multi-dimensional tables) | Sheets and Bitable are different products — use the feishu-bitable skill for Bitable |
| Using feishu_chat search to list group members | To list group members, use feishu_chat_members from the feishu-chat skill |
