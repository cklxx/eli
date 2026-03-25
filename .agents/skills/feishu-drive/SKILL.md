---
name: feishu-drive
description: "Manage files in Feishu Drive (cloud storage). Supports listing, uploading, downloading, copying, moving, and deleting files."
---

# feishu-drive

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

File management for Feishu Drive (cloud storage), including browsing, upload/download, and file operations.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| List folder contents | feishu_drive_file | action=list, folder_token |
| Batch query document metadata | feishu_drive_file | action=get_meta, request_docs |
| Upload file to Drive | feishu_drive_file | action=upload, file_path |
| Download file from Drive | feishu_drive_file | action=download, file_token, output_path |
| Copy/move/delete file | feishu_drive_file | action=copy/move/delete, file_token, type |

## Tools

### feishu_drive_file
Feishu Drive file management tool (runs as the user). Use when the user wants to browse cloud storage files, get file info, copy/move/delete files, or upload/download files. **Do not** use this tool for files in chat messages.

Actions:
- list: List files in a folder. Omit folder_token to list root directory contents
- get_meta: Batch query document metadata. Uses the request_docs array parameter, format: `[{doc_token: '...', doc_type: 'sheet'}]`
- copy: Copy a file to a specified location
- move: Move a file to a specified folder
- delete: Delete a file
- upload: Upload a local file to Drive. Provide file_path (local file path) or file_content_base64 (Base64 encoded)
- download: Download a file to local storage. Provide output_path (local save path) to save locally, otherwise returns Base64 encoded content

**Important:** copy/move/delete operations require both file_token and type parameters.

**Important:** upload prefers file_path (automatically reads file, extracts filename and size). Also supports file_content_base64 (requires manually providing file_name and size).

**Important:** download with output_path saves to local (can be a file path or folder path + file_name). Without output_path, returns Base64.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Using feishu_drive_file to read/write files from chat messages | For message files, use feishu_im_user_fetch_resource or feishu_im_bot_image from the feishu-im skill |
| Using feishu_drive_file to download embedded document assets | For embedded document assets, use feishu_doc_media download from the feishu-doc skill |
| Omitting the type parameter for copy/move/delete | Always provide both file_token and type (e.g., doc, sheet, bitable) |
