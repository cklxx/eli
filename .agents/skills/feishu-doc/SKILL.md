---
name: feishu-doc
description: "Manage comments and media files on Feishu cloud documents. Supports viewing/adding comments and inserting local images or downloading document assets."
---

# feishu-doc

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Comment management and media file operations for Feishu cloud documents (docx).

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| View document comments | feishu_doc_comments | action=list, document_id |
| Add a document comment | feishu_doc_comments | action=create, document_id, content |
| Resolve/restore a comment | feishu_doc_comments | action=patch, comment_id |
| Insert local image into document | feishu_doc_media | action=insert, document_id, file_path |
| Download document asset | feishu_doc_media | action=download, token, output_path |

## Tools

### feishu_doc_comments
Manages cloud document comments (runs as the user). Supports: (1) list — get comment list with full replies; (2) create — add a document-level comment (supports text, @mentions, hyperlinks); (3) patch — resolve/restore a comment. Accepts wiki tokens.

### feishu_doc_media
Document media management tool (runs as the user). Supports two operations: (1) insert — append a local image or file to the end of a Feishu document (requires document ID + local file path); (2) download — download a document asset or whiteboard thumbnail to local storage (requires resource token + output path).

**Important:** insert only accepts local file paths. For URL images, use the `<image url="..."/>` syntax in create-doc/update-doc.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Using feishu_doc_media insert for URL images | insert only supports local files; use `<image url="..."/>` syntax in create-doc/update-doc for URLs |
| Using feishu_doc_media to download images from chat messages | For message images, use feishu_im_bot_image or feishu_im_user_fetch_resource from the feishu-im skill |
| Using feishu_doc_comments on spreadsheet comments | This tool only supports cloud document (docx) comments |
