# feishu-im-read 使用示例

## 场景 1: 获取群聊消息并展开话题

**步骤 1**：获取群聊消息
```json
{ "chat_id": "oc_xxx" }
```

**步骤 2**：返回的消息中发现 `thread_id`，展开话题最新回复：
```json
{ "thread_id": "omt_xxx", "page_size": 10, "sort_rule": "create_time_desc" }
```

## 场景 2: 跨会话搜索消息

```json
{ "query": "项目进度", "chat_id": "oc_xxx" }
```

## 场景 3: 分页获取更多消息

第一次调用返回 `has_more: true` 和 `page_token: "xxx"`，继续获取：
```json
{ "chat_id": "oc_xxx", "page_token": "xxx" }
```

## 场景 4: 下载消息中的资源

```json
{ "message_id": "om_xxx", "file_key": "img_v3_xxx", "type": "image" }
```
