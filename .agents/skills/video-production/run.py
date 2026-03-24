#!/usr/bin/env python3
"""video-production skill — Seedance 视频生成。

通过 ARK API (Seedance) 生成短视频。
"""

from __future__ import annotations

from pathlib import Path
import sys

_SCRIPTS_DIR = Path(__file__).resolve().parents[2] / "scripts"
if str(_SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(_SCRIPTS_DIR))

from skill_runner.env import load_repo_dotenv
from skill_runner.cli_contract import parse_cli_args, render_result

load_repo_dotenv(__file__)

import json
import os
import time
import urllib.error
import urllib.request


_ARK_BASE = "https://ark.cn-beijing.volces.com/api/v3"
_DEFAULT_SEEDANCE_MODEL = "doubao-seedance-1-5-pro-251215"
_POLL_INTERVAL = 5
_POLL_TIMEOUT = 600


def _ark_headers(api_key: str) -> dict:
    return {"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"}


def _create_task(api_key: str, model: str, prompt: str) -> dict:
    body = json.dumps({
        "model": model,
        "content": [{"type": "text", "text": prompt}],
    }).encode()
    req = urllib.request.Request(
        f"{_ARK_BASE}/contents/generations/tasks",
        data=body,
        headers=_ark_headers(api_key),
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read().decode())


def _poll_task(api_key: str, task_id: str) -> dict:
    req = urllib.request.Request(
        f"{_ARK_BASE}/contents/generations/tasks/{task_id}",
        headers=_ark_headers(api_key),
        method="GET",
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read().decode())


def generate(args: dict) -> dict:
    prompt = args.get("prompt", "")
    if not prompt:
        return {"success": False, "error": "prompt is required"}

    api_key = os.environ.get("ARK_API_KEY", "")
    model = os.environ.get("SEEDANCE_ENDPOINT_ID", "").strip() or _DEFAULT_SEEDANCE_MODEL
    if not api_key:
        return {"success": False, "error": "ARK_API_KEY not set"}

    try:
        task_data = _create_task(api_key, model, prompt)
    except urllib.error.HTTPError as exc:
        detail = ""
        try:
            detail = exc.read().decode().strip()
        except Exception:
            pass
        return {"success": False, "error": f"create task failed: HTTP {exc.code} {detail}"}
    except urllib.error.URLError as exc:
        return {"success": False, "error": f"create task failed: {exc}"}

    task_id = task_data.get("id", "")
    if not task_id:
        return {"success": False, "error": "no task id returned", "response": task_data}

    deadline = time.time() + _POLL_TIMEOUT
    status = "running"
    result_data: dict = {}
    while time.time() < deadline:
        time.sleep(_POLL_INTERVAL)
        try:
            result_data = _poll_task(api_key, task_id)
        except Exception as exc:
            return {"success": False, "error": f"poll failed: {exc}", "task_id": task_id}
        status = result_data.get("status", "")
        if status not in ("running", "queued", "pending"):
            break
    else:
        return {"success": False, "error": f"task timed out after {_POLL_TIMEOUT}s", "task_id": task_id}

    if status != "succeeded":
        return {
            "success": False,
            "error": f"task {status}: {result_data.get('error', '')}",
            "task_id": task_id,
        }

    content = result_data.get("content", {})
    video_url = ""
    if isinstance(content, dict):
        video_url = content.get("video_url", "")
    elif isinstance(content, list):
        for item in content:
            if isinstance(item, dict) and item.get("type") == "video_url":
                video_url = item.get("video_url", {}).get("url", "")
                break

    if not video_url:
        return {"success": False, "error": "no video url in result", "task_id": task_id}

    output = args.get("output", f"/tmp/seedance_{int(time.time())}.mp4")
    out_path = Path(output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with urllib.request.urlopen(video_url, timeout=300) as resp:
            out_path.write_bytes(resp.read())
    except urllib.error.URLError as exc:
        return {"success": False, "error": f"download failed: {exc}"}

    if not out_path.exists() or out_path.stat().st_size <= 0:
        return {"success": False, "error": f"video file empty or missing: {output}"}

    return {
        "success": True,
        "path": output,
        "prompt": prompt,
        "model": model,
        "task_id": task_id,
        "message": f"视频已保存到 {output}",
    }


def run(args: dict) -> dict:
    action = args.pop("action", "generate")
    if action == "generate":
        return generate(args)
    return {"success": False, "error": f"unknown action: {action}"}


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
