"""Tests for tmux skill."""

from __future__ import annotations

import importlib.util
import io
from pathlib import Path
import subprocess
import sys
from unittest.mock import patch

import pytest

_SCRIPTS_DIR = Path(__file__).resolve().parent.parent.parent.parent / "scripts"
sys.path.insert(0, str(_SCRIPTS_DIR))

_RUN_PATH = Path(__file__).resolve().parent.parent / "run.py"
_SPEC = importlib.util.spec_from_file_location("tmux_skill_run", _RUN_PATH)
_MOD = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MOD)


def _completed(*, stdout: str = "", stderr: str = "", code: int = 0):
    return subprocess.CompletedProcess(["tmux"], code, stdout, stderr)


class TestListPanes:
    def test_list_panes_parses_tmux_rows(self):
        stdout = "work\t1\t2\t%7\t/dev/ttys001\tzsh\t\tpane\t/tmp\t10\t1\t100\t1\t0\n"
        with patch.object(_MOD.subprocess, "run", return_value=_completed(stdout=stdout)):
            result = _MOD.list_panes({})
        assert result["success"] is True
        assert result["count"] == 1
        assert result["panes"][0]["pane_id"] == "%7"
        assert result["panes"][0]["active"] is True


class TestCapture:
    def test_capture_requires_target(self):
        result = _MOD.capture({})
        assert result["success"] is False
        assert result["error"] == "target is required"

    def test_capture_returns_output(self):
        with patch.object(_MOD.subprocess, "run", return_value=_completed(stdout="line-1\nline-2\n")):
            result = _MOD.capture({"target": "5:1.1", "lines": 20})
        assert result["success"] is True
        assert result["target"] == "5:1.1"
        assert result["output"] == "line-1\nline-2"


