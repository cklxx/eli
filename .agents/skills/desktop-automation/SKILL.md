---
name: desktop-automation
description: Automate macOS desktop actions via AppleScript to control apps like Chrome, Finder, and others.
triggers:
  intent_patterns:
    - "applescript|桌面自动化|desktop|macos|打开应用|切换窗口"
  context_signals:
    keywords: ["applescript", "desktop", "macos", "自动化", "窗口"]
  confidence_threshold: 0.7
priority: 6
requires_tools: [bash]
max_tokens: 200
cooldown: 60
---

# desktop-automation

macOS desktop automation: run AppleScript to control applications.

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| Run AppleScript | `python3 $SKILL_DIR/run.py run --script '...'` | `--script` |
| Open an app | `python3 $SKILL_DIR/run.py open_app --app Safari` | `--app` |

## Usage

```bash
python3 $SKILL_DIR/run.py run --script 'tell application "Finder" to activate'
python3 $SKILL_DIR/run.py open_app --app Safari
```
