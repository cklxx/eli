#!/usr/bin/env python3
"""browser-use skill — Playwright MCP Extension Relay 控制用户已登录的浏览器。

每次调用启动一个 playwright-mcp 进程，通过 stdin JSON-RPC 发送工具调用。
单动作直接调用，多步操作用 pipeline 在同一个浏览器会话内批量执行。

actions: navigate, snapshot, click, type, screenshot, tabs, evaluate, run_code, press_key, wait_for, pipeline, follow_user
"""

from __future__ import annotations

import contextlib
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

_SCRIPTS_DIR = Path(__file__).resolve().parents[2] / "scripts"
if str(_SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(_SCRIPTS_DIR))

from skill_runner.env import load_repo_dotenv
from skill_runner.cli_contract import parse_cli_args, render_result

load_repo_dotenv(__file__)

_CALL_TIMEOUT = int(os.environ.get("BROWSER_SKILL_TIMEOUT", "30"))
_INTER_CALL_DELAY = float(os.environ.get("BROWSER_SKILL_DELAY", "4"))
_MCP_EXTENSION_ID = "mmlmfjhmonkocbjadbfplnigmagldckm"
_MCP_CLI_PATH = "/Users/bytedance/.npm/_npx/86170c4cd1c5da32/node_modules/@playwright/mcp/cli.js"

_CLOSE_TABS_APPLESCRIPT = f'''
tell application "Google Chrome"
    repeat with w in windows
        set n to count of tabs of w
        repeat with i from n to 1 by -1
            if URL of tab i of w contains "{_MCP_EXTENSION_ID}" then
                close tab i of w
            end if
        end repeat
        if (count of tabs of w) = 0 then close w
    end repeat
end tell
'''


def _close_extension_tabs():
    """Close all Playwright MCP extension tabs in Chrome via AppleScript."""
    if sys.platform != "darwin":
        return
    with contextlib.suppress(Exception):
        subprocess.run(["osascript", "-e", _CLOSE_TABS_APPLESCRIPT], capture_output=True, timeout=5)


def _mcp_init_message():
    """Build the JSON-RPC initialize message."""
    return json.dumps({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05", "capabilities": {},
            "clientInfo": {"name": "browser-use-skill", "version": "1.0"},
        },
    })


def _parse_mcp_response(raw: str, call_id: int) -> dict | None:
    """Parse a JSON-RPC response line, returning a result dict or None if not matching."""
    try:
        msg = json.loads(raw)
    except json.JSONDecodeError:
        return None
    if msg.get("id") != call_id:
        return None
    if "error" in msg:
        return {"success": False, "error": msg["error"].get("message", str(msg["error"]))}
    content = msg.get("result", {}).get("content", [])
    texts = [c.get("text", "") for c in content if c.get("type") == "text"]
    return {"success": True, "output": "\n".join(texts)}