class TestInspectAndSurvey:
    def test_inspect_reports_foreground_process_and_state(self):
        pane_stdout = "work\t1\t2\t%7\t/dev/ttys001\tzsh\t\tpane\t/tmp\t10\t1\t100\t1\t0\n"
        with (
            patch.object(_MOD, "_capture_text", return_value={"success": True, "stdout": "sh-3.2$ "}),
            patch.object(_MOD, "_ps_rows", return_value=[{"pid": 1, "pgid": 2, "tpgid": 2, "stat": "S+", "command": "zsh"}]),
            patch.object(_MOD.subprocess, "run", return_value=_completed(stdout=pane_stdout)),
            patch.object(_MOD.time, "time", return_value=110),
        ):
            result = _MOD.inspect({"target": "%7", "lines": 5})
        assert result["success"] is True
        assert result["pane"]["foreground_command"] == "zsh"
        assert result["pane"]["activity_age_secs"] == 10
        assert result["pane"]["state"] == "idle"
        assert result["pane"]["worth_messaging"] is True
        assert "session" not in result["pane"]

    def test_inspect_extracts_focus_prompt_and_status_lines(self):
        pane_stdout = "work\t1\t2\t%7\t/dev/ttys001\tnode\t\tpane\t/tmp\t10\t1\t100\t1\t0\n"
        preview = "\n".join(
            [
                "• Searching the web",
                "› Run /review on my current changes",
                "关键判断：把 request-state 延后到 admitted 阶段。",
                "• Working (3m 08s • esc to interrupt)",
            ]
        )
        with (
            patch.object(_MOD, "_capture_text", return_value={"success": True, "stdout": preview}),
            patch.object(_MOD, "_ps_rows", return_value=[{"pid": 1, "pgid": 2, "tpgid": 2, "stat": "S+", "command": "node codex"}]),
            patch.object(_MOD.subprocess, "run", return_value=_completed(stdout=pane_stdout)),
            patch.object(_MOD.time, "time", return_value=110),
        ):
            result = _MOD.inspect({"target": "%7", "lines": 5})
        pane = result["pane"]
        assert pane["prompt_line"] == "› Run /review on my current changes"
        assert pane["focus_line"] == "关键判断：把 request-state 延后到 admitted 阶段。"
        assert pane["status_line"] == "• Working (3m 08s • esc to interrupt)"
        assert "Searching the web" not in "\n".join(pane["key_lines"])

    def test_inspect_marks_starship_shell_prompt_idle(self):
        pane_stdout = "work\t1\t2\t%7\t/dev/ttys001\tzsh\t\tpane\t/tmp\t10\t1\t100\t1\t0\n"
        preview = "eli on  main [$?] via 🐳 orbstack via 🦀 v1.93.0"
        with (
            patch.object(_MOD, "_capture_text", return_value={"success": True, "stdout": preview}),
            patch.object(_MOD, "_ps_rows", return_value=[{"pid": 1, "pgid": 2, "tpgid": 2, "stat": "S+", "command": "zsh"}]),
            patch.object(_MOD.subprocess, "run", return_value=_completed(stdout=pane_stdout)),
            patch.object(_MOD.time, "time", return_value=110),
        ):
            result = _MOD.inspect({"target": "%7", "lines": 5})
        assert result["pane"]["state"] == "idle"

    def test_inspect_filters_ui_noise_and_path_refs_from_content(self):
        pane_stdout = "work\t1\t2\t%7\t/dev/ttys001\tnode\t\tpane\t/tmp\t10\t1\t100\t1\t0\n"
        preview = "\n".join(
            [
                "docs/experience/errors/2026-04-21-metal.md:1",
                "■ Conversation interrupted - tell the model what to do differently.",
                "真正的结论：waiting queue 的 consume_one 时机不对。",
                "2 background terminals running · /ps to view · /stop to close",
            ]
        )
        with (
            patch.object(_MOD, "_capture_text", return_value={"success": True, "stdout": preview}),
            patch.object(_MOD, "_ps_rows", return_value=[{"pid": 1, "pgid": 2, "tpgid": 2, "stat": "S+", "command": "node codex"}]),
            patch.object(_MOD.subprocess, "run", return_value=_completed(stdout=pane_stdout)),
            patch.object(_MOD.time, "time", return_value=110),
        ):
            result = _MOD.inspect({"target": "%7", "lines": 5})
        pane = result["pane"]
        assert pane["focus_line"] == "真正的结论：waiting queue 的 consume_one 时机不对。"
        assert pane["content_lines"] == ["真正的结论：waiting queue 的 consume_one 时机不对。"]

    def test_inspect_filters_low_signal_command_traces_from_content(self):
        pane_stdout = "work\t1\t2\t%7\t/dev/ttys001\tnode\t\tpane\t/tmp\t10\t1\t100\t1\t0\n"
        preview = "\n".join(
            [
                "• Ran git show HEAD",
                "└ Search max_batch_tokens in scheduler.rs",
                "List scheduler",
                "└ (no output)",
                "真正的动作：把 shared token budget 改成 decode/prefill 共用。",
            ]
        )
        with (
            patch.object(_MOD, "_capture_text", return_value={"success": True, "stdout": preview}),
            patch.object(_MOD, "_ps_rows", return_value=[{"pid": 1, "pgid": 2, "tpgid": 2, "stat": "S+", "command": "node codex"}]),
            patch.object(_MOD.subprocess, "run", return_value=_completed(stdout=pane_stdout)),
            patch.object(_MOD.time, "time", return_value=110),
        ):
            result = _MOD.inspect({"target": "%7", "lines": 5})
        pane = result["pane"]
        assert pane["focus_line"] == "真正的动作：把 shared token budget 改成 decode/prefill 共用。"
        assert pane["content_lines"] == ["真正的动作：把 shared token budget 改成 decode/prefill 共用。"]

    def test_inspect_prefers_prompt_and_latest_content_in_summary(self):
        pane_stdout = "work\t1\t2\t%7\t/dev/ttys001\tnode\t\tpane\t/tmp\t10\t1\t100\t1\t0\n"
        preview = "\n".join(
            [
                "› Run /review on my current changes",
                "max_waiting_requests|SchedulerHandle|Continuous batching|prefix cache in scheduler",
                "└ ## main...origin/main [ahead 1]",
                "M crates/cuda-kernels/src/paged_kv.rs",
                "关键结论：scheduler smoke 已经跑通，剩下并发压测。",
                "关键结论：scheduler smoke 已经跑通，剩下并发压测。",
                "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
            ]
        )
        with (
            patch.object(_MOD, "_capture_text", return_value={"success": True, "stdout": preview}),
            patch.object(_MOD, "_ps_rows", return_value=[{"pid": 1, "pgid": 2, "tpgid": 2, "stat": "S+", "command": "node codex"}]),
            patch.object(_MOD.subprocess, "run", return_value=_completed(stdout=pane_stdout)),
            patch.object(_MOD.time, "time", return_value=110),
        ):
            result = _MOD.inspect({"target": "%7", "lines": 5})
        pane = result["pane"]
        assert pane["content_lines"] == ["关键结论：scheduler smoke 已经跑通，剩下并发压测。"]
        assert "Task: Run /review on my current changes." in pane["summary"]
        assert "Latest: 关键结论：scheduler smoke 已经跑通，剩下并发压测。" in pane["summary"]

    def test_survey_filters_session_and_sorts_by_recent_activity(self):
        pane_stdout = (
            "a\t1\t1\t%1\t/dev/ttys001\tzsh\t\tone\t/tmp\t1\t1\t100\t1\t0\n"
            "b\t1\t1\t%2\t/dev/ttys002\tnode\t\ttwo\t/tmp\t1\t1\t140\t1\t0\n"
            "a\t1\t2\t%3\t/dev/ttys003\tzsh\t\tthree\t/tmp\t1\t1\t130\t1\t0\n"
        )

        def _capture(target, _lines):
            text = "doing work" if target == "a:1.2" else "sh-3.2$ "
            return {"success": True, "stdout": text}

        def _ps_rows(tty):
            return [{"pid": 1, "pgid": 2, "tpgid": 2, "stat": "S+", "command": "node" if tty.endswith("002") else "zsh"}]

        with (
            patch.object(_MOD, "_capture_text", side_effect=_capture),
            patch.object(_MOD, "_ps_rows", side_effect=_ps_rows),
            patch.object(_MOD.subprocess, "run", return_value=_completed(stdout=pane_stdout)),
            patch.object(_MOD.time, "time", return_value=150),
        ):
            result = _MOD.survey({"session": "a", "lines": 3})

        assert result["success"] is True
        assert result["count"] == 2
        assert result["running"][0]["pane_id"] == "%3"
        assert result["running"][0]["content_lines"] == ["doing work"]
        assert result["idle"][0]["pane_id"] == "%1"
        assert "panes" not in result


