---
name: feishu-task
description: |
  Manage Feishu tasks and task lists -- create, assign, complete tasks, and organize them into lists.
---

# feishu-task

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Manages Feishu tasks and task lists, including creation, assignment, completion, and list membership.

## Prerequisites

- **Time format**: ISO 8601 with timezone, e.g. `2026-02-28T17:00:00+08:00`
- **current_user_id**: Obtain from SenderId (`ou_...`); the tool auto-adds this user as a follower so the creator can edit
- **Complete a task**: set `completed_at = "2026-02-26 15:00:00"`
- **Uncomplete a task**: set `completed_at = "0"` (string, not number)
- **patch/get require** `task_guid`; **tasklist.tasks requires** `tasklist_guid`

## Quick Reference

| Intent | Tool | Action | Required Params | Recommended | Optional |
|--------|------|--------|-----------------|-------------|----------|
| Create a task | feishu_task_task | create | summary | current_user_id (SenderId) | members, due, description |
| List incomplete tasks | feishu_task_task | list | -- | completed=false | page_size |
| Get task details | feishu_task_task | get | task_guid | -- | -- |
| Complete a task | feishu_task_task | patch | task_guid, completed_at | -- | -- |
| Uncomplete a task | feishu_task_task | patch | task_guid, completed_at="0" | -- | -- |
| Change due date | feishu_task_task | patch | task_guid, due | -- | -- |
| Create a task list | feishu_task_tasklist | create | name | -- | members |
| View list tasks | feishu_task_tasklist | tasks | tasklist_guid | -- | completed |
| Add list members | feishu_task_tasklist | add_members | tasklist_guid, members[] | -- | -- |

## Constraints

### User Identity and Permissions

The tool uses `user_access_token` (user identity). If you don't add yourself as a member when creating a task, you won't be able to edit it later. Passing `current_user_id` auto-adds the creator as a follower.

### Task Member Roles

- **assignee**: Responsible for completing the task; can edit
- **follower**: Receives notifications

```json
{
  "members": [
    {"id": "ou_xxx", "role": "assignee"},
    {"id": "ou_yyy", "role": "follower"}
  ]
}
```

### Task List Creator Role Conflict

The creator automatically becomes the list owner. If `members` includes the creator, that entry is removed (a user can only have one role). Do not include the creator in members.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Create a task without passing current_user_id | Pass SenderId so the tool auto-adds you as a follower |
| Uncomplete by passing numeric 0 | Pass the string `"0"` |
| Include the creator in task list members | Creator auto-becomes owner; don't add them again |
| Use Unix timestamps for time | Use ISO 8601 format |

---

> Detailed references: use `fs.read` to view
> - `$SKILL_DIR/references/examples.md` -- full usage examples
> - `$SKILL_DIR/references/appendix.md` -- resource relationships, GUID retrieval, recurring tasks, permission model, common errors and troubleshooting
