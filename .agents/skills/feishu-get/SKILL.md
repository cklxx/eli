---
name: feishu-get
description: "Look up detailed info for a known Feishu user. Pass a user_id for a specific user, or omit it to query the current user."
---

# feishu-get

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Retrieve detailed user information by ID, or query the current user's own profile.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| View own info | feishu_get_user | (no params) |
| View a specific user's info | feishu_get_user | user_id |

## Tools

### feishu_get_user
Retrieves user information. Omit user_id to get the current user's own info; provide user_id to look up a specific user. Returns name, avatar, email, phone number, department, and more.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Using feishu_get_user to search for users | Use feishu_search_user from the feishu-search skill; get_user only works with a known ID or for querying yourself |
| Asking the user to provide a user_id when you don't have one | First use feishu_search_user from the feishu-search skill to search by name and obtain the open_id |
