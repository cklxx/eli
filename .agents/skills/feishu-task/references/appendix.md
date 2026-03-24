# Task 背景知识与限制

## 资源关系

```
任务清单（Tasklist）
  └─ 自定义分组（Section，可选）
      └─ 任务（Task）
          ├─ 成员：负责人（assignee）、关注人（follower）
          ├─ 子任务（Subtask）
          ├─ 截止时间（due）、开始时间（start）
          └─ 附件、评论
```

**核心概念**：
- **任务（Task）**：独立的待办事项，有唯一的 `task_guid`
- **清单（Tasklist）**：组织多个任务的容器，有唯一的 `tasklist_guid`
- **负责人（assignee）**：可以编辑任务并标记完成
- **关注人（follower）**：接收任务更新通知
- **我负责的（MyTasks）**：所有负责人为自己的任务集合

## 如何获取 GUID

- **task_guid**：创建任务后从返回值的 `task.guid` 获取，或通过 `list` 查询
- **tasklist_guid**：创建清单后从返回值的 `tasklist.guid` 获取，或通过 `list` 查询

## 如何将任务加入清单

创建任务时指定 `tasklists` 参数：
```json
{
  "action": "create",
  "summary": "任务标题",
  "tasklists": [
    {
      "tasklist_guid": "清单的guid",
      "section_guid": "分组的guid（可选）"
    }
  ]
}
```

## 重复任务

使用 `repeat_rule` 参数，采用 RRULE 格式：
```json
{
  "action": "create",
  "summary": "每周例会",
  "due": {"timestamp": "2026-03-03 14:00:00", "is_all_day": false},
  "repeat_rule": "FREQ=WEEKLY;INTERVAL=1;BYDAY=MO"
}
```

**说明**：只有设置了截止时间的任务才能设置重复规则。

## 清单成员角色

| 成员类型 | 角色 | 说明 |
|---------|------|------|
| user（用户） | owner | 所有者，可转让所有权 |
| user（用户） | editor | 可编辑，可修改清单和任务 |
| user（用户） | viewer | 可查看，只读权限 |
| chat（群组） | editor/viewer | 整个群组获得权限 |

**说明**：创建清单时，创建者自动成为 owner，无需在 members 中指定。

## completed_at 的三种用法

1. **完成任务**：传北京时间字符串 `"2026-02-26 15:30:00"`
2. **反完成**：传特殊值 `"0"`（字符串）
3. **毫秒时间戳**：`"1740545400000"`（不推荐）

## 数据权限

- 只能操作自己有权限的任务（作为成员的任务）
- 只能操作自己有权限的清单（作为成员的清单）
- 将任务加入清单需要同时拥有任务和清单的编辑权限

## 常见错误与排查

| 错误现象 | 根本原因 | 解决方案 |
|---------|---------|---------|
| **创建后无法编辑任务** | 创建时未将自己加入 members | 创建时至少将当前用户（SenderId）加为 assignee 或 follower |
| **patch 失败提示 task_guid 缺失** | 未传 task_guid 参数 | patch/get 必须传 task_guid |
| **tasks 失败提示 tasklist_guid 缺失** | 未传 tasklist_guid 参数 | tasklist.tasks action 必须传 tasklist_guid |
| **反完成失败** | completed_at 格式错误 | 使用 `"0"` 字符串，不是数字 0 |
| **时间不对** | 使用了 Unix 时间戳 | 改用 ISO 8601 格式（带时区）：`2024-01-01T00:00:00+08:00` |
