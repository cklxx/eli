---
name: feishu-oauth
description: Batch-authorize all Feishu permissions at once. Use only when the user explicitly requests a one-time full authorization.
---

# feishu-oauth

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Perform a one-time batch authorization of all user permissions configured for the Feishu app. Only use when the user explicitly requests authorizing all permissions at once.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| Authorize all permissions at once | feishu_oauth_batch_auth | (none required) |

## Tools

### feishu_oauth_batch_auth

Batch-authorize all user permissions that the Feishu app has enabled. Use only when the user explicitly says "authorize all permissions" or "batch authorize".

## Pitfalls

| Wrong | Right |
|-------|-------|
| Call batch_auth when authorization fails or expires | Only use when user explicitly requests "authorize all permissions"; auth failures/expiry are handled automatically by the system |
| Call batch_auth when user says "authorize" generically | Normal single-permission authorization pops up automatically; batch_auth is only for "authorize all" scenarios |
| Confuse batch_auth with feishu skill's feishu_oauth revoke | batch_auth grants permissions; revoke removes them — opposite operations |
