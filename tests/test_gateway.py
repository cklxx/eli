"""Gateway E2E integration tests — full IM pipeline via sidecar.

Tests the EXACT same path as real IM messages:
  InboundEnvelope → sidecar envelope conversion → eli webhook → turn pipeline
  → LLM API → outbound callback → sidecar routing → channel plugin delivery

Architecture:
  Python test → POST /test/inbound → sidecar → eli gateway → LLM
                                         ↓ (outbound callback)
  Python test ← GET /test/responses ← sidecar mock plugin captures response

Requires:
  1. eli CLI available on PATH
  2. bun available on PATH
"""

import base64
import json
import os
import signal
import socket
import subprocess
import tempfile
import time
from datetime import datetime
from pathlib import Path

import pytest
import requests

from conftest import (
    BLUE_KEYWORDS,
    BLUE_PNG,
    ELI_BIN,
    MAX_VISION_RETRIES,
    RED_KEYWORDS,
    RED_PNG,
    assert_nonempty,
    assert_response_contains,
    require_profile,
    unique_name,
)

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

WEBHOOK_PORT = 0
SIDECAR_PORT = 0
TEST_PORT = 0

TEST_INBOUND_URL = ""
TEST_RESPONSES_URL = ""
TEST_CLEAR_URL = ""
TEST_HEALTH_URL = ""

STARTUP_TIMEOUT = 15
LLM_TIMEOUT = 60
POLL_INTERVAL = 0.5

# Where to save structured test traces
RESULTS_DIR = Path("test-results")


def configured_port(env_name: str) -> int:
    value = os.environ.get(env_name)
    if value:
        return int(value)
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def configure_ports(webhook_port: int, sidecar_port: int, test_port: int):
    global WEBHOOK_PORT, SIDECAR_PORT, TEST_PORT
    global TEST_INBOUND_URL, TEST_RESPONSES_URL, TEST_CLEAR_URL, TEST_HEALTH_URL
    WEBHOOK_PORT = webhook_port
    SIDECAR_PORT = sidecar_port
    TEST_PORT = test_port
    TEST_INBOUND_URL = f"http://127.0.0.1:{TEST_PORT}/test/inbound"
    TEST_RESPONSES_URL = f"http://127.0.0.1:{TEST_PORT}/test/responses"
    TEST_CLEAR_URL = f"http://127.0.0.1:{TEST_PORT}/test/clear"
    TEST_HEALTH_URL = f"http://127.0.0.1:{TEST_PORT}/test/health"


# ---------------------------------------------------------------------------
# Structured trace logging
# ---------------------------------------------------------------------------

class GatewayTrace:
    """Structured trace for a single test execution."""

    def __init__(self, test_name: str, provider: str):
        self.test_name = test_name
        self.provider = provider
        self.start_time = time.time()
        self.inbound_envelope = None
        self.outbound_response = None
        self.result = "UNKNOWN"
        self.error = None

    def finish(self, result: str, error: str | None = None):
        self.result = result
        self.error = error
        duration_ms = int((time.time() - self.start_time) * 1000)

        trace = {
            "test_name": self.test_name,
            "provider": self.provider,
            "result": result,
            "duration_ms": duration_ms,
            "timestamp": datetime.utcnow().isoformat() + "Z",
        }
        if self.inbound_envelope:
            trace["inbound"] = {
                "channel": self.inbound_envelope.get("channel", ""),
                "text_len": len(self.inbound_envelope.get("text", "")),
                "media_paths": self.inbound_envelope.get("media_paths", []),
            }
        if self.outbound_response:
            trace["outbound"] = {
                "text_len": len(self.outbound_response.get("text", "")),
                "to": self.outbound_response.get("to", ""),
            }
        if error:
            trace["error"] = error

        # Print trace
        status = "✓" if result == "PASS" else "✗"
        print(f"\n  [{status}] {self.test_name} ({self.provider}) — {duration_ms}ms")
        if error:
            print(f"      error: {error[:200]}")

        # Append to results file
        RESULTS_DIR.mkdir(exist_ok=True)
        results_file = RESULTS_DIR / f"gateway-e2e-{datetime.utcnow().strftime('%Y%m%d')}.jsonl"
        with open(results_file, "a") as f:
            f.write(json.dumps(trace) + "\n")


