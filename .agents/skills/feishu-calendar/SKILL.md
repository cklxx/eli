---
name: feishu-calendar
description: |
  飞书日历与日程管理。包含日程创建、参会人管理、忙闲查询。
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 执行前必读

- **时区固定**：Asia/Shanghai（UTC+8）
- **时间格式**：ISO 8601（带时区），例如 `2026-02-25T14:00:00+08:00`
- **create 最小必填**：summary, start_time, end_time
- **user_open_id**：从 SenderId 获取（ou_xxx），确保发起人在参会人列表中
- **ID 格式**：用户 `ou_...`，群 `oc_...`，会议室 `omm_...`，邮箱 `email@...`

## 快速索引

| 用户意图 | 工具 | action | 必填参数 | 强烈建议 | 常用可选 |
|---------|------|--------|---------|---------|---------|
| 创建会议 | feishu_calendar_event | create | summary, start_time, end_time | user_open_id | attendees, description, location |
| 查某时间段日程 | feishu_calendar_event | list | start_time, end_time | - | - |
| 改日程时间 | feishu_calendar_event | patch | event_id, start_time/end_time | - | summary, description |
| 搜关键词找会 | feishu_calendar_event | search | query | - | - |
| 回复邀请 | feishu_calendar_event | reply | event_id, rsvp_status | - | - |
| 查重复日程实例 | feishu_calendar_event | instances | event_id, start_time, end_time | - | - |
| 查忙闲 | feishu_calendar_freebusy | list | time_min, time_max, user_ids[] | - | - |
| 邀请参会人 | feishu_calendar_event_attendee | create | calendar_id, event_id, attendees[] | - | - |

## 核心约束

### user_open_id 为什么必填

日程创建在用户主日历上，但不传 user_open_id 时发起人不会作为参会人出现。传入后：
- 发起人收到通知、可回复 RSVP、出现在参会人列表中

### instances 仅对重复日程有效

对普通日程调用 `instances` 会报错。先用 `get` 检查 `recurrence` 字段是否存在且非空。

### 会议室预约是异步流程

添加会议室后 `rsvp_status: "needs_action"`（预约中），最终变为 `accept` 或 `decline`。用 `event_attendee.list` 查询结果。

## 不要这样做

| 错误做法 | 正确做法 |
|---------|---------|
| 创建日程不传 user_open_id | 传 SenderId 确保发起人在参会人列表 |
| 时间用 Unix 时间戳 | 用 ISO 8601 格式 `2026-02-25T14:00:00+08:00` |
| 对普通日程调用 instances | instances 仅对重复日程有效，先 get 检查 recurrence 字段 |
| 混淆 open_id (ou_xxx) 和 attendee_id (user_xxx) | 始终用 open_id |

---

> 详细参考：使用 `fs.read` 读取
> - `$SKILL_DIR/references/examples.md` — 完整使用示例
> - `$SKILL_DIR/references/appendix.md` — 日历架构、参会人类型、权限模型、使用限制、常见错误与排查
