#!/usr/bin/env python3
"""tmux skill — inspect panes and send input."""

from __future__ import annotations

import os
from pathlib import Path
import re
import subprocess
import sys
import time
from typing import Any

_SCRIPTS_DIR = Path(__file__).resolve().parents[2] / "scripts"
if str(_SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(_SCRIPTS_DIR))

from skill_runner.env import load_repo_dotenv
from skill_runner.cli_contract import parse_cli_args, render_result

load_repo_dotenv(__file__)

_PANE_FIELDS = [
    "session",
    "window",
    "pane",
    "pane_id",
    "tty",
    "command",
    "start_command",
    "title",
    "path",
    "history_size",
    "attached",
    "activity_epoch",
    "active",
    "dead",
]
_PANE_FORMAT = "\t".join(
    [
        "#{session_name}",
        "#{window_index}",
        "#{pane_index}",
        "#{pane_id}",
        "#{pane_tty}",
        "#{pane_current_command}",
        "#{pane_start_command}",
        "#{pane_title}",
        "#{pane_current_path}",
        "#{history_size}",
        "#{session_attached}",
        "#{window_activity}",
        "#{pane_active}",
        "#{pane_dead}",
    ]
)
_SHELL_NAMES = {"bash", "fish", "sh", "tmux", "zsh"}
_ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
_SEPARATOR_CHARS = set("─━═-│┃┆┊╭╮╰╯•· ")
_SIGNAL_RULES = [
    ("error", ("error", "failed", "panic", "traceback")),
    ("warning", ("warn", "warning")),
    ("test_ok", ("test result: ok", "passed", "all doctests ran")),
    ("build", ("compiling ", "cargo build", "cargo test", "pytest ")),
    ("review", ("codex review", "/review")),
    ("plan", ("updated plan", "final session summary")),
    ("waiting", ("waited for background terminal", "press ctrl-c again to exit")),
    ("approval_prompt", ("[y/n]", "[y/N]", "approve", "continue?", "press enter to continue")),
    ("tui_active", ("working (", "esc to interrupt")),
]
_NOISE_MARKERS = (
    "searching the web",
    "searched ",
    "explored",
    "read ",
    "if the browser didn't open",
)
_STATUS_MARKERS = ("working (", "esc to interrupt", "gpt-5.4", "claude code")


