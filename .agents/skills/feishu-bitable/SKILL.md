---
name: feishu-bitable
description: Create, query, edit, and manage Feishu Bitable (multidimensional spreadsheets). Supports 27 field types, advanced filtering, and batch operations.
---

# feishu-bitable

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Manage Feishu Bitable apps, tables, fields, records, and views. Supports 27 field types, advanced filtering, and batch operations up to 500 records per call.

## Prerequisites

- **Before writing records**: call `feishu_bitable_app_table_field.list` to get field type/ui_type
- **Creating a table**: when requirements are clear, define fields in `table.fields` during `create`; for exploratory work, use the default table and modify incrementally
- **Default table empty rows**: `app.create` includes a default table with empty records — call `record.list` + `batch_delete` to clean up before inserting
- **Batch limit**: max 500 records per call; split larger sets into batches
- **Concurrency limit**: concurrent writes to the same table are not supported — serialize calls with 0.5-1 second delays

## Quick Reference

| Intent | Tool | action | Required Params | Common Optional |
|--------|------|--------|-----------------|-----------------|
| List table fields | feishu_bitable_app_table_field | list | app_token, table_id | - |
| List records | feishu_bitable_app_table_record | list | app_token, table_id | filter, sort, field_names |
| Create one record | feishu_bitable_app_table_record | create | app_token, table_id, fields | - |
| Batch import records | feishu_bitable_app_table_record | batch_create | app_token, table_id, records (max 500) | - |
| Update one record | feishu_bitable_app_table_record | update | app_token, table_id, record_id, fields | - |
| Batch update records | feishu_bitable_app_table_record | batch_update | app_token, table_id, records (max 500) | - |
| Create a Bitable app | feishu_bitable_app | create | name | folder_token |
| Create a table | feishu_bitable_app_table | create | app_token, name | fields |
| Create a field | feishu_bitable_app_table_field | create | app_token, table_id, field_name, type | property |
| Create a view | feishu_bitable_app_table_view | create | app_token, table_id, view_name, view_type | - |

## Constraints

### Field types and value formats must match exactly

| type | ui_type | Field Type | Correct Format | Common Mistake |
|------|---------|------------|----------------|----------------|
| 11 | User | Person | `[{id: "ou_xxx"}]` | Passing string `"ou_xxx"` |
| 5 | DateTime | Date | `1674206443000` (milliseconds) | Passing seconds timestamp or string |
| 3 | SingleSelect | Single select | `"Option name"` | Passing array `["Option name"]` |
| 4 | MultiSelect | Multi select | `["Option1", "Option2"]` | Passing string |
| 15 | Url | Hyperlink | `{link: "...", text: "..."}` | Passing bare URL string |
| 17 | Attachment | Attachment | `[{file_token: "..."}]` | Passing external URL |

**Mandatory workflow**: call `field.list` to get type/ui_type first, then construct values per the table above. Error codes `125406X` / `1254015` indicate format mismatch.

**Person field**: defaults to open_id (`ou_...`), format `[{id: "ou_xxx"}]` — only the `id` field is accepted.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Write records without checking field types first | Always call `field.list` to get type/ui_type first |
| Pass string `"ou_xxx"` for person field | Must pass array of objects `[{id: "ou_xxx"}]` |
| Pass string `"2026-02-27"` for date field | Must pass millisecond timestamp `1740614400000` |
| Pass array `["Option"]` for single select | Pass string `"Option"` |
| Batch more than 500 records in one call | Split into batches of 500 or fewer |
| Write to the same table concurrently | Serialize calls with 0.5-1 second delays |

---

> Detailed references: use `fs.read` to read
> - `$SKILL_DIR/references/examples.md` — Full usage examples (list fields, batch import, advanced filtering)
> - `$SKILL_DIR/references/errors.md` — Error codes and troubleshooting
> - `$SKILL_DIR/references/appendix.md` — Resource hierarchy, filter operators, usage limits, field configuration details
