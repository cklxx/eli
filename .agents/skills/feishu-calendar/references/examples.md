# Calendar 使用场景示例

## 场景 1: 创建会议并邀请参会人

```json
{
  "action": "create",
  "summary": "项目复盘会议",
  "description": "讨论 Q1 项目进展",
  "start_time": "2026-02-25 14:00:00",
  "end_time": "2026-02-25 15:30:00",
  "user_open_id": "ou_aaa",
  "attendees": [
    {"type": "user", "id": "ou_bbb"},
    {"type": "user", "id": "ou_ccc"},
    {"type": "resource", "id": "omm_xxx"}
  ]
}
```

## 场景 2: 查询用户未来一周的日程

```json
{
  "action": "list",
  "start_time": "2026-02-25 00:00:00",
  "end_time": "2026-03-03 23:59:00"
}
```

## 场景 3: 查看多个用户的忙闲时间

```json
{
  "action": "list",
  "time_min": "2026-02-25 09:00:00",
  "time_max": "2026-02-25 18:00:00",
  "user_ids": ["ou_aaa", "ou_bbb", "ou_ccc"]
}
```

**注意**：user_ids 是数组，支持 1-10 个用户。当前不支持会议室忙闲查询。

## 场景 4: 修改日程时间

```json
{
  "action": "patch",
  "event_id": "xxx_0",
  "start_time": "2026-02-25 15:00:00",
  "end_time": "2026-02-25 16:00:00"
}
```

## 场景 5: 搜索日程（按关键词）

```json
{
  "action": "search",
  "query": "项目复盘"
}
```

## 场景 6: 回复日程邀请

```json
{
  "action": "reply",
  "event_id": "xxx_0",
  "rsvp_status": "accept"
}
```