class TestWatch:
    def test_watch_reports_changes_for_target(self):
        snapshots = [
            {"success": True, "pane": {"target": "7:1.1", "pane_id": "%7", "path": "/tmp", "activity_age_secs": 0, "state": "active", "work_kind": "codex", "foreground_command": "node codex", "focus_line": "step one", "prompt_line": "", "status_line": "", "signals": [], "summary": "active", "worth_messaging": False, "messaging_reason": "busy", "key_lines": ["step one"], "preview": "step one", "last_line": "step one"}},
            {"success": True, "pane": {"target": "7:1.1", "pane_id": "%7", "path": "/tmp", "activity_age_secs": 1, "state": "idle", "work_kind": "shell", "foreground_command": "zsh", "focus_line": "watch-b", "prompt_line": "", "status_line": "", "signals": ["waiting"], "summary": "idle", "worth_messaging": True, "messaging_reason": "safe", "key_lines": ["watch-b"], "preview": "step one\nwatch-b", "last_line": "watch-b"}},
        ]
        with (
            patch.object(_MOD, "_watch_snapshot", side_effect=snapshots),
            patch.object(_MOD.time, "sleep", return_value=None),
        ):
            result = _MOD.watch({"target": "%7", "ticks": 2, "interval": 1, "lines": 5, "stop_on_idle": True})
        assert result["success"] is True
        assert result["mode"] == "target"
        assert len(result["events"]) == 1
        assert result["events"][0]["changed"] == ["state", "foreground_command", "focus_line", "signals", "new_lines"]
        assert result["events"][0]["new_lines"] == ["watch-b"]
        assert result["final"]["state"] == "idle"
        assert result["stop_reason"] == "idle"

    def test_watch_active_only_filters_idle_changes(self):
        snapshots = [
            {"success": True, "panes": [{"target": "4:1.1", "pane_id": "%4", "path": "/tmp", "activity_age_secs": 0, "state": "active", "work_kind": "codex", "foreground_command": "node codex", "focus_line": "step one", "prompt_line": "", "status_line": "", "signals": [], "summary": "active", "worth_messaging": False, "messaging_reason": "busy", "key_lines": ["step one"], "last_line": "step one"}]},
            {"success": True, "panes": [{"target": "4:1.1", "pane_id": "%4", "path": "/tmp", "activity_age_secs": 1, "state": "idle", "work_kind": "shell", "foreground_command": "zsh", "focus_line": "done", "prompt_line": "", "status_line": "", "signals": [], "summary": "idle", "worth_messaging": True, "messaging_reason": "safe", "key_lines": ["done"], "last_line": "done"}]},
        ]
        with (
            patch.object(_MOD, "_watch_snapshot", side_effect=snapshots),
            patch.object(_MOD.time, "sleep", return_value=None),
        ):
            result = _MOD.watch({"session": "4", "ticks": 2, "interval": 1, "lines": 5, "active_only": True})
        assert result["success"] is True
        assert result["events"] == []

    def test_watch_stops_after_silence_window(self):
        snapshot = {
            "success": True,
            "pane": {
                "target": "7:1.1",
                "pane_id": "%7",
                "path": "/tmp",
                "activity_age_secs": 0,
                "state": "active",
                "work_kind": "process",
                "foreground_command": "python job",
                "focus_line": "watch-a",
                "prompt_line": "",
                "status_line": "",
                "signals": [],
                "summary": "active",
                "worth_messaging": False,
                "messaging_reason": "busy",
                "key_lines": ["watch-a"],
                "preview": "watch-a",
                "last_line": "watch-a",
            },
        }
        with (
            patch.object(_MOD, "_watch_snapshot", side_effect=[snapshot, snapshot, snapshot]),
            patch.object(_MOD.time, "sleep", return_value=None),
        ):
            result = _MOD.watch({"target": "%7", "ticks": 5, "interval": 1, "lines": 5, "silence_secs": 2})
        assert result["success"] is True
        assert result["stop_reason"] == "silence"

    def test_watch_new_lines_handles_sliding_window(self):
        snapshots = [
            {"success": True, "pane": {"target": "7:1.1", "pane_id": "%7", "path": "/tmp", "activity_age_secs": 0, "state": "active", "work_kind": "process", "foreground_command": "python", "focus_line": "c", "prompt_line": "", "status_line": "", "signals": [], "summary": "active", "worth_messaging": False, "messaging_reason": "busy", "key_lines": ["c"], "preview": "a\nb\nc", "last_line": "c"}},
            {"success": True, "pane": {"target": "7:1.1", "pane_id": "%7", "path": "/tmp", "activity_age_secs": 1, "state": "active", "work_kind": "process", "foreground_command": "python", "focus_line": "d", "prompt_line": "", "status_line": "", "signals": [], "summary": "active", "worth_messaging": False, "messaging_reason": "busy", "key_lines": ["d"], "preview": "b\nc\nd", "last_line": "d"}},
        ]
        with (
            patch.object(_MOD, "_watch_snapshot", side_effect=snapshots),
            patch.object(_MOD.time, "sleep", return_value=None),
        ):
            result = _MOD.watch({"target": "%7", "ticks": 2, "interval": 1, "lines": 3})
        assert result["events"][0]["new_lines"] == ["d"]
        assert "preview" not in result["final"]

    def test_watch_target_missing_returns_dead_final(self):
        first = {
            "success": True,
            "pane": {
                "target": "7:1.1",
                "pane_id": "%7",
                "path": "/tmp",
                "activity_age_secs": 0,
                "state": "active",
                "work_kind": "process",
                "foreground_command": "python",
                "focus_line": "running",
                "prompt_line": "",
                "status_line": "",
                "signals": [],
                "summary": "active",
                "worth_messaging": False,
                "messaging_reason": "busy",
                "key_lines": ["running"],
                "preview": "running",
                "last_line": "running",
            },
        }
        with (
            patch.object(_MOD, "_watch_snapshot", side_effect=[first, {"success": False, "error": "pane not found: %7"}]),
            patch.object(_MOD.time, "sleep", return_value=None),
        ):
            result = _MOD.watch({"target": "%7", "ticks": 2, "interval": 1, "lines": 5})
        assert result["success"] is True
        assert result["stop_reason"] == "missing"
        assert result["final"]["state"] == "dead"