def _tmux(command: list[str]) -> dict[str, Any]:
    try:
        result = subprocess.run(
            ["tmux", *command],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
    except FileNotFoundError:
        return {"success": False, "error": "tmux not found"}
    except subprocess.TimeoutExpired:
        return {"success": False, "error": "tmux command timed out"}
    return _result_from_process(result, command)


def _result_from_process(
    result: subprocess.CompletedProcess[str],
    command: list[str],
) -> dict[str, Any]:
    if result.returncode == 0:
        return {"success": True, "stdout": result.stdout.rstrip("\n")}
    message = result.stderr.strip() or result.stdout.strip()
    fallback = f"tmux {' '.join(command)} failed"
    return {"success": False, "error": message or fallback}


def _target(args: dict[str, Any]) -> str:
    explicit = str(args.get("target", "")).strip()
    if explicit:
        return explicit
    return _positionals(args)[:1][0] if _positionals(args) else ""


def _text(args: dict[str, Any]) -> str:
    if "text" in args:
        return str(args.get("text", ""))
    return " ".join(_tail_positionals(args))


def _session(args: dict[str, Any]) -> str:
    return str(args.get("session", "")).strip()


def _keys(args: dict[str, Any]) -> list[str]:
    raw = args.get("keys", _tail_positionals(args))
    items = raw if isinstance(raw, list) else [raw]
    return [key for item in items for key in _split_keys(str(item))]


def _lines(args: dict[str, Any]) -> int | None:
    try:
        value = int(args.get("lines", 80))
    except (TypeError, ValueError):
        return None
    return value if value > 0 else None


def _positionals(args: dict[str, Any]) -> list[str]:
    raw = args.get("positionals", [])
    values = raw if isinstance(raw, list) else [raw]
    return [str(item).strip() for item in values if str(item).strip()]


def _tail_positionals(args: dict[str, Any]) -> list[str]:
    values = _positionals(args)
    return values[1:] if values and "target" not in args else values


def _split_keys(raw: str) -> list[str]:
    parts = re.split(r"[\s,]+", raw.strip())
    return [part for part in parts if part]


def _ticks(args: dict[str, Any]) -> int | None:
    try:
        value = int(args.get("ticks", 5))
    except (TypeError, ValueError):
        return None
    return value if value > 0 else None


def _interval(args: dict[str, Any]) -> float | None:
    try:
        value = float(args.get("interval", 2))
    except (TypeError, ValueError):
        return None
    return value if value > 0 else None


def _repeat(args: dict[str, Any]) -> int | None:
    try:
        value = int(args.get("repeat", 1))
    except (TypeError, ValueError):
        return None
    return value if value > 0 else None


def _pane_record(line: str) -> dict[str, Any]:
    values = dict(zip(_PANE_FIELDS, line.split("\t"), strict=False))
    return {
        "session": values["session"],
        "window": int(values["window"]),
        "pane": int(values["pane"]),
        "pane_id": values["pane_id"],
        "tty": values["tty"],
        "command": values["command"],
        "start_command": values["start_command"],
        "title": values["title"],
        "path": values["path"],
        "history_size": int(values["history_size"] or 0),
        "attached": values["attached"] == "1",
        "activity_epoch": int(values["activity_epoch"] or 0),
        "active": values["active"] == "1",
        "dead": values["dead"] == "1",
    }


def _target_of(pane: dict[str, Any]) -> str:
    return f'{pane["session"]}:{pane["window"]}.{pane["pane"]}'


def _capture_text(target: str, lines: int) -> dict[str, Any]:
    return _tmux(["capture-pane", "-J", "-p", "-S", f"-{lines}", "-t", target])


def _tty_name(tty: str) -> str:
    return tty.rsplit("/", 1)[-1]


def _ps_row(line: str) -> dict[str, Any]:
    pid, pgid, tpgid, stat, command = line.strip().split(None, 4)
    return {
        "pid": int(pid),
        "pgid": int(pgid),
        "tpgid": int(tpgid),
        "stat": stat,
        "command": command,
    }


def _ps_rows(tty: str) -> list[dict[str, Any]]:
    process = subprocess.run(
        [
            "ps",
            "-t",
            _tty_name(tty),
            "-o",
            "pid=",
            "-o",
            "pgid=",
            "-o",
            "tpgid=",
            "-o",
            "stat=",
            "-o",
            "command=",
        ],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    if process.returncode != 0:
        return []
    return [_ps_row(line) for line in process.stdout.splitlines() if line.strip()]


def _command_name(command: str) -> str:
    head = command.split()[0] if command.split() else command
    return os.path.basename(head.lstrip("-"))


def _foreground(rows: list[dict[str, Any]]) -> dict[str, Any] | None:
    matches = [row for row in rows if row["pgid"] == row["tpgid"]]
    return matches[0] if matches else None


def _clean_preview(text: str) -> str:
    return _ANSI_RE.sub("", text).replace("\r", "").strip("\n")


def _is_separator(line: str) -> bool:
    stripped = line.strip()
    return bool(stripped) and len(stripped) > 12 and set(stripped) <= _SEPARATOR_CHARS


def _meaningful_lines(text: str) -> list[str]:
    lines = [_clean_preview(line).strip() for line in text.splitlines()]
    return [line for line in lines if line and not _is_separator(line)]


def _line_kind(line: str) -> str:
    lowered = line.lower()
    if any(marker in lowered for marker in _NOISE_MARKERS):
        return "noise"
    if " on  " in line and " via " in line:
        return "status"
    if line.startswith("›"):
        return "prompt"
    if any(marker in lowered for marker in _STATUS_MARKERS):
        return "status"
    return "content"


def _content_lines(text: str) -> list[str]:
    return [line for line in _meaningful_lines(text) if _line_kind(line) == "content"]


def _last_of_kind(text: str, kind: str) -> str:
    lines = [line for line in _meaningful_lines(text) if _line_kind(line) == kind]
    return lines[-1] if lines else ""


def _line_score(line: str) -> tuple[int, int]:
    lowered = line.lower()
    score = 1
    if any(word in lowered for word in ("error", "failed", "panic", "traceback")):
        score = 5
    elif any(word in lowered for word in ("warn", "warning")):
        score = 4
    elif any(word in lowered for word in ("test result: ok", "passed", "all doctests ran", "finished ok")):
        score = 4
    elif any(word in lowered for word in ("ran ", "edited ", "updated plan", "working (", "compiling ", "cargo test", "pytest ")):
        score = 3
    if _looks_like_command_echo(line) or _looks_like_codeish(line):
        score -= 1
    return score, len(line)


def _key_lines(text: str, limit: int = 4) -> list[str]:
    lines = _content_lines(text)
    ranked = sorted(enumerate(lines), key=lambda item: (_line_score(item[1]), item[0]), reverse=True)
    picked = sorted(ranked[:limit], key=lambda item: item[0])
    return [_shorten_line(line) for _, line in picked]


def _shorten_line(line: str, width: int = 160) -> str:
    return line if len(line) <= width else f"{line[: width - 1]}…"


def _signals(text: str) -> list[str]:
    lowered = _clean_preview(text).lower()
    return [name for name, needles in _SIGNAL_RULES if any(needle in lowered for needle in needles)]


def _last_meaningful_line(text: str) -> str:
    lines = _meaningful_lines(text)
    return lines[-1] if lines else ""


def _focus_line(text: str) -> str:
    lines = _content_lines(text)
    if not lines:
        return ""
    ranked = sorted(enumerate(lines), key=lambda item: (_line_score(item[1]), item[0]), reverse=True)
    return _shorten_line(ranked[0][1])


def _status_line(text: str) -> str:
    return _shorten_line(_last_of_kind(text, "status"))


def _prompt_line(text: str) -> str:
    return _shorten_line(_last_of_kind(text, "prompt"))


def _dedupe(items: list[str]) -> list[str]:
    seen: set[str] = set()
    kept: list[str] = []
    for item in items:
        if item and item not in seen:
            seen.add(item)
            kept.append(item)
    return kept


def _looks_like_command_echo(line: str) -> bool:
    return bool(re.match(r"^(python|cargo|pytest|tmux|git|printf|node|bash|sh)\b", line.strip()))


def _looks_like_codeish(line: str) -> bool:
    stripped = line.strip()
    return bool(re.match(r"^\d+\s+[+-]?", stripped)) or "!(" in stripped or "::" in stripped


def _looks_idle(preview: str, foreground_name: str, activity_age: int | None) -> bool:
    line = _last_meaningful_line(preview).strip()
    if foreground_name not in _SHELL_NAMES:
        return False
    if line.endswith(("$", "%", ">", "❯")) or (" on  " in line and " via " in line):
        return True
    return activity_age is not None and activity_age > 30


def _activity_age(activity_epoch: int) -> int | None:
    return None if activity_epoch <= 0 else max(0, int(time.time()) - activity_epoch)


def _pane_state(pane: dict[str, Any], foreground_name: str, preview: str) -> str:
    activity_age = _activity_age(pane["activity_epoch"])
    if pane["dead"]:
        return "dead"
    if _looks_idle(preview, foreground_name, activity_age):
        return "idle"
    return "active"


def _messaging_recommendation(state: str, foreground_command: str) -> tuple[bool, str]:
    if state == "idle":
        return True, "foreground is an idle shell; safe to message"
    if state == "dead":
        return False, "pane is dead; sending input will not help"
    return False, f"foreground job is active: {foreground_command}"


def _work_kind(foreground_command: str, signals: list[str]) -> str:
    lowered = foreground_command.lower()
    if "codex" in lowered:
        return "codex"
    if "claude" in lowered:
        return "claude"
    if "eli gateway" in lowered or lowered.startswith("eli "):
        return "gateway"
    if "build" in signals or "test_ok" in signals:
        return "build_or_test"
    return "shell" if _command_name(foreground_command) in _SHELL_NAMES else "process"


def _summary(
    state: str,
    kind: str,
    signals: list[str],
    focus_line: str,
    status_line: str,
) -> str:
    if state == "dead":
        return "Pane is dead."
    summary = f'{"Idle" if state == "idle" else "Active"} {kind} pane.'
    if signals:
        summary += f" Signals: {', '.join(signals)}."
    if focus_line:
        summary += f" Focus: {focus_line}"
    elif status_line:
        summary += f" Status: {status_line}"
    return summary


def _inspect_pane(pane: dict[str, Any], lines: int, include_preview: bool) -> dict[str, Any]:
    target = _target_of(pane)
    preview = _capture_text(target, lines)
    output = preview["stdout"] if preview["success"] else ""
    rows = _ps_rows(pane["tty"])
    foreground = _foreground(rows) or {"command": pane["command"]}
    foreground_name = _command_name(foreground["command"])
    state = _pane_state(pane, foreground_name, output)
    worth_messaging, reason = _messaging_recommendation(state, foreground["command"])
    signals = _signals(output)
    last_line = _last_meaningful_line(output)
    focus_line = _focus_line(output)
    prompt_line = _prompt_line(output)
    status_line = _status_line(output)
    extra_lines = [line for line in _key_lines(output) if line not in {prompt_line, focus_line, status_line}]
    key_lines = _dedupe([prompt_line, focus_line, status_line, *extra_lines])[:4]
    kind = _work_kind(foreground["command"], signals)
    pane_view = {
        **pane,
        "target": target,
        "activity_age_secs": _activity_age(pane["activity_epoch"]),
        "foreground_name": foreground_name,
        "foreground_command": foreground["command"],
        "state": state,
        "work_kind": kind,
        "worth_messaging": worth_messaging,
        "messaging_reason": reason,
        "summary": _summary(state, kind, signals, focus_line, status_line),
        "signals": signals,
        "last_line": last_line,
        "focus_line": focus_line,
        "prompt_line": prompt_line,
        "status_line": status_line,
        "key_lines": key_lines,
    }
    if include_preview:
        pane_view["preview"] = output
    return pane_view


def _compact_pane_view(pane: dict[str, Any]) -> dict[str, Any]:
    keys = (
        "target",
        "pane_id",
        "path",
        "activity_age_secs",
        "foreground_command",
        "state",
        "work_kind",
        "worth_messaging",
        "messaging_reason",
        "summary",
        "signals",
        "focus_line",
        "prompt_line",
        "status_line",
        "key_lines",
    )
    compact = {key: pane[key] for key in keys}
    return compact | {"preview": pane["preview"]} if "preview" in pane else compact


def list_panes(_: dict[str, Any]) -> dict[str, Any]:
    result = _tmux(["list-panes", "-a", "-F", _PANE_FORMAT])
    if not result["success"]:
        return result
    panes = [_pane_record(line) for line in result["stdout"].splitlines() if line]
    return {"success": True, "count": len(panes), "panes": panes}


def capture(args: dict[str, Any]) -> dict[str, Any]:
    target = _target(args)
    lines = _lines(args)
    if not target:
        return {"success": False, "error": "target is required"}
    if lines is None:
        return {"success": False, "error": "lines must be a positive integer"}
    result = _capture_text(target, lines)
    if not result["success"]:
        return result
    return {"success": True, "target": target, "lines": lines, "output": result["stdout"]}


def inspect(args: dict[str, Any]) -> dict[str, Any]:
    target = _target(args)
    lines = _lines(args)
    if not target:
        return {"success": False, "error": "target is required"}
    if lines is None:
        return {"success": False, "error": "lines must be a positive integer"}
    panes = list_panes({})
    if not panes["success"]:
        return panes
    pane = next((pane for pane in panes["panes"] if target in {pane["pane_id"], _target_of(pane)}), None)
    if pane is None:
        return {"success": False, "error": f"pane not found: {target}"}
    return {"success": True, "pane": _compact_pane_view(_inspect_pane(pane, lines, include_preview=True))}


def survey(args: dict[str, Any]) -> dict[str, Any]:
    lines = _lines(args)
    session = _session(args)
    if lines is None:
        return {"success": False, "error": "lines must be a positive integer"}
    panes = list_panes({})
    if not panes["success"]:
        return panes
    items = [pane for pane in panes["panes"] if not session or pane["session"] == session]
    inspected = [_compact_pane_view(_inspect_pane(pane, lines, include_preview=False)) for pane in items]
    ordered = sorted(inspected, key=lambda item: (item["activity_age_secs"] is None, item["activity_age_secs"] or 0))
    return {"success": True, "count": len(ordered), "panes": ordered}


def _watch_snapshot(args: dict[str, Any]) -> dict[str, Any]:
    return inspect(args) if _target(args) else survey(args)


def _watch_items(snapshot: dict[str, Any]) -> list[dict[str, Any]]:
    return [snapshot["pane"]] if "pane" in snapshot else snapshot["panes"]


def _watch_fingerprint(pane: dict[str, Any]) -> tuple[Any, ...]:
    return (
        pane["state"],
        pane["foreground_command"],
        pane.get("focus_line", ""),
        pane.get("prompt_line", ""),
        tuple(pane.get("signals", [])),
    )


def _watch_event(tick: int, pane: dict[str, Any], previous: dict[str, Any]) -> dict[str, Any]:
    return {
        "tick": tick,
        "target": pane["target"],
        "state": pane["state"],
        "worth_messaging": pane["worth_messaging"],
        "summary": pane["summary"],
        "changed": _changed_fields(pane, previous),
        "focus_line": pane.get("focus_line", ""),
        "status_line": pane.get("status_line", ""),
    }


def _changed_fields(current: dict[str, Any], previous: dict[str, Any]) -> list[str]:
    fields = ("state", "foreground_command", "focus_line", "prompt_line", "status_line", "signals")
    return [field for field in fields if current.get(field) != previous.get(field)]


def _watch_summary(events: list[dict[str, Any]], final_items: list[dict[str, Any]]) -> str:
    if not final_items:
        return "No panes matched watch target."
    changed = len(events)
    active = sum(1 for item in final_items if item["state"] == "active")
    latest = next((item.get("focus_line") or item.get("status_line") for item in final_items if item.get("focus_line") or item.get("status_line")), "")
    summary = f"Observed {changed} change(s); {active} pane(s) still active."
    return f"{summary} Latest: {latest}" if latest else summary


def watch(args: dict[str, Any]) -> dict[str, Any]:
    lines = _lines(args)
    ticks = _ticks(args)
    interval = _interval(args)
    if lines is None or ticks is None or interval is None:
        return {"success": False, "error": "lines, ticks, and interval must be positive"}
    initial = _watch_snapshot(args | {"lines": lines})
    if not initial["success"]:
        return initial
    return _watch_loop(args, initial, lines, ticks, interval)


def _watch_loop(
    args: dict[str, Any],
    initial: dict[str, Any],
    lines: int,
    ticks: int,
    interval: float,
) -> dict[str, Any]:
    previous = {pane["target"]: pane for pane in _watch_items(initial)}
    events: list[dict[str, Any]] = []
    current = initial
    for tick in range(1, ticks):
        time.sleep(interval)
        current = _watch_snapshot(args | {"lines": lines})
        if not current["success"]:
            return current
        events.extend(_watch_changes(previous, _watch_items(current), tick, args))
        previous = {pane["target"]: pane for pane in _watch_items(current)}
        if _stop_on_idle(args, current):
            break
    final_items = _watch_items(current)
    return _watch_result(initial, current, events, final_items, ticks, interval)


def _watch_changes(
    previous: dict[str, dict[str, Any]],
    panes: list[dict[str, Any]],
    tick: int,
    args: dict[str, Any],
) -> list[dict[str, Any]]:
    events = [_watch_event(tick, pane, previous[pane["target"]]) for pane in panes if previous.get(pane["target"]) and _watch_fingerprint(pane) != _watch_fingerprint(previous[pane["target"]])]
    return [event for event in events if not args.get("active_only") or event["state"] == "active"]


def _stop_on_idle(args: dict[str, Any], snapshot: dict[str, Any]) -> bool:
    return bool(args.get("stop_on_idle")) and any(pane["state"] == "idle" for pane in _watch_items(snapshot))


def _watch_result(
    initial: dict[str, Any],
    current: dict[str, Any],
    events: list[dict[str, Any]],
    final_items: list[dict[str, Any]],
    ticks: int,
    interval: float,
) -> dict[str, Any]:
    return {
        "success": True,
        "mode": "target" if "pane" in initial else "session",
        "ticks": ticks,
        "interval_secs": interval,
        "initial": initial.get("pane", initial.get("panes", [])),
        "events": events,
        "final": current.get("pane", current.get("panes", [])),
        "summary": _watch_summary(events, final_items),
    }


def send_text(args: dict[str, Any]) -> dict[str, Any]:
    target = _target(args)
    text = _text(args)
    if not target:
        return {"success": False, "error": "target is required"}
    if text == "":
        return {"success": False, "error": "text is required"}
    result = _tmux(["send-keys", "-t", target, "-l", text])
    if not result["success"] or args.get("enter", True) is False:
        return _send_text_result(result, target, text, args)
    enter = _tmux(["send-keys", "-t", target, "Enter"])
    return _send_text_result(enter, target, text, args)


def _send_text_result(
    result: dict[str, Any],
    target: str,
    text: str,
    args: dict[str, Any],
) -> dict[str, Any]:
    if not result["success"]:
        return result
    return {"success": True, "target": target, "text": text, "enter": args.get("enter", True)}


def send_keys(args: dict[str, Any]) -> dict[str, Any]:
    target = _target(args)
    keys = _keys(args)
    repeat = _repeat(args)
    if not target:
        return {"success": False, "error": "target is required"}
    if not keys:
        return {"success": False, "error": "keys are required"}
    if repeat is None:
        return {"success": False, "error": "repeat must be a positive integer"}
    result = _send_keys_repeat(target, keys, repeat)
    if not result["success"]:
        return result
    return {"success": True, "target": target, "keys": keys, "repeat": repeat}


def _send_keys_repeat(target: str, keys: list[str], repeat: int) -> dict[str, Any]:
    for _ in range(repeat):
        result = _tmux(["send-keys", "-t", target, *keys])
        if not result["success"]:
            return result
    return {"success": True}


def run(args: dict[str, Any]) -> dict[str, Any]:
    action = str(args.pop("action", "list_panes")).strip()
    handlers = {
        "list_panes": list_panes,
        "capture": capture,
        "inspect": inspect,
        "survey": survey,
        "watch": watch,
        "send_text": send_text,
        "send_keys": send_keys,
    }
    handler = handlers.get(action)
    if not handler:
        return {"success": False, "error": f"unknown action: {action}"}
    return handler(args)


def main() -> None:
    args = parse_cli_args(sys.argv[1:])
    result = run(args)
    stdout_text, stderr_text, exit_code = render_result(result)
    if stdout_text:
        sys.stdout.write(stdout_text)
        if not stdout_text.endswith("\n"):
            sys.stdout.write("\n")
    if stderr_text:
        sys.stderr.write(stderr_text)
        if not stderr_text.endswith("\n"):
            sys.stderr.write("\n")
    sys.exit(exit_code)


if __name__ == "__main__":
    main()
