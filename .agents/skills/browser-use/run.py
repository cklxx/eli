#!/usr/bin/env python3
"""browser-use skill — Playwright MCP Extension Relay 控制用户已登录的浏览器。

每次调用启动一个 playwright-mcp 进程，通过 stdin JSON-RPC 发送工具调用。
单动作直接调用，多步操作用 pipeline 在同一个浏览器会话内批量执行。

actions: navigate, snapshot, click, type, screenshot, tabs, evaluate, run_code, press_key, wait_for, pipeline
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


class McpSession:
    """Persistent MCP session that keeps one playwright-mcp process alive."""

    def __init__(self, timeout: int = _CALL_TIMEOUT):
        self.timeout = timeout
        self.proc = None
        self.lines: list[str] = []
        self._reader_thread = None
        self._next_id = 1

    def start(self):
        token = os.environ.get("ALEX_BROWSER_BRIDGE_TOKEN", "")
        env = {**os.environ}
        if token:
            env["PLAYWRIGHT_MCP_EXTENSION_TOKEN"] = token

        self.proc = subprocess.Popen(
            ["npx", "-y", "@playwright/mcp@latest", "--extension"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env=env, text=True,
        )

        self._reader_thread = threading.Thread(target=self._reader, daemon=True)
        self._reader_thread.start()

        init_msg = json.dumps({
            "jsonrpc": "2.0", "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "browser-use-skill", "version": "1.0"},
            },
        })
        self.proc.stdin.write(init_msg + "\n")
        self.proc.stdin.flush()
        time.sleep(2)

    def _reader(self):
        for line in self.proc.stdout:
            line = line.strip()
            if line:
                self.lines.append(line)

    def call(self, tool_name: str, arguments: dict) -> dict:
        """Send a single tool call and wait for its response."""
        call_id = self._next_id
        self._next_id += 1

        call_msg = json.dumps({
            "jsonrpc": "2.0", "id": call_id,
            "method": "tools/call",
            "params": {"name": tool_name, "arguments": arguments},
        })
        self.proc.stdin.write(call_msg + "\n")
        self.proc.stdin.flush()

        deadline = time.monotonic() + self.timeout
        while time.monotonic() < deadline:
            for raw in self.lines:
                try:
                    msg = json.loads(raw)
                except json.JSONDecodeError:
                    continue
                if msg.get("id") == call_id:
                    if "error" in msg:
                        return {"success": False, "error": msg["error"].get("message", str(msg["error"]))}
                    content = msg.get("result", {}).get("content", [])
                    texts = [c.get("text", "") for c in content if c.get("type") == "text"]
                    return {"success": True, "output": "\n".join(texts)}
            time.sleep(0.3)

        return {"success": False, "error": f"timeout waiting for response to call {call_id}"}

    def close(self):
        if self.proc:
            # Close extension tab we opened before killing the process
            with contextlib.suppress(Exception):
                self.call("browser_run_code", {
                    "code": """async (page) => {
                        const pages = page.context().pages();
                        for (const p of pages) {
                            if (p.url().includes('chrome-extension://')) await p.close();
                        }
                    }"""
                })
            with contextlib.suppress(Exception):
                self.proc.stdin.close()
            try:
                self.proc.terminate()
                self.proc.wait(timeout=3)
            except Exception:
                self.proc.kill()


def _call_mcp_tools(tool_calls: list[tuple[str, dict]], timeout: int = _CALL_TIMEOUT) -> list[dict]:
    """Spawn one playwright-mcp process, send init + N tool calls, return results.

    Kept for backward compatibility. Uses McpSession internally.
    """
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


# ── Smart multi-step helper ──

def _find_ref(snapshot_text: str, pattern: str) -> str | None:
    """Extract element ref from snapshot text matching a pattern."""
    m = re.search(rf'{pattern}.*?\[ref=(e\d+)\]', snapshot_text)
    return m.group(1) if m else None


def smart_post(session: McpSession, url: str, text_to_type: str, textbox_pattern: str, button_pattern: str) -> dict:
    """Navigate to a page, wait for it to load, type text, and click a button.

    All in one MCP session — no window stealing.
    """
    # 1. Navigate
    nav = session.call("browser_navigate", {"url": url})
    if not nav.get("success"):
        return nav

    # 2. Wait for page to fully load (poll snapshot until textbox appears)
    textbox_ref = None
    for attempt in range(8):
        time.sleep(2)
        snap = session.call("browser_snapshot", {})
        if snap.get("success"):
            textbox_ref = _find_ref(snap["output"], textbox_pattern)
            if textbox_ref:
                break

    if not textbox_ref:
        return {"success": False, "error": f"textbox matching '{textbox_pattern}' not found after retries"}

    # 3. Type text
    type_result = session.call("browser_type", {"ref": textbox_ref, "text": text_to_type, "submit": False})
    if not type_result.get("success"):
        return type_result

    time.sleep(2)

    # 4. Snapshot to find button
    snap2 = session.call("browser_snapshot", {})
    if not snap2.get("success"):
        return snap2

    # Find the enabled Post button (not the disabled one)
    # Look for button with "Post" that is NOT disabled
    output = snap2["output"]
    # Find all Post button refs
    post_refs = re.findall(r'button\s+"Post"\s+\[ref=(e\d+)\]', output)
    disabled_refs = set(re.findall(r'button\s+"Post"\s+\[disabled\]\s+\[ref=(e\d+)\]', output))
    enabled_post_refs = [r for r in post_refs if r not in disabled_refs]

    if not enabled_post_refs:
        # Maybe button text is inside a child element
        # Look for button [ref=eXXX] followed by "Post" text
        post_refs2 = re.findall(r'button\s+\[ref=(e\d+)\](?:\s*\[cursor=pointer\])?\s*:\s*\n\s*-\s*generic.*?Post', output)
        disabled_refs2 = set(re.findall(r'button\s+\[disabled\]\s+\[ref=(e\d+)\]', output))
        enabled_post_refs = [r for r in post_refs2 if r not in disabled_refs2]

    if not enabled_post_refs:
        return {"success": False, "error": "enabled Post button not found", "snapshot": output[:3000]}

    button_ref = enabled_post_refs[0]

    # 5. Click Post
    click_result = session.call("browser_click", {"ref": button_ref, "element": "Post button"})
    time.sleep(3)

    # 6. Final snapshot to confirm
    final = session.call("browser_snapshot", {})
    return {
        "success": click_result.get("success", False),
        "click_result": click_result,
        "final_snapshot": final.get("output", "")[:1000],
    }


# ── Actions ──

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
    """Run multiple actions in one browser session.

    Example: {"action": "pipeline", "steps": [
        {"tool": "browser_navigate", "args": {"url": "https://x.com"}},
        {"tool": "browser_snapshot", "args": {}}
    ]}
    """
    steps = a.get("steps", [])
    if not steps:
        return {"success": False, "error": "steps is required"}
    tool_calls = [(s["tool"], s.get("args", {})) for s in steps]
    results = _call_mcp_tools(tool_calls)
    return {"success": all(r.get("success") for r in results), "results": results}


def post(a: dict) -> dict:
    """Smart post: navigate to URL, find textbox, type text, click Post — all in one session."""
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
    "wait_for": wait_for, "pipeline": pipeline, "post": post,
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
