---
name: feishu-fetch-doc
description: Fetch Feishu cloud document content as Markdown, with support for images, files, and whiteboards.
---

# feishu-fetch-doc

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Fetches Feishu cloud document content in Lark-flavored Markdown format. Media items (images, files, whiteboards) are returned as token references and must be downloaded separately.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| Fetch document text | `feishu_mcp_fetch_doc` | doc_id |
| Download image/file/whiteboard | `feishu_doc_media` | action=download, resource_token, resource_type=media, output_path |
| Resolve wiki token type | `feishu_wiki_space_node` | action=get, node_token |
| Read/write spreadsheet | `feishu_sheet` | spreadsheet_token |
| Operate on bitable | `feishu_bitable_*` | app_token |

## Tools

### feishu_mcp_fetch_doc

Fetches the Markdown content of a Feishu cloud document.

**Parameters:**

- **`doc_id`** (required): Accepts a document URL or token directly.
  - URL: `https://xxx.feishu.cn/docx/Z1FjxxxxxxxxxxxxxxxxxxxtnAc` (token extracted automatically)
  - Token: `Z1FjxxxxxxxxxxxxxxxxxxxtnAc`
  - Wiki URL/token also supported: `https://xxx.feishu.cn/wiki/Z1FjxxxxxxxxxxxxxxxxxxxtnAc`

### feishu_doc_media (action: download)

Downloads media items referenced in the document. Images, files, and whiteboards appear in the returned Markdown as HTML tags:

- **Images**: `<image token="..." width="..." height="..." align="..."/>`
- **Files**: `<view type="1"><file token="..." name="..."/></view>`
- **Whiteboards**: `<whiteboard token="..."/>`

**Steps to download:**

1. Extract the `token` attribute from the HTML tag
2. Call `feishu_doc_media` with action=download, resource_token=extracted_token, resource_type=media, output_path=save_path

## Wiki URL Handling

Wiki links (`/wiki/TOKEN`) can point to different document types (cloud doc, spreadsheet, bitable, etc.). You must resolve the actual type before fetching.

**Workflow:**

1. Call `feishu_wiki_space_node` (action=get) to resolve the wiki token
2. Read `obj_type` and `obj_token` from the returned `node`
3. Call the appropriate tool based on `obj_type`:

| obj_type | Tool | Parameter |
|----------|------|-----------|
| `docx` | `feishu_mcp_fetch_doc` | doc_id = obj_token |
| `sheet` | `feishu_sheet` | spreadsheet_token = obj_token |
| `bitable` | `feishu_bitable_*` series | app_token = obj_token |
| other | Inform user this type is not supported | -- |

> See `$SKILL_DIR/references/examples.md` for full examples.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Treat a wiki URL directly as a docx and fetch it | Use `feishu_wiki_space_node` get to resolve `obj_type` first |
| Expect `fetch_doc` to return image content inline | Images are returned as token tags; use `feishu_doc_media` download to retrieve them |
| Ignore `<image>`/`<file>`/`<whiteboard>` tags in the output | Extract tokens and download as needed, or inform the user |
