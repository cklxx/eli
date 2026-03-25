---
name: feishu-calendar
description: |
  Manage Feishu calendar events -- create events, manage attendees, and query free/busy status.
---

# feishu-calendar

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Manages Feishu calendar events, attendees, and free/busy queries.

## Prerequisites

- **Timezone**: Fixed to Asia/Shanghai (UTC+8)
- **Time format**: ISO 8601 with timezone, e.g. `2026-02-25T14:00:00+08:00`
- **create minimum required**: summary, start_time, end_time
- **user_open_id**: Obtain from SenderId (`ou_xxx`); ensures the organizer appears in the attendee list
- **ID formats**: user `ou_...`, group `oc_...`, meeting room `omm_...`, email `email@...`

## Quick Reference

| Intent | Tool | Action | Required Params | Recommended | Optional |
|--------|------|--------|-----------------|-------------|----------|
| Create an event | feishu_calendar_event | create | summary, start_time, end_time | user_open_id | attendees, description, location |
| List events in time range | feishu_calendar_event | list | start_time, end_time | -- | -- |
| Update event time | feishu_calendar_event | patch | event_id, start_time/end_time | -- | summary, description |
| Search events by keyword | feishu_calendar_event | search | query | -- | -- |
| Reply to invitation | feishu_calendar_event | reply | event_id, rsvp_status | -- | -- |
| List recurring event instances | feishu_calendar_event | instances | event_id, start_time, end_time | -- | -- |
| Query free/busy status | feishu_calendar_freebusy | list | time_min, time_max, user_ids[] | -- | -- |
| Invite attendees | feishu_calendar_event_attendee | create | calendar_id, event_id, attendees[] | -- | -- |

## Constraints

### Why user_open_id is essential

Events are created on the user's primary calendar, but without `user_open_id` the organizer won't appear as an attendee. When provided:
- The organizer receives notifications, can reply RSVP, and appears in the attendee list

### instances only works for recurring events

Calling `instances` on a non-recurring event returns an error. Use `get` first to check whether the `recurrence` field exists and is non-empty.

### Meeting room booking is asynchronous

After adding a meeting room, its `rsvp_status` is `"needs_action"` (booking in progress) and eventually changes to `accept` or `decline`. Use `event_attendee.list` to check the result.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Create an event without passing user_open_id | Pass SenderId to ensure the organizer is in the attendee list |
| Use Unix timestamps for time values | Use ISO 8601 format: `2026-02-25T14:00:00+08:00` |
| Call instances on a non-recurring event | instances only works for recurring events; check the recurrence field with get first |
| Confuse open_id (`ou_xxx`) with attendee_id (`user_xxx`) | Always use open_id |

---

> Detailed references: use `fs.read` to view
> - `$SKILL_DIR/references/examples.md` -- full usage examples
> - `$SKILL_DIR/references/appendix.md` -- calendar architecture, attendee types, permission model, usage limits, common errors and troubleshooting
