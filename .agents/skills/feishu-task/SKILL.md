---
name: feishu-task
description: |
  飞书任务与清单管理。支持创建、分配、完成任务和管理清单。
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 执行前必读

- **时间格式**：ISO 8601（带时区），例如 `2026-02-28T17:00:00+08:00`
- **current_user_id**：从 SenderId 获取（ou_...），工具自动添加为 follower，确保创建者可编辑
- **完成任务**：`completed_at = "2026-02-26 15:00:00"`
- **反完成**：`completed_at = "0"`（字符串，不是数字）
- **patch/get 必须传** task_guid；**tasklist.tasks 必须传** tasklist_guid

## 快速索引

| 用户意图 | 工具 | action | 必填参数 | 强烈建议 | 常用可选 |
|---------|------|--------|---------|---------|---------|
| 新建待办 | feishu_task_task | create | summary | current_user_id（SenderId） | members, due, description |
| 查未完成任务 | feishu_task_task | list | - | completed=false | page_size |
| 获取任务详情 | feishu_task_task | get | task_guid | - | - |
| 完成任务 | feishu_task_task | patch | task_guid, completed_at | - | - |
| 反完成任务 | feishu_task_task | patch | task_guid, completed_at="0" | - | - |
| 改截止时间 | feishu_task_task | patch | task_guid, due | - | - |
| 创建清单 | feishu_task_tasklist | create | name | - | members |
| 查看清单任务 | feishu_task_tasklist | tasks | tasklist_guid | - | completed |
| 添加清单成员 | feishu_task_tasklist | add_members | tasklist_guid, members[] | - | - |

## 核心约束

### 用户身份与成员权限

工具使用 `user_access_token`（用户身份）。创建任务时如果没把自己加入成员，后续无法编辑。传入 `current_user_id` 后工具自动添加为 follower。

### 任务成员角色

- **assignee（负责人）**：负责完成任务，可编辑
- **follower（关注人）**：接收通知

```json
{
  "members": [
    {"id": "ou_xxx", "role": "assignee"},
    {"id": "ou_yyy", "role": "follower"}
  ]
}
```

### 清单创建人角色冲突

创建人自动成为清单 owner。如果 `members` 中包含创建人，该用户会从 members 中移除（同一用户只能有一个角色）。不要在 members 中包含创建人。

## 不要这样做

| 错误做法 | 正确做法 |
|---------|---------|
| 创建任务不传 current_user_id | 传 SenderId，工具自动添加为 follower |
| 反完成传数字 0 | 传字符串 `"0"` |
| 清单 members 包含创建人 | 创建人自动成为 owner，不要重复添加 |
| 时间用 Unix 时间戳 | 用 ISO 8601 格式 |

---

> 详细参考：使用 `fs.read` 读取
> - `$SKILL_DIR/references/examples.md` — 完整使用示例
> - `$SKILL_DIR/references/appendix.md` — 资源关系、GUID 获取、重复任务、权限模型、常见错误与排查
