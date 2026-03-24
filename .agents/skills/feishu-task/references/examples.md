# Task 使用场景示例

## 场景 1: 创建任务并分配负责人

```json
{
  "action": "create",
  "summary": "准备周会材料",
  "description": "整理本周工作进展和下周计划",
  "current_user_id": "ou_发送者的open_id",
  "due": {
    "timestamp": "2026-02-28 17:00:00",
    "is_all_day": false
  },
  "members": [
    {"id": "ou_协作者的open_id", "role": "assignee"}
  ]
}
```

**说明**：
- `summary` 是必填字段
- `current_user_id` 强烈建议传入（从 SenderId 获取），工具会自动添加为 follower
- `members` 可以只包含其他协作者，当前用户会被自动添加
- 时间使用北京时间字符串格式

## 场景 2: 查询我负责的未完成任务

```json
{
  "action": "list",
  "completed": false,
  "page_size": 20
}
```

## 场景 3: 完成任务

```json
{
  "action": "patch",
  "task_guid": "任务的guid",
  "completed_at": "2026-02-26 15:30:00"
}
```

## 场景 4: 反完成任务（恢复未完成状态）

```json
{
  "action": "patch",
  "task_guid": "任务的guid",
  "completed_at": "0"
}
```

## 场景 5: 创建清单并添加协作者

```json
{
  "action": "create",
  "name": "产品迭代 v2.0",
  "members": [
    {"id": "ou_xxx", "role": "editor"},
    {"id": "ou_yyy", "role": "viewer"}
  ]
}
```

## 场景 6: 查看清单内的未完成任务

```json
{
  "action": "tasks",
  "tasklist_guid": "清单的guid",
  "completed": false
}
```

## 场景 7: 全天任务

```json
{
  "action": "create",
  "summary": "年度总结",
  "due": {
    "timestamp": "2026-03-01 00:00:00",
    "is_all_day": true
  }
}
```