# ---------------------------------------------------------------------------
# Fixtures — service lifecycle
# ---------------------------------------------------------------------------

def start_logged_process(
    cmd: list[str],
    *,
    env: dict[str, str],
    cwd: Path,
    log_name: str,
) -> tuple[subprocess.Popen[str], object, Path]:
    RESULTS_DIR.mkdir(exist_ok=True)
    log_path = RESULTS_DIR / log_name
    log_file = open(log_path, "w")
    proc = subprocess.Popen(
        cmd,
        cwd=cwd,
        env=env,
        stdout=log_file,
        stderr=subprocess.STDOUT,
        text=True,
    )
    return proc, log_file, log_path


def wait_for_sidecar_ready():
    deadline = time.time() + STARTUP_TIMEOUT
    while time.time() < deadline:
        try:
            r = requests.get(TEST_HEALTH_URL, timeout=1)
            if r.ok and "test" in r.json().get("channels", []):
                return
        except requests.ConnectionError:
            pass
        time.sleep(0.3)
    raise RuntimeError("Test sidecar failed to start")


def wait_for_gateway_ready():
    deadline = time.time() + STARTUP_TIMEOUT
    while time.time() < deadline:
        try:
            r = requests.get(TEST_HEALTH_URL, timeout=1)
            if r.ok and r.json().get("gateway") is True:
                return
        except requests.ConnectionError:
            pass
        time.sleep(0.3)
    raise RuntimeError("Gateway failed to start")


def stop_process(proc: subprocess.Popen[str]):
    if proc.poll() is not None:
        return
    proc.send_signal(signal.SIGINT)
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=3)


@pytest.fixture(scope="module")
def services():
    """Start test sidecar + eli gateway, yield when both are ready, tear down after."""
    procs: list[subprocess.Popen[str]] = []
    logs = []
    gateway_dir = tempfile.TemporaryDirectory(prefix="eli-gateway-e2e-")
    gateway_cwd = Path(gateway_dir.name)

    try:
        configure_ports(
            configured_port("ELI_WEBHOOK_PORT"),
            configured_port("SIDECAR_PORT"),
            configured_port("TEST_PORT"),
        )
        sidecar_env = {
            **os.environ,
            "SIDECAR_ELI_URL": f"http://127.0.0.1:{WEBHOOK_PORT}",
            "ELI_WEBHOOK_PORT": str(WEBHOOK_PORT),
            "SIDECAR_PORT": str(SIDECAR_PORT),
            "TEST_PORT": str(TEST_PORT),
        }
        sidecar_proc, sidecar_log, sidecar_log_path = start_logged_process(
            ["bun", "sidecar/test/start-test.ts"],
            env=sidecar_env,
            cwd=Path.cwd(),
            log_name="gateway-sidecar.log",
        )
        procs.append(sidecar_proc)
        logs.append(sidecar_log)
        wait_for_sidecar_ready()

        gateway_env = {
            **os.environ,
            "ELI_WEBHOOK_PORT": str(WEBHOOK_PORT),
            "ELI_WEBHOOK_CALLBACK_URL": f"http://127.0.0.1:{SIDECAR_PORT}/outbound",
            "ELI_TELEGRAM_TOKEN": "",
            "ELI_SIDECAR_DIR": str(gateway_cwd / "missing-sidecar"),
        }
        gw_proc, gw_log, gw_log_path = start_logged_process(
            [ELI_BIN, "gateway"],
            env=gateway_env,
            cwd=gateway_cwd,
            log_name="gateway-eli.log",
        )
        procs.append(gw_proc)
        logs.append(gw_log)
        wait_for_gateway_ready()

        yield {
            "gateway": gw_proc,
            "gateway_log": gw_log_path,
            "sidecar": sidecar_proc,
            "sidecar_log": sidecar_log_path,
        }
    finally:
        for proc in reversed(procs):
            stop_process(proc)
        for log in logs:
            log.close()
        gateway_dir.cleanup()


