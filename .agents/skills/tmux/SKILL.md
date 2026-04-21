---
name: tmux
description: Inspect tmux panes, survey which panes are active, capture pane output, and send text or key presses to a tmux target. Use when the user wants to monitor a tmux session, identify busy panes, read pane content, or message a pane.
triggers:
  intent_patterns:
    - "tmux|pane|session|监控 pane|发送到 pane|抓取 pane"
    - "都在干嘛|谁在跑|哪个在跑|哪些在跑|看下 session|看下 pane"
    - "盯着|持续看|有变化告诉我|watch 一下|监控一下"
    - "给 .* 发消息|给 .* 发 continue|发到 pane|发回车|按 enter|按 ctrl-c|发送按键"
    - "抓最近输出|看最近日志|看最近内容|看详细输出|展开看"
  context_signals:
    keywords: ["tmux", "pane", "session", "capture-pane", "send-keys", "watch", "continue", "Enter", "C-c"]
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
| Send command and read feedback | `$PYTHON $SKILL_DIR/run.py send_command --target %5 --text 'cargo test' --lines 12` | `--target`, `--text`, `--keys`, `--wait_secs`, `--lines` |
| Send literal text | `$PYTHON $SKILL_DIR/run.py send_text --target %5 --text 'cargo test'` | `--target`, `--text`, `--no-enter` |
| Send tmux keys | `$PYTHON $SKILL_DIR/run.py send_keys --target %5 --keys C-c,Enter --repeat 2` | `--target`, repeated `--keys`, `--repeat` |

## Workflow

1. Run `survey` first when the current information is insufficient; it returns `summary` plus grouped `running`, `idle`, and `dead` panes. Running panes include direct `content_lines` so you do not need a second inspect just to see what they are doing.
2. Use `inspect` on the most relevant pane to get real foreground process, recent activity age, and raw `preview`.
3. Use `watch` when you need a bounded polling loop that only reports meaningful changes across a pane or session. It now emits compact events with `changed` and `new_lines`.
4. Use `capture` when you need a larger raw scrollback.
5. Use `send_command` when the user wants to run something in a pane and expects immediate feedback; it types the command, presses launch keys, waits briefly, then returns a compact pane summary.
6. Use `send_text` only when you intentionally want to type without necessarily executing yet.
7. Use `send_keys` for pure control sequences such as `Enter`, `C-c`, or repeated key presses.

## When to Trigger

Trigger this skill as soon as the user is asking about a `tmux`-hosted terminal, even if they do not say `tmux` explicitly but the context already points to pane/session work.

- **状态判断**: "都在活动吗", "谁在跑", "看下 agent-infer 里哪个 codex 还活着"
- **内容理解**: "这些 pane 现在在干嘛", "抓 `%7` 最近输出", "把 running 的内容总结一下"
- **变化监控**: "盯一下 `%9`", "有变化告诉我", "watch 一下这个 session"
- **发消息/按键**: "给 `%7` 发 continue", "往 pane 里发 `cargo test`", "按一下 Enter/C-c"
- **运行命令并看结果**: "在 `%7` 里跑 `cargo test` 然后告诉我结果", "给 pane 发命令并回我反馈"
- **二次确认**: 用户先给了 pane 列表或 TTY 信息，接着问 "现在怎样了"、"继续看"

Do not trigger this skill for non-`tmux` desktop tabs, GUI terminals without `tmux`, or background daemons that are not writing to a pane.

## Response Shape

Default to a conclusion-first answer. Do not narrate pane-discovery mechanics unless the user explicitly asks for them.

1. Start with the answer the user actually wants: how many panes are running, which ones matter, or whether it is safe to message.
2. For each relevant running pane, summarize what it is doing from `summary` and `content_lines`, not from raw TTY/process bookkeeping.
3. Mention idle panes only when they are actionable, for example "safe to message".
4. Only surface raw details such as `tty`, `pane_current_command`, unmatched background processes, or invisible panes when they change the decision.

Good:
- "`agent-infer` 里现在明确在跑的是 `%7` 和 `%9`。`%7` 在做 scheduler review，`%9` 还在跑 benchmark。"
- "`%4` 现在是 idle shell，可以直接发消息。"

Bad:
- "我看到 `ttys000` 还有一组 node，`%6` 之前没了，pane_current_command 是 ..."
- "先列了 pane，再比对了 TTY，再看了 ps ..."

## Next-Step Guidance

After using this skill, always end with the single most useful next step instead of a generic "要不要继续看".

- If `survey` shows **running panes**, suggest one of:
  - "可以直接抓 `%7` 最近 20 行详细输出。"
  - "可以只盯 `%7` / `%9`，有变化再汇报。"
- If the user needs **more context on one pane**, move to `inspect`.
- If the user needs **ongoing observation**, move to `watch`.
- If a pane is **idle and actionable**, suggest:
  - "可以直接给 `%4` 发 `continue`。"
  - "可以发 `Enter` / `C-c` 到 `%4`。"
- If the user wants to **run a command and know what happened**, prefer:
  - "可以直接发命令到 `%7`，按 `Enter` 启动，然后把最新反馈抓回来。"
- If the pane is **active**, avoid suggesting messages unless the user explicitly wants to interrupt it.

Preferred follow-up prompts:
- "`%7` 还在跑 scheduler review。可以直接抓它最近 20 行，或者继续 watch 它的变化。"
- "`%4` 现在空闲，可以直接发命令进去。"
- "`%9` 还在忙，不建议现在打断；如果你要，我可以只盯它到空闲为止。"

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
- `send_command` is the default choice for "run this in the pane and tell me what happened". It sends the text, launches it with `Enter` by default, then returns feedback from `inspect`.
- `send_keys` accepts repeated `--keys`, comma-separated keys, whitespace-separated keys, and `--repeat` for repeated key presses.

## Boundaries

- This skill can only read pane content that exists in `tmux`.
- It cannot inspect non-`tmux` terminal tabs or background daemons that do not write to the pane.
- Do not send destructive commands unless the user explicitly asked for them.