class TestSendText:
    def test_send_text_sends_literal_and_enter(self):
        calls: list[list[str]] = []

        def _fake_run(command, **_kwargs):
            calls.append(command)
            return _completed()

        with patch.object(_MOD.subprocess, "run", side_effect=_fake_run):
            result = _MOD.send_text({"target": "%5", "text": "cargo test"})

        assert result["success"] is True
        assert calls[0] == ["tmux", "send-keys", "-t", "%5", "-l", "cargo test"]
        assert calls[1] == ["tmux", "send-keys", "-t", "%5", "Enter"]

    def test_send_text_honors_no_enter(self):
        calls: list[list[str]] = []

        def _fake_run(command, **_kwargs):
            calls.append(command)
            return _completed()

        with patch.object(_MOD.subprocess, "run", side_effect=_fake_run):
            result = _MOD.send_text({"target": "%5", "text": "pwd", "enter": False})

        assert result["success"] is True
        assert calls == [["tmux", "send-keys", "-t", "%5", "-l", "pwd"]]


class TestSendKeys:
    def test_send_keys_uses_explicit_keys(self):
        calls: list[list[str]] = []

        def _fake_run(command, **_kwargs):
            calls.append(command)
            return _completed()

        with patch.object(_MOD.subprocess, "run", side_effect=_fake_run):
            result = _MOD.send_keys({"target": "%5", "keys": ["C-c", "Enter"]})

        assert result["success"] is True
        assert result["keys"] == ["C-c", "Enter"]
        assert calls == [["tmux", "send-keys", "-t", "%5", "C-c", "Enter"]]

    def test_send_keys_accepts_positionals_and_repeat(self):
        calls: list[list[str]] = []

        def _fake_run(command, **_kwargs):
            calls.append(command)
            return _completed()

        with patch.object(_MOD.subprocess, "run", side_effect=_fake_run):
            result = _MOD.send_keys({"positionals": ["%5", "C-c,Enter"], "repeat": 2})

        assert result["success"] is True
        assert result["keys"] == ["C-c", "Enter"]
        assert result["repeat"] == 2
        assert calls == [
            ["tmux", "send-keys", "-t", "%5", "C-c", "Enter"],
            ["tmux", "send-keys", "-t", "%5", "C-c", "Enter"],
        ]


class TestMainRouting:
    def test_unknown_action_returns_structured_error(self):
        with (
            patch("sys.argv", ["run.py", "nope"]),
            patch("sys.stdout", new=io.StringIO()),
            patch("sys.stderr", new=io.StringIO()) as stderr,
        ):
            with pytest.raises(SystemExit) as exc:
                _MOD.main()
        assert exc.value.code == 1
        assert "unknown action: nope" in stderr.getvalue()

    def test_json_stdin_dispatches_action(self):
        stdout = "work\t1\t2\t%7\t/dev/ttys001\tzsh\t\tpane\t/tmp\t10\t1\t100\t1\t0\n"
        with (
            patch.object(_MOD.subprocess, "run", return_value=_completed(stdout=stdout)),
            patch("sys.argv", ["run.py"]),
            patch("sys.stdin", new=io.StringIO('{"action":"list_panes"}')),
            patch("sys.stdout", new=io.StringIO()) as stdout_io,
        ):
            with pytest.raises(SystemExit) as exc:
                _MOD.main()
        assert exc.value.code == 0
        assert "count: 1" in stdout_io.getvalue()
