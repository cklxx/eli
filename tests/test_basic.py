"""Basic CLI integration tests — text chat across providers."""

import pytest
from conftest import run_eli, switch_profile, get_profiles, assert_nonempty


# ---------------------------------------------------------------------------
# Smoke tests — does the CLI work at all?
# ---------------------------------------------------------------------------

class TestSmoke:
    def test_version(self):
        r = run_eli("--version")
        assert r.ok, f"eli --version failed: {r.stderr}"
        assert "eli" in r.stdout.lower() or r.stdout.strip() != ""

    def test_status(self):
        r = run_eli("status")
        assert r.ok, f"eli status failed: {r.stderr}"
        assert "Active profile" in r.stdout

    def test_help(self):
        r = run_eli("--help")
        assert r.ok
        assert "run" in r.stdout
        assert "chat" in r.stdout


# ---------------------------------------------------------------------------
# Text chat — each provider
# ---------------------------------------------------------------------------

class TestTextChat:
    """Basic text chat: send a prompt, verify we get a meaningful response."""

    def test_openai_text(self):
        switch_profile("openai")
        r = run_eli("run", "Reply with exactly one word: pineapple")
        assert r.ok, f"Failed: {r.stderr}"
        assert_nonempty(r.full_output, "openai text")
        assert "pineapple" in r.full_output.lower(), f"Expected 'pineapple', got: {r.full_output}"

    def test_anthropic_text(self):
        switch_profile("anthropic")
        r = run_eli("run", "Reply with exactly one word: mango")
        assert r.ok, f"Failed: {r.stderr}"
        assert_nonempty(r.full_output, "anthropic text")
        assert "mango" in r.full_output.lower(), f"Expected 'mango', got: {r.full_output}"

    def test_system_prompt_followed(self):
        """Model follows the system prompt configured in the framework."""
        switch_profile("openai")
        r = run_eli("run", "What is 2+2? Reply with just the number.")
        assert r.ok, f"Failed: {r.stderr}"
        assert "4" in r.full_output, f"Expected '4' in response: {r.full_output}"

    def test_long_prompt(self):
        """Handles a reasonably long prompt without truncation."""
        switch_profile("openai")
        long_msg = "The quick brown fox jumps over the lazy dog. " * 50
        long_msg += "What animal jumped? Reply in one word."
        r = run_eli("run", long_msg)
        assert r.ok, f"Failed: {r.stderr}"
        assert_nonempty(r.full_output, "long prompt")
        assert "fox" in r.full_output.lower(), f"Expected 'fox': {r.full_output}"

    def test_unicode_prompt(self):
        """Handles CJK and emoji in prompts."""
        switch_profile("openai")
        r = run_eli("run", "用一个词回答：1+1等于几？")
        assert r.ok, f"Failed: {r.stderr}"
        assert_nonempty(r.full_output, "unicode")
        assert "2" in r.full_output or "二" in r.full_output, f"Expected '2' or '二': {r.full_output}"

    def test_empty_prompt_handling(self):
        """Empty or whitespace prompt should not crash."""
        r = run_eli("run", " ")
        # May error gracefully or produce output — either is fine, just no crash
        assert r.returncode in (0, 1), f"Unexpected exit code {r.returncode}: {r.stderr}"


# ---------------------------------------------------------------------------
# Provider switching
# ---------------------------------------------------------------------------

class TestProviderSwitch:
    def test_switch_to_openai(self):
        r = switch_profile("openai")
        assert r.ok, f"switch failed: {r.stderr}"

    def test_switch_to_anthropic(self):
        r = switch_profile("anthropic")
        assert r.ok, f"switch failed: {r.stderr}"

    def test_switch_to_nonexistent(self):
        r = switch_profile("nonexistent_provider_xyz")
        assert not r.ok, "Should fail for nonexistent profile"

    def test_roundtrip_switch(self):
        """Switch away and back, verify the model changes."""
        switch_profile("anthropic")
        r1 = run_eli("status")
        assert "anthropic" in r1.stdout

        switch_profile("openai")
        r2 = run_eli("status")
        assert "openai" in r2.stdout


# ---------------------------------------------------------------------------
# Error handling
# ---------------------------------------------------------------------------

class TestErrorHandling:
    def test_invalid_subcommand(self):
        r = run_eli("nonexistent_command_xyz")
        assert not r.ok

    def test_run_without_message(self):
        r = run_eli("run")
        assert not r.ok, "Should fail without message argument"
