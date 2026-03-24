---
name: feishu-doc
description: "飞书云文档的评论管理与媒体文件操作。支持查看/添加文档评论，以及插入本地图片或下载文档素材。"
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 快速索引

| 用户意图 | 工具 | 关键参数 |
|---------|------|---------|
| 查看文档评论 | feishu_doc_comments | action=list, document_id |
| 添加文档评论 | feishu_doc_comments | action=create, document_id, content |
| 解决/恢复评论 | feishu_doc_comments | action=patch, comment_id |
| 插入本地图片到文档 | feishu_doc_media | action=insert, document_id, file_path |
| 下载文档素材 | feishu_doc_media | action=download, token, output_path |

## 工具说明

### feishu_doc_comments
【以用户身份】管理云文档评论。支持: (1) list - 获取评论列表(含完整回复); (2) create - 添加全文评论(支持文本、@用户、超链接); (3) patch - 解决/恢复评论。支持 wiki token。

### feishu_doc_media
【以用户身份】文档媒体管理工具。支持两种操作：(1) insert - 在飞书文档末尾插入本地图片或文件（需要文档 ID + 本地文件路径）；(2) download - 下载文档素材或画板缩略图到本地（需要资源 token + 输出路径）。

【重要】insert 仅支持本地文件路径。URL 图片请使用 create-doc/update-doc 的 `<image url="..."/>` 语法。

## 不要这样做

- ❌ 用 feishu_doc_media insert 插入 URL 图片 → ✅ insert 仅支持本地文件，URL 图片用 create-doc/update-doc 的 `<image url="..."/>` 语法
- ❌ 用 feishu_doc_media 下载消息中的图片 → ✅ 消息中的图片用 feishu-im 的 feishu_im_bot_image 或 feishu_im_user_fetch_resource
- ❌ 用 feishu_doc_comments 操作电子表格的评论 → ✅ 本工具仅支持云文档（docx）评论
