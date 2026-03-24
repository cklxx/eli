---
name: feishu-drive
description: "飞书云空间（云盘）文件管理。支持文件列表查看、上传下载、复制移动删除等操作。"
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 快速索引

| 用户意图 | 工具 | 关键参数 |
|---------|------|---------|
| 列出文件夹内容 | feishu_drive_file | action=list, folder_token |
| 批量查询文档元信息 | feishu_drive_file | action=get_meta, request_docs |
| 上传文件到云空间 | feishu_drive_file | action=upload, file_path |
| 下载云空间文件 | feishu_drive_file | action=download, file_token, output_path |
| 复制/移动/删除文件 | feishu_drive_file | action=copy/move/delete, file_token, type |

## 工具说明

### feishu_drive_file
【以用户身份】飞书云空间文件管理工具。当用户要求查看云空间(云盘)中的文件列表、获取文件信息、复制/移动/删除文件、上传/下载文件时使用。消息中的文件读写**禁止**使用该工具!

Actions:
- list（列出文件）：列出文件夹下的文件。不提供 folder_token 时获取根目录清单
- get_meta（批量获取元数据）：批量查询文档元信息，使用 request_docs 数组参数，格式：[{doc_token: '...', doc_type: 'sheet'}]
- copy（复制文件）：复制文件到指定位置
- move（移动文件）：移动文件到指定文件夹
- delete（删除文件）：删除文件
- upload（上传文件）：上传本地文件到云空间。提供 file_path（本地文件路径）或 file_content_base64（Base64 编码）
- download（下载文件）：下载文件到本地。提供 output_path（本地保存路径）则保存到本地，否则返回 Base64 编码

【重要】copy/move/delete 操作需要 file_token 和 type 参数。get_meta 使用 request_docs 数组参数。
【重要】upload 优先使用 file_path（自动读取文件、提取文件名和大小），也支持 file_content_base64（需手动提供 file_name 和 size）。
【重要】download 提供 output_path 时保存到本地（可以是文件路径或文件夹路径+file_name），不提供则返回 Base64。

## 不要这样做

- ❌ 用 feishu_drive_file 读写消息中的文件 → ✅ 消息中的文件用 feishu-im 的 feishu_im_user_fetch_resource 或 feishu_im_bot_image
- ❌ 用 feishu_drive_file 下载文档内嵌的素材 → ✅ 文档内嵌素材用 feishu-doc 的 feishu_doc_media download
- ❌ copy/move/delete 时不传 type 参数 → ✅ 必须同时提供 file_token 和 type（如 doc、sheet、bitable 等）
