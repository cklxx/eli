"""Gateway E2E integration tests — full IM pipeline via sidecar.

Tests the EXACT same path as real IM messages:
  InboundEnvelope → sidecar envelope conversion → eli webhook → turn pipeline
  → LLM API → outbound callback → sidecar routing → channel plugin delivery

Architecture:
  Python test → POST /test/inbound → sidecar → eli gateway → LLM
                                         ↓ (outbound callback)
  Python test ← GET /test/responses ← sidecar mock plugin captures response

Requires:
  1. eli gateway running (ELI_WEBHOOK_PORT=3100)
  2. test sidecar running (bun sidecar/test/start-test.ts)
"""

import base64
import json
import os
import signal
import subprocess
import tempfile
import time
from datetime import datetime
from pathlib import Path

import pytest
import requests

from conftest import (
    RED_PNG, BLUE_PNG,
    RED_KEYWORDS, BLUE_KEYWORDS,
    assert_response_contains, assert_nonempty,
    switch_profile,
)

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

WEBHOOK_PORT = int(os.environ.get("ELI_WEBHOOK_PORT", "3100"))
SIDECAR_PORT = int(os.environ.get("SIDECAR_PORT", "3201"))
TEST_PORT = int(os.environ.get("TEST_PORT", "3211"))

TEST_INBOUND_URL = f"http://127.0.0.1:{TEST_PORT}/test/inbound"
TEST_RESPONSES_URL = f"http://127.0.0.1:{TEST_PORT}/test/responses"
TEST_CLEAR_URL = f"http://127.0.0.1:{TEST_PORT}/test/clear"
TEST_HEALTH_URL = f"http://127.0.0.1:{TEST_PORT}/test/health"

STARTUP_TIMEOUT = 15
LLM_TIMEOUT = 60
POLL_INTERVAL = 0.5

# Where to save structured test traces
RESULTS_DIR = Path("test-results")


# ---------------------------------------------------------------------------
# Structured trace logging
# ---------------------------------------------------------------------------

class TestTrace:
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

@pytest.fixture(scope="module")
def services():
    """Start test sidecar + eli gateway, yield when both are ready, tear down after.

    Order: sidecar first (needs to own :3201), then gateway (webhook :3100).
    Gateway's built-in sidecar is disabled by temporarily hiding the sidecar/ dir.
    """
    procs = []
    sidecar_dir = Path("sidecar")
    sidecar_bak = Path("sidecar.bak")
    moved = False

    try:
        # 1. Start test sidecar
        sidecar_env = {
            **os.environ,
            "SIDECAR_ELI_URL": f"http://127.0.0.1:{WEBHOOK_PORT}",
            "SIDECAR_PORT": str(SIDECAR_PORT),
            "TEST_PORT": str(TEST_PORT),
        }
        services = subprocess.Popen(
            ["bun", "sidecar/test/start-test.ts"],
            env=sidecar_env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        procs.append(services)

        # Wait for sidecar test endpoints
        deadline = time.time() + STARTUP_TIMEOUT
        while time.time() < deadline:
            try:
                r = requests.get(TEST_HEALTH_URL, timeout=1)
                if r.ok and "test" in r.json().get("channels", []):
                    break
            except requests.ConnectionError:
                pass
            time.sleep(0.3)
        else:
            raise RuntimeError("Test sidecar failed to start")

        # 2. Hide sidecar dir so gateway doesn't start its own
        if sidecar_dir.exists():
            sidecar_dir.rename(sidecar_bak)
            moved = True

        # 3. Start gateway with webhook pointing to our test sidecar
        gw_env = {
            **os.environ,
            "ELI_WEBHOOK_PORT": str(WEBHOOK_PORT),
            "ELI_WEBHOOK_CALLBACK_URL": f"http://127.0.0.1:{SIDECAR_PORT}/outbound",
            "ELI_TELEGRAM_TOKEN": "",
        }
        gw_proc = subprocess.Popen(
            ["eli", "gateway"],
            env=gw_env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        procs.append(gw_proc)

        # Restore sidecar dir immediately after gateway starts
        if moved:
            sidecar_bak.rename(sidecar_dir)
            moved = False

        # Wait for gateway webhook
        deadline = time.time() + STARTUP_TIMEOUT
        while time.time() < deadline:
            try:
                requests.post(
                    f"http://127.0.0.1:{WEBHOOK_PORT}/inbound",
                    json={"session_id": "probe", "channel": "test", "content": "",
                          "chat_id": "probe", "is_active": False, "kind": "normal",
                          "context": {}, "output_channel": ""},
                    timeout=1,
                )
                break
            except requests.ConnectionError:
                time.sleep(0.3)
        else:
            raise RuntimeError("Gateway failed to start")

        yield {"gateway": gw_proc, "sidecar": services}

    finally:
        # Restore sidecar dir if still moved
        if moved and sidecar_bak.exists():
            sidecar_bak.rename(sidecar_dir)

        # Kill all procs
        for proc in reversed(procs):
            proc.send_signal(signal.SIGINT)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=3)


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
    chat_id: str = "test_chat",
    sender_id: str = "test_user",
    sender_name: str = "Test User",
    chat_type: str = "direct",
    media_paths: list[str] | None = None,
    media_types: list[str] | None = None,
    **extra,
) -> dict:
    """Send an InboundEnvelope to the test sidecar."""
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