class McpSession:
    """Persistent MCP session that keeps one playwright-mcp process alive."""

    def __init__(self, timeout: int = _CALL_TIMEOUT):
        self.timeout = timeout
        self.proc = None
        self.lines: list[str] = []
        self._reader_thread = None
        self._next_id = 1

    def start(self):
        env = {**os.environ}
        token = env.get("ALEX_BROWSER_BRIDGE_TOKEN", "")
        if token:
            env["PLAYWRIGHT_MCP_EXTENSION_TOKEN"] = token
        self.proc = subprocess.Popen(
            ["node", _MCP_CLI_PATH, "--extension"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env=env, text=True,
        )
        self._reader_thread = threading.Thread(target=self._reader, daemon=True)
        self._reader_thread.start()
        self._send_raw(_mcp_init_message())
        time.sleep(2)

    def _reader(self):
        for line in self.proc.stdout:
            line = line.strip()
            if line:
                self.lines.append(line)

    def _send_raw(self, msg: str):
        self.proc.stdin.write(msg + "\n")
        self.proc.stdin.flush()

    def call(self, tool_name: str, arguments: dict) -> dict:
        """Send a single tool call and wait for its response."""
        call_id = self._next_id
        self._next_id += 1
        self._send_raw(json.dumps({
            "jsonrpc": "2.0", "id": call_id,
            "method": "tools/call",
            "params": {"name": tool_name, "arguments": arguments},
        }))
        deadline = time.monotonic() + self.timeout
        while time.monotonic() < deadline:
            for raw in self.lines:
                result = _parse_mcp_response(raw, call_id)
                if result is not None:
                    return result
            time.sleep(0.3)
        return {"success": False, "error": f"timeout waiting for response to call {call_id}"}

    def close(self):
        if self.proc:
            with contextlib.suppress(Exception):
                self.proc.stdin.close()
            try:
                self.proc.terminate()
                self.proc.wait(timeout=3)
            except Exception:
                self.proc.kill()
        _close_extension_tabs()


def _call_mcp_tools(tool_calls: list[tuple[str, dict]], timeout: int = _CALL_TIMEOUT) -> list[dict]:
    """Spawn one playwright-mcp process, send init + N tool calls, return results."""
    session = McpSession(timeout=timeout)
    session.start()
    results = []
    for tool_name, arguments in tool_calls:
        result = session.call(tool_name, arguments)
        results.append(result)
        time.sleep(_INTER_CALL_DELAY)
    session.close()
    return results


def _call_single(tool_name: str, arguments: dict) -> dict:
    results = _call_mcp_tools([(tool_name, arguments)])
    return results[0] if results else {"success": False, "error": "no response"}


# -- Smart multi-step helpers --

def _find_ref(snapshot_text: str, pattern: str) -> str | None:
    """Extract element ref from snapshot text matching a pattern."""
    m = re.search(rf'{pattern}.*?\[ref=(e\d+)\]', snapshot_text)
    return m.group(1) if m else None


def _wait_for_element(session: McpSession, pattern: str, max_attempts: int = 8) -> str | None:
    """Poll snapshot until element matching pattern appears, return ref or None."""
    for _ in range(max_attempts):
        time.sleep(2)
        snap = session.call("browser_snapshot", {})
        if snap.get("success"):
            ref = _find_ref(snap["output"], pattern)
            if ref:
                return ref
    return None


def _find_enabled_post_button(output: str) -> str | None:
    """Find the first enabled Post button ref from a snapshot."""
    post_refs = re.findall(r'button\s+"Post"\s+\[ref=(e\d+)\]', output)
    disabled = set(re.findall(r'button\s+"Post"\s+\[disabled\]\s+\[ref=(e\d+)\]', output))
    enabled = [r for r in post_refs if r not in disabled]
    if enabled:
        return enabled[0]
    # Fallback: button with nested Post text
    nested = re.findall(r'button\s+\[ref=(e\d+)\](?:\s*\[cursor=pointer\])?\s*:\s*\n\s*-\s*generic.*?Post', output)
    disabled2 = set(re.findall(r'button\s+\[disabled\]\s+\[ref=(e\d+)\]', output))
    return next((r for r in nested if r not in disabled2), None)


def _click_post_button(session: McpSession) -> dict:
    """Snapshot, find enabled Post button, click it, return result."""
    snap = session.call("browser_snapshot", {})
    if not snap.get("success"):
        return snap
    button_ref = _find_enabled_post_button(snap["output"])
    if not button_ref:
        return {"success": False, "error": "enabled Post button not found", "snapshot": snap["output"][:3000]}
    click_result = session.call("browser_click", {"ref": button_ref, "element": "Post button"})
    time.sleep(3)
    final = session.call("browser_snapshot", {})
    return {"success": click_result.get("success", False), "click_result": click_result, "final_snapshot": final.get("output", "")[:1000]}


def smart_post(session: McpSession, url: str, text_to_type: str, textbox_pattern: str, button_pattern: str) -> dict:
    """Navigate, type text, click Post — all in one MCP session."""
    nav = session.call("browser_run_code", {"code": f"async (page) => {{ await page.goto('{url}'); return page.url(); }}"})
    if not nav.get("success"):
        nav = session.call("browser_navigate", {"url": url})
    if not nav.get("success"):
        return nav
    textbox_ref = _wait_for_element(session, textbox_pattern)
    if not textbox_ref:
        return {"success": False, "error": f"textbox matching '{textbox_pattern}' not found after retries"}
    type_result = session.call("browser_type", {"ref": textbox_ref, "text": text_to_type, "submit": False})
    if not type_result.get("success"):
        return type_result
    time.sleep(2)
    return _click_post_button(session)


# -- Actions --

def navigate(a: dict) -> dict:
    url = a.get("url", "")
    if not url:
        return {"success": False, "error": "url is required"}
    return _call_single("browser_navigate", {"url": url})


def snapshot(_args: dict) -> dict:
    return _call_single("browser_snapshot", {})


def click(a: dict) -> dict:
    ref = a.get("ref", "")
    if not ref:
        return {"success": False, "error": "ref is required (from snapshot)"}
    return _call_single("browser_click", {"ref": ref, "element": a.get("element", "")})


def type_text(a: dict) -> dict:
    ref, text = a.get("ref", ""), a.get("text", "")
    if not ref or not text:
        return {"success": False, "error": "ref and text are required"}
    return _call_single("browser_type", {"ref": ref, "text": text, "submit": a.get("submit", False)})


def screenshot(a: dict) -> dict:
    params = {"type": a.get("format", "png")}
    if a.get("filename"):
        params["filename"] = a["filename"]
    if a.get("full_page"):
        params["fullPage"] = True
    return _call_single("browser_take_screenshot", params)


def tabs(a: dict) -> dict:
    params = {"action": a.get("tab_action", "list")}
    if "index" in a:
        params["index"] = a["index"]
    return _call_single("browser_tabs", params)


def evaluate(a: dict) -> dict:
    fn = a.get("function", "")
    if not fn:
        return {"success": False, "error": "function is required"}
    return _call_single("browser_evaluate", {"function": fn})


def run_code(a: dict) -> dict:
    code = a.get("code", "")
    if not code:
        return {"success": False, "error": "code is required"}
    return _call_single("browser_run_code", {"code": code})


def press_key(a: dict) -> dict:
    key = a.get("key", "")
    if not key:
        return {"success": False, "error": "key is required"}
    return _call_single("browser_press_key", {"key": key})


def wait_for(a: dict) -> dict:
    params = {}
    for src, dst in [("time", "time"), ("text", "text"), ("text_gone", "textGone")]:
        if src in a:
            params[dst] = a[src]
    return _call_single("browser_wait_for", params)


def pipeline(a: dict) -> dict:
    """Run multiple actions in one browser session."""
    steps = a.get("steps", [])
    if not steps:
        return {"success": False, "error": "steps is required"}
    tool_calls = [(s["tool"], s.get("args", {})) for s in steps]
    results = _call_mcp_tools(tool_calls)
    return {"success": all(r.get("success") for r in results), "results": results}


def _find_follow_button_ref(snapshot_text: str) -> str | None:
    patterns = [
        r'button\s+"Follow"\s+\[ref=(e\d+)\]',
        r'button\s+"Follow @[^"]+"\s+\[ref=(e\d+)\]',
        r'button\s+"Follow"\s+\[cursor=pointer\]\s+\[ref=(e\d+)\]',
    ]
    for pattern in patterns:
        m = re.search(pattern, snapshot_text)
        if m:
            return m.group(1)
    return None


def follow_user(a: dict) -> dict:
    """Open an X profile, click Follow if visible, and verify the result in one session."""
    handle = (a.get("handle") or "").strip().lstrip("@")
    if not handle:
        return {"success": False, "error": "handle is required"}

    wait_seconds = int(a.get("wait_seconds", 5))
    verify_wait = int(a.get("verify_wait", 4))
    session = McpSession(timeout=max(_CALL_TIMEOUT, 60))
    session.start()
    try:
        url = f"https://x.com/{handle}"
        nav = session.call("browser_navigate", {"url": url})
        if not nav.get("success"):
            return nav

        wait_result = session.call("browser_wait_for", {"time": wait_seconds})
        snap1 = session.call("browser_snapshot", {})
        if not snap1.get("success"):
            return snap1
        snap1_text = snap1.get("output", "")

        if re.search(r'button\s+"Following"', snap1_text) or re.search(r'button\s+"Requested"', snap1_text) or re.search(r'button\s+"Unfollow"', snap1_text):
            return {
                "success": True,
                "status": "already_following",
                "handle": f"@{handle}",
                "url": url,
                "wait_success": wait_result.get("success", False),
            }

        ref = _find_follow_button_ref(snap1_text)
        if not ref:
            return {
                "success": False,
                "status": "follow_button_not_found",
                "handle": f"@{handle}",
                "url": url,
                "wait_success": wait_result.get("success", False),
                "snapshot_excerpt": snap1_text[:4000],
            }

        click = session.call("browser_click", {"ref": ref, "element": f"Follow @{handle}"})
        if not click.get("success"):
            return click

        session.call("browser_wait_for", {"time": verify_wait})
        snap2 = session.call("browser_snapshot", {})
        if not snap2.get("success"):
            return snap2
        snap2_text = snap2.get("output", "")

        if re.search(r'button\s+"Following"', snap2_text) or re.search(r'button\s+"Requested"', snap2_text) or re.search(r'button\s+"Unfollow"', snap2_text):
            status = "requested" if re.search(r'button\s+"Requested"', snap2_text) else "followed"
            return {
                "success": True,
                "status": status,
                "handle": f"@{handle}",
                "url": url,
                "clicked_ref": ref,
                "wait_success": wait_result.get("success", False),
                "snapshot_excerpt": snap2_text[:4000],
            }

        if re.search(r'button\s+"Follow"', snap2_text) and not re.search(r'button\s+"Following"', snap2_text):
            return {
                "success": False,
                "status": "follow_click_no_effect",
                "handle": f"@{handle}",
                "url": url,
                "clicked_ref": ref,
                "wait_success": wait_result.get("success", False),
                "snapshot_excerpt": snap2_text[:4000],
            }

        return {
            "success": False,
            "status": "clicked_but_unverified",
            "handle": f"@{handle}",
            "url": url,
            "clicked_ref": ref,
            "wait_success": wait_result.get("success", False),
            "snapshot_excerpt": snap2_text[:4000],
        }
    finally:
        session.close()

def post(a: dict) -> dict:
    """Smart post: navigate to URL, find textbox, type text, click Post."""
    url = a.get("url", "")
    text = a.get("text", "")
    textbox = a.get("textbox_pattern", 'textbox.*Post text')
    button = a.get("button_pattern", 'button.*Post')
    if not url or not text:
        return {"success": False, "error": "url and text are required"}
    session = McpSession(timeout=int(a.get("timeout", 45)))
    session.start()
    try:
        return smart_post(session, url, text, textbox, button)
    finally:
        session.close()


_ACTIONS = {
    "navigate": navigate, "snapshot": snapshot, "click": click,
    "type": type_text, "screenshot": screenshot, "tabs": tabs,
    "evaluate": evaluate, "run_code": run_code, "press_key": press_key,
    "wait_for": wait_for, "pipeline": pipeline, "follow_user": follow_user, "post": post,
}


def run(args: dict) -> dict:
    action = args.pop("action", "snapshot")
    handler = _ACTIONS.get(action)
    if not handler:
        return {"success": False, "error": f"unknown action: {action} (available: {', '.join(_ACTIONS)})"}
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
