---
name: browser-use
description: Automate Chrome via Playwright MCP Extension Relay, reusing logged-in cookies and sessions.
triggers:
  intent_patterns:
    - "浏览器|browser|网页|webpage|打开网站|open.*url|截图|screenshot|x\\.com|twitter"
  context_signals:
    keywords: ["浏览器", "browser", "网页", "截图", "打开", "navigate", "snapshot"]
  confidence_threshold: 0.7
priority: 6
requires_tools: [bash]
max_tokens: 4000
cooldown: 10
---

# browser-use

Control the user's current Chrome browser via Playwright MCP Extension Relay, reusing logged-in sessions and cookies.

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| Open a URL | `navigate` | `--url` |
| Get page structure | `snapshot` | — |
| Click an element | `click` | `--ref`, `--element` |
| Type into a field | `type` | `--ref`, `--text`, `--submit` |
| Take a screenshot | `screenshot` | `--filename` |
| List open tabs | `tabs list` | — |
| Run JavaScript | `evaluate` | `--function` |
| Run Playwright code | `run_code` | `--code` |
| Press a key | `press_key` | `--key` |
| Wait for text | `wait_for` | `--text` |

## Prerequisites

1. Chrome has the [Playwright MCP Bridge](https://chromewebstore.google.com/detail/playwright-mcp-bridge/mmlmfjhmonkocbjadbfplnigmagldckm) extension installed.
2. `ALEX_BROWSER_BRIDGE_TOKEN` is set in `.env` (copy from the extension popup).

## Usage

```bash
# Navigate to URL
python3 $SKILL_DIR/run.py navigate --url https://x.com

# Get page snapshot (accessibility tree)
python3 $SKILL_DIR/run.py snapshot

# Click element (ref from snapshot)
python3 $SKILL_DIR/run.py click --ref e44 --element 'Home link'

# Type text
python3 $SKILL_DIR/run.py type --ref e100 --text hello --submit true

# Screenshot
python3 $SKILL_DIR/run.py screenshot --filename page.png

# List tabs
python3 $SKILL_DIR/run.py tabs list

# Execute JavaScript
python3 $SKILL_DIR/run.py evaluate --function '() => document.title'

# Execute Playwright code
python3 $SKILL_DIR/run.py run_code --code 'async (page) => await page.title()'

# Press key
python3 $SKILL_DIR/run.py press_key --key Enter

# Wait for text to appear
python3 $SKILL_DIR/run.py wait_for --text 'Loading complete'
```

## Typical Workflow

1. `navigate` — open the target page.
2. `snapshot` — get page structure and element refs.
3. `click` / `type` — interact with elements.
4. `snapshot` — confirm the result.
