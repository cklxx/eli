---
name: feishu-doc
description: "2 tools: feishu_doc_comments, feishu_doc_media"
---

Call tools via: sidecar(tool="<name>", params={...})

## feishu_doc_comments
【以用户身份】管理云文档评论。支持: (1) list - 获取评论列表(含完整回复); (2) create - 添加全文评论(支持文本、@用户、超链接); (3) patch - 解决/恢复评论。支持 wiki token。

## feishu_doc_media
【以用户身份】文档媒体管理工具。支持两种操作：(1) insert - 在飞书文档末尾插入本地图片或文件（需要文档 ID + 本地文件路径）；(2) download - 下载文档素材或画板缩略图到本地（需要资源 token + 输出路径）。

【重要】insert 仅支持本地文件路径。URL 图片请使用 create-doc/update-doc 的 <image url="..."/> 语法。