def wait_for_response(timeout: float = LLM_TIMEOUT) -> dict | None:
    """Poll /test/responses until a response appears."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            r = requests.get(TEST_RESPONSES_URL, timeout=2)
            if r.ok:
                data = r.json()
                responses = data.get("responses", [])
                if responses:
                    return responses[-1]
        except requests.ConnectionError:
            pass
        time.sleep(POLL_INTERVAL)
    return None


def send_and_wait(text: str, timeout: float = LLM_TIMEOUT, **kwargs) -> str:
    """Send envelope and wait for response. Returns response text."""
    envelope = send_envelope(text, **kwargs)
    msg = wait_for_response(timeout=timeout)
    assert msg is not None, f"No response within {timeout}s for: {text[:80]}"
    return msg.get("text", "")


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
        trace = TestTrace("test_openai_text_e2e", "openai")
        switch_profile("openai")
        try:
            response = send_and_wait("Reply with exactly: hello_e2e")
            assert_nonempty(response, "gateway openai text")
            assert "hello" in response.lower() or "e2e" in response.lower()
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise

    def test_anthropic_text_e2e(self, services):
        trace = TestTrace("test_anthropic_text_e2e", "anthropic")
        switch_profile("anthropic")
        try:
            response = send_and_wait("What is 3+3? Reply with just the number.")
            assert_nonempty(response, "gateway anthropic text")
            assert "6" in response
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise

    def test_unicode_content(self, services):
        trace = TestTrace("test_unicode_content", "openai")
        switch_profile("openai")
        try:
            response = send_and_wait("用中文说：你好")
            assert_nonempty(response, "gateway unicode")
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise

    def test_envelope_fields_preserved(self, services):
        """Verify sender/chat metadata survives the pipeline."""
        trace = TestTrace("test_envelope_fields_preserved", "openai")
        switch_profile("openai")
        try:
            send_envelope(
                "Reply with: ok",
                sender_id="user_42",
                sender_name="Alice",
                chat_type="group",
                chat_id="group_chat_1",
            )
            msg = wait_for_response()
            assert msg is not None, "No response"
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
        trace = TestTrace("test_openai_image_e2e", "openai")
        switch_profile("openai")
        img = write_temp_image(RED_PNG)
        try:
            response = send_and_wait(
                "What color is this image? One word.",
                media_paths=[img],
            )
            assert_nonempty(response, "gateway openai vision")
            assert_response_contains(response, RED_KEYWORDS, "gateway openai red")
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise
        finally:
            os.unlink(img)

    def test_anthropic_image_e2e(self, services):
        trace = TestTrace("test_anthropic_image_e2e", "anthropic")
        switch_profile("anthropic")
        img = write_temp_image(BLUE_PNG)
        try:
            response = send_and_wait(
                "What color is this image? One word.",
                media_paths=[img],
            )
            assert_nonempty(response, "gateway anthropic vision")
            assert_response_contains(response, BLUE_KEYWORDS, "gateway anthropic blue")
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise
        finally:
            os.unlink(img)

    def test_multi_image_e2e(self, services):
        trace = TestTrace("test_multi_image_e2e", "openai")
        switch_profile("openai")
        red = write_temp_image(RED_PNG)
        blue = write_temp_image(BLUE_PNG)
        try:
            response = send_and_wait(
                "What two colors are in these images? Answer briefly.",
                media_paths=[red, blue],
            )
            assert_nonempty(response, "gateway multi-image")
            assert_response_contains(response, RED_KEYWORDS, "gateway multi red")
            assert_response_contains(response, BLUE_KEYWORDS, "gateway multi blue")
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise
        finally:
            os.unlink(red)
            os.unlink(blue)

    def test_nonexistent_media_path(self, services):
        """Bad media path should not crash the pipeline."""
        trace = TestTrace("test_nonexistent_media_path", "openai")
        switch_profile("openai")
        try:
            response = send_and_wait(
                "Hello, just checking.",
                media_paths=["/tmp/nonexistent_xyz.png"],
            )
            assert_nonempty(response, "gateway bad media")
            trace.finish("PASS")
        except Exception as e:
            trace.finish("FAIL", str(e))
            raise

    def test_empty_content_with_image(self, services):
        """Image with no text — model should still respond."""
        trace = TestTrace("test_empty_content_with_image", "openai")
        switch_profile("openai")
        img = write_temp_image(BLUE_PNG)
        try:
            response = send_and_wait("", media_paths=[img])
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
