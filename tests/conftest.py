"""Shared fixtures and helpers for eli CLI integration tests."""

import json
import os
import subprocess
import base64
import struct
import zlib
import time
import uuid
from dataclasses import dataclass
from pathlib import Path

import pytest


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

ELI_BIN = os.environ.get("ELI_BIN", "eli")
TIMEOUT = int(os.environ.get("ELI_TEST_TIMEOUT", "60"))


# ---------------------------------------------------------------------------
# CLI runner
# ---------------------------------------------------------------------------

@dataclass
class CliResult:
    stdout: str
    stderr: str
    returncode: int

    @property
    def ok(self) -> bool:
        return self.returncode == 0

    @property
    def output(self) -> str:
        """The assistant response — last non-empty line from stdout."""
        lines = [l for l in self.stdout.strip().splitlines() if l.strip()]
        return lines[-1].strip() if lines else ""

    @property
    def full_output(self) -> str:
        """Full stdout with log lines stripped."""
        lines = []
        for line in self.stdout.splitlines():
            # Skip ANSI-colored tracing log lines
            stripped = line.strip()
            if not stripped:
                continue
            # Log lines start with timestamp or [cli:...]
            if stripped.startswith("\x1b[") or stripped.startswith("[cli:"):
                continue
            if (
                len(stripped) > 20
                and stripped[:4].isdigit()
                and stripped[4] == "-"
                and stripped[7] == "-"
                and stripped[10] == "T"
            ):
                continue
            lines.append(stripped)
        return "\n".join(lines)


def run_eli(*args: str, timeout: int = TIMEOUT, env_override: dict | None = None) -> CliResult:
    """Run an eli CLI command and return the result."""
    cmd = [ELI_BIN, *args]
    env = {**os.environ, **(env_override or {})}
    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
        )
        return CliResult(proc.stdout, proc.stderr, proc.returncode)
    except subprocess.TimeoutExpired:
        pytest.fail(f"eli command timed out after {timeout}s: {' '.join(cmd)}")


# ---------------------------------------------------------------------------
# Image fixture generators
# ---------------------------------------------------------------------------

def _make_solid_png(r: int, g: int, b: int, size: int = 128) -> str:
    """Generate a solid-color 128x128 RGB PNG and return as base64 string."""
    def chunk(chunk_type: bytes, data: bytes) -> bytes:
        c = chunk_type + data
        crc = struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)
        return struct.pack(">I", len(data)) + c + crc

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)  # 8-bit RGB
    raw = b""
    for _ in range(size):
        raw += b"\x00"  # filter byte per row
        for _ in range(size):
            raw += bytes([r, g, b])
    idat = chunk(b"IDAT", zlib.compress(raw))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + idat + chunk(b"IEND", b"")
    return base64.b64encode(png).decode()


RED_PNG = _make_solid_png(255, 0, 0)
BLUE_PNG = _make_solid_png(0, 0, 255)


# ---------------------------------------------------------------------------
# Provider helpers
# ---------------------------------------------------------------------------

def get_profiles() -> dict:
    """Parse eli status output to get available profiles."""
    result = run_eli("status")
    profiles = {}
    in_profiles = False
    for line in result.stdout.splitlines():
        line = line.strip()
        if line.startswith("Profiles:"):
            in_profiles = True
            continue
        if in_profiles:
            if not line or line.startswith("Stored") or line.startswith("Environment"):
                break
            # e.g. "openai * (provider: openai, model: openai:gpt-5.4)"
            is_active = " * " in line or line.endswith(" *")
            name = line.split("(")[0].replace("*", "").strip()
            profiles[name] = {"active": is_active}
    return profiles


def switch_profile(name: str) -> CliResult:
    """Switch eli to a different provider profile."""
    return run_eli("use", name)


def require_profile(name: str) -> CliResult:
    """Switch to a known profile and verify the CLI reports it as active."""
    result = switch_profile(name)
    assert result.ok, f"failed to switch to profile '{name}': {result.stderr}"
    profiles = get_profiles()
    assert profiles.get(name, {}).get("active"), (
        f"profile '{name}' is not active after switch; status output was:\n"
        f"{run_eli('status').stdout}"
    )
    return result


def unique_name(prefix: str) -> str:
    """Return a stable-ish unique identifier for test sessions/chats/files."""
    return f"{prefix}_{int(time.time() * 1000)}_{uuid.uuid4().hex[:8]}"


# ---------------------------------------------------------------------------
# Assertion helpers
# ---------------------------------------------------------------------------

RED_KEYWORDS = ["red", "scarlet", "crimson", "rouge", "rojo"]
BLUE_KEYWORDS = ["blue", "azul", "bleu", "cobalt", "navy"]

# LLM vision is nondeterministic — allow retries for color identification
MAX_VISION_RETRIES = 2


def assert_response_contains(response: str, keywords: list[str], context: str = ""):
    """Assert the response contains at least one keyword (case-insensitive)."""
    lower = response.lower()
    found = any(kw in lower for kw in keywords)
    assert found, f"[{context}] Expected one of {keywords}, got:\n{response}"


def assert_nonempty(response: str, context: str = ""):
    """Assert the response is non-empty."""
    assert len(response.strip()) > 0, f"[{context}] Empty response"