@pytest.fixture(autouse=True)
def _clear_responses(services):
    """Clear captured responses before each test."""
    try:
        requests.post(TEST_CLEAR_URL, timeout=2)
    except requests.ConnectionError:
        pass
    yield


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def send_envelope(
    text: str,
    channel: str = "test",
    chat_id: str | None = None,
    sender_id: str = "test_user",
    sender_name: str = "Test User",
    chat_type: str = "direct",
    media_paths: list[str] | None = None,
    media_types: list[str] | None = None,
    **extra,
) -> dict:
    """Send an InboundEnvelope to the test sidecar."""
    if chat_id is None:
        chat_id = unique_name("gateway_chat")

    envelope = {
        "channel": channel,
        "accountId": "default",
        "senderId": sender_id,
        "senderName": sender_name,
        "chatType": chat_type,
        "chatId": chat_id,
        "text": text,
        **extra,
    }
    if media_paths:
        envelope["media_paths"] = media_paths
        envelope["media_types"] = media_types or ["image"] * len(media_paths)

    r = requests.post(TEST_INBOUND_URL, json=envelope, timeout=10)
    assert r.ok, f"POST /test/inbound failed: {r.status_code} {r.text}"
    return envelope


def wait_for_response(
    expected_to: str,
    *,
    sent_after_ms: int,
    timeout: float = LLM_TIMEOUT,
) -> dict | None:
    """Poll /test/responses until the expected response appears."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            r = requests.get(TEST_RESPONSES_URL, timeout=2)
            if r.ok:
                data = r.json()
                responses = data.get("responses", [])
                matches = [
                    response
                    for response in responses
                    if response.get("to") == expected_to
                    and response.get("timestamp", 0) >= sent_after_ms
                ]
                if matches:
                    return matches[-1]
        except requests.ConnectionError:
            pass
        time.sleep(POLL_INTERVAL)
    return None


def send_and_wait(text: str, timeout: float = LLM_TIMEOUT, **kwargs) -> tuple[dict, dict]:
    """Send envelope and wait for the correlated response."""
    sent_after_ms = int(time.time() * 1000)
    envelope = send_envelope(text, **kwargs)
    msg = wait_for_response(
        envelope["chatId"],
        sent_after_ms=sent_after_ms,
        timeout=timeout,
    )
    assert msg is not None, f"No response within {timeout}s for: {text[:80]}"
    return envelope, msg


def write_temp_image(b64_data: str) -> str:
    """Write base64 PNG to temp file, return path."""
    fd, path = tempfile.mkstemp(suffix=".png")
    with os.fdopen(fd, "wb") as f:
        f.write(base64.b64decode(b64_data))
    return path


# ---------------------------------------------------------------------------
# Tests — text through full IM pipeline
# ---------------------------------------------------------------------------

class TestGatewayText:
    """Text messages through the complete sidecar → eli → sidecar pipeline."""

    def test_openai_text_e2e(self, services):
        trace = GatewayTrace("test_openai_text_e2e", "openai")
        require_profile("openai")
        try:
            envelope, msg = send_and_wait("Reply with exactly: hello_e2e")
            response = msg.get("text", "")
            trace.inbound_envelope = envelope
            trace.outbound_response = msg
            assert_nonempty(response, "gateway openai text")
            assert "hello" in response.lower() or "e2e" in response.lower()
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise

    def test_anthropic_text_e2e(self, services):
        trace = GatewayTrace("test_anthropic_text_e2e", "anthropic")
        require_profile("anthropic")
        try:
            envelope, msg = send_and_wait("What is 3+3? Reply with just the number.")
            response = msg.get("text", "")
            trace.inbound_envelope = envelope
            trace.outbound_response = msg
            assert_nonempty(response, "gateway anthropic text")
            assert "6" in response
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise

    def test_unicode_content(self, services):
        trace = GatewayTrace("test_unicode_content", "openai")
        require_profile("openai")
        try:
            envelope, msg = send_and_wait("用中文说：你好")
            response = msg.get("text", "")
            trace.inbound_envelope = envelope
            trace.outbound_response = msg
            assert_nonempty(response, "gateway unicode")
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise

    def test_envelope_fields_preserved(self, services):
        """Verify sender/chat metadata survives the pipeline."""
        trace = GatewayTrace("test_envelope_fields_preserved", "openai")
        require_profile("openai")
        try:
            sent_after_ms = int(time.time() * 1000)
            envelope = send_envelope(
                "Reply with: ok",
                sender_id="user_42",
                sender_name="Alice",
                chat_type="group",
                chat_id="group_chat_1",
            )
            trace.inbound_envelope = envelope
            msg = wait_for_response("group_chat_1", sent_after_ms=sent_after_ms)
            assert msg is not None, "No response"
            trace.outbound_response = msg
            # The response should be routed back to the mock plugin
            assert msg.get("to") is not None, f"Missing 'to' in response: {msg}"
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise


# ---------------------------------------------------------------------------
# Tests — vision through full IM pipeline
# ---------------------------------------------------------------------------

class TestGatewayVision:
    """Image tests through the complete IM pipeline.

    Images go through: InboundEnvelope.media_paths → sidecar context →
    eli gateway media resolution (file read + base64) → LLM API.
    """

    def test_openai_image_e2e(self, services):
        trace = GatewayTrace("test_openai_image_e2e", "openai")
        require_profile("openai")
        img = write_temp_image(RED_PNG)
        try:
            last_response = ""
            for attempt in range(MAX_VISION_RETRIES + 1):
                envelope, msg = send_and_wait(
                    "What color is this solid-color image? Reply with one color word only.",
                    media_paths=[img],
                )
                last_response = msg.get("text", "")
                trace.inbound_envelope = envelope
                trace.outbound_response = msg
                if any(kw in last_response.lower() for kw in RED_KEYWORDS):
                    break
            assert_nonempty(last_response, "gateway openai vision")
            assert_response_contains(last_response, RED_KEYWORDS, "gateway openai red")
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise
        finally:
            os.unlink(img)

    def test_anthropic_image_e2e(self, services):
        trace = GatewayTrace("test_anthropic_image_e2e", "anthropic")
        require_profile("anthropic")
        img = write_temp_image(BLUE_PNG)
        try:
            envelope, msg = send_and_wait(
                "What color is this image? One word.",
                media_paths=[img],
            )
            response = msg.get("text", "")
            trace.inbound_envelope = envelope
            trace.outbound_response = msg
            assert_nonempty(response, "gateway anthropic vision")
            assert_response_contains(response, BLUE_KEYWORDS, "gateway anthropic blue")
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise
        finally:
            os.unlink(img)

    def test_multi_image_e2e(self, services):
        trace = GatewayTrace("test_multi_image_e2e", "openai")
        require_profile("openai")
        red = write_temp_image(RED_PNG)
        blue = write_temp_image(BLUE_PNG)
        try:
            last_response = ""
            for attempt in range(MAX_VISION_RETRIES + 1):
                envelope, msg = send_and_wait(
                    "These are two solid-color images. Name the two colors. Reply briefly.",
                    media_paths=[red, blue],
                )
                last_response = msg.get("text", "")
                trace.inbound_envelope = envelope
                trace.outbound_response = msg
                if (any(kw in last_response.lower() for kw in RED_KEYWORDS)
                        and any(kw in last_response.lower() for kw in BLUE_KEYWORDS)):
                    break
            assert_nonempty(last_response, "gateway multi-image")
            assert_response_contains(last_response, RED_KEYWORDS, "gateway multi red")
            assert_response_contains(last_response, BLUE_KEYWORDS, "gateway multi blue")
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise
        finally:
            os.unlink(red)
            os.unlink(blue)

    def test_nonexistent_media_path(self, services):
        """Bad media path should not crash the pipeline."""
        trace = GatewayTrace("test_nonexistent_media_path", "openai")
        require_profile("openai")
        try:
            envelope, msg = send_and_wait(
                "Hello, just checking.",
                media_paths=["/tmp/nonexistent_xyz.png"],
            )
            response = msg.get("text", "")
            trace.inbound_envelope = envelope
            trace.outbound_response = msg
            assert_nonempty(response, "gateway bad media")
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise

    def test_empty_content_with_image(self, services):
        """Image with no text — model should still respond."""
        trace = GatewayTrace("test_empty_content_with_image", "openai")
        require_profile("openai")
        img = write_temp_image(BLUE_PNG)
        try:
            envelope, msg = send_and_wait("", media_paths=[img])
            response = msg.get("text", "")
            trace.inbound_envelope = envelope
            trace.outbound_response = msg
            # May or may not produce output — at least shouldn't crash
            if response.strip():
                lower = response.lower()
                mentions_red = any(kw in lower for kw in RED_KEYWORDS)
                assert not mentions_red, f"Hallucinating red for blue: {response[:200]}"
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise
        finally:
            os.unlink(img)
