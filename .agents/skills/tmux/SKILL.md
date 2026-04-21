---
name: tmux
description: Inspect tmux panes, survey which panes are active, capture pane output, and send text or key presses to a tmux target. Use when the user wants to monitor a tmux session, identify busy panes, read pane content, or message a pane.
triggers:
  intent_patterns:
    - "tmux|pane|session|监控 pane|发送到 pane|抓取 pane"
  context_signals:
    keywords: ["tmux", "pane", "session", "capture-pane", "send-keys"]
  confidence_threshold: 0.8
priority: 8
requires_tools: [bash]
max_tokens: 280
cooldown: 30
---

# tmux

Use this skill when the target terminal lives inside `tmux`. Prefer it over desktop UI automation because `tmux` exposes pane text directly and supports stable targeting.

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| List panes | `$PYTHON $SKILL_DIR/run.py list_panes` | none |
| Survey pane activity | `$PYTHON $SKILL_DIR/run.py survey --session 7 --lines 8` | `--session`, `--lines` |
| Inspect one pane | `$PYTHON $SKILL_DIR/run.py inspect --target %5 --lines 20` | `--target`, `--lines` |
| Watch for changes | `$PYTHON $SKILL_DIR/run.py watch --target %5 --ticks 6 --interval 2 --lines 6 --silence-secs 4` | `--target` or `--session`, `--ticks`, `--interval`, `--lines`, `--silence-secs` |
| Capture pane text | `$PYTHON $SKILL_DIR/run.py capture --target 5:1.1 --lines 80` | `--target`, `--lines` |
| Send literal text | `$PYTHON $SKILL_DIR/run.py send_text --target %5 --text 'cargo test'` | `--target`, `--text`, `--no-enter` |
| Send tmux keys | `$PYTHON $SKILL_DIR/run.py send_keys --target %5 --keys C-c,Enter --repeat 2` | `--target`, repeated `--keys`, `--repeat` |

## Workflow

1. Run `survey` first when the current information is insufficient; it returns `summary` plus grouped `running`, `idle`, and `dead` panes. Running panes include direct `content_lines` so you do not need a second inspect just to see what they are doing.
2. Use `inspect` on the most relevant pane to get real foreground process, recent activity age, and raw `preview`.
3. Use `watch` when you need a bounded polling loop that only reports meaningful changes across a pane or session. It now emits compact events with `changed` and `new_lines`.
4. Use `capture` when you need a larger raw scrollback.
5. Use `send_text` for commands and `send_keys` for control sequences such as `Enter` or `C-c`.

## Target Forms

- `session:window.pane` such as `5:1.1`
- `%pane_id` such as `%5`
- Positional forms also work for quick use, for example `send_keys %5 Enter` or `send_text %5 cargo test`

## Notes

- `survey` uses `tmux` metadata plus the pane TTY foreground process from `ps`, which is usually more accurate than `pane_current_command` alone.
- `inspect` and `survey` return a compact pane view by default. Raw pane metadata stays available through `list_panes`; `inspect` still includes `preview`.
- Prefer `survey` for user-facing answers: it already groups panes by state and gives a top-level summary, so avoid repeating raw process bookkeeping unless the user asked for it.
- `watch` is intentionally finite. It polls and returns `initial`, `events`, `final`, and `summary`; it does not run as a background daemon.
- `watch --silence-secs N` stops early after `N` seconds with no meaningful change, which is useful when a pane settles back into a prompt or a long-running task goes quiet.
- `send_keys` accepts repeated `--keys`, comma-separated keys, whitespace-separated keys, and `--repeat` for repeated key presses.

## Boundaries

- This skill can only read pane content that exists in `tmux`.
- It cannot inspect non-`tmux` terminal tabs or background daemons that do not write to the pane.
- Do not send destructive commands unless the user explicitly asked for them.
