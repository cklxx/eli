---
name: feishu-troubleshoot
description: Troubleshoot Feishu plugin issues. Includes a FAQ for common problems and deep diagnostic commands.
---

# feishu-troubleshoot

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Diagnose and resolve Feishu plugin issues. Covers common problems (FAQ) and deep permission diagnostics via the `/feishu doctor` command.

## FAQ

### Card button clicks have no effect

**Symptom**: Clicking a card button does nothing, then shows an error.

**Cause**: The app has not enabled the "Message Card Callback Interaction" permission.

**Resolution**:

1. Log in to the Feishu Open Platform: https://open.feishu.cn/app
2. Select your app, then go to **Events & Callbacks**
3. In the callback configuration, change the subscription method to "Long Connection" and add the callback "Card Callback Interaction" (card.action.trigger)
4. Create an app version, submit for review, and publish

## Diagnostic Commands

**Note**: Diagnostic commands are only for troubleshooting complex/difficult **permission-related issues**. Routine permission problems trigger the automatic authorization flow and do not require manual diagnosis.

**When to use diagnostics**:
- Authorization still fails after multiple attempts
- The automatic authorization flow cannot resolve the issue
- You need to see the full permission configuration state

**Usage**:

Enter the following as a user message in a Feishu chat session:

`/feishu doctor`

The diagnostic command checks:

- **Diagnostic Summary** (shown first):
  - Overall status (OK / Warning / Failed)
  - List of discovered issues with brief descriptions

- **Environment Info**:
  - Plugin version

- **Account Info**:
  - Credential completeness (appId, appSecret masked)
  - Account enabled status
  - API connectivity test
  - Bot info (name and openId)

- **App Identity Permissions**:
  - Number of required permissions the app has enabled
  - List of missing required permissions
  - One-click application link (auto-populated with missing permissions)

- **User Identity Permissions**:
  - User authorization status summary (valid / needs refresh / expired)
  - Token auto-refresh status (whether offline_access is included)
  - Permission comparison table (app-enabled vs user-authorized, item by item)
  - Application guide and link when app permissions are missing
  - Re-authorization instructions when user authorization is insufficient

## Pitfalls

| Wrong | Right |
|-------|-------|
| Run diagnostic commands for routine permission issues | Routine permission issues are handled by the automatic authorization flow |
| Execute `/feishu doctor` on behalf of the user | The diagnostic command must be entered by the user themselves in the Feishu chat |
