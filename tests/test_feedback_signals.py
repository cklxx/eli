"""End-to-end tests for tool feedback signals — verify LLMs receive and act on them.

Each test exercises a changed feedback signal and checks that the LLM's response
reflects correct understanding of what happened.
"""

import os
import tempfile
import uuid

import pytest
from conftest import run_eli, switch_profile, assert_nonempty


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

PROVIDER = "openai"


def eli_run(prompt: str, timeout: int = 90, chat_id: str | None = None):
    """Run eli with an isolated chat-id to prevent session contamination."""
    cid = chat_id or f"test-{uuid.uuid4().hex[:12]}"
    return run_eli("run", prompt, "--chat-id", cid, timeout=timeout)


def setup_provider():
    switch_profile(PROVIDER)


# ---------------------------------------------------------------------------
# P0: "(command succeeded, no output)" vs "(command failed with no output)"
# ---------------------------------------------------------------------------

class TestBashNoOutputSignals:
    """LLM must distinguish success-no-output from failure-no-output."""

    def test_bash_success_no_output(self):
        """Commands like `true` or `mkdir` produce no output — LLM should know it succeeded."""
        setup_provider()
        r = eli_run(
            "Run the bash command `true` and tell me: did it succeed or fail? "
            "Reply with exactly one word: SUCCESS or FAILURE."
        )
        assert r.ok, f"Failed: {r.stderr}"
        assert "success" in r.full_output.lower(), (
            f"LLM should recognize success-no-output. Got: {r.full_output}"
        )

    def test_bash_failure_no_output(self):
        """Commands like `false` fail with no output — LLM should know it failed."""
        setup_provider()
        r = eli_run(
            "Run the bash command `false` and tell me: did it succeed or fail? "
            "Reply with exactly one word: SUCCESS or FAILURE."
        )
        assert r.ok, f"Failed: {r.stderr}"
        assert "fail" in r.full_output.lower(), (
            f"LLM should recognize failure-no-output. Got: {r.full_output}"
        )

    def test_bash_nonexistent_command(self):
        """A command that doesn't exist fails — LLM should report failure."""
        setup_provider()
        r = eli_run(
            "Run the bash command `__nonexistent_cmd_xyz_42__` and tell me what happened. "
            "Did it succeed or fail? Reply with one word: SUCCESS or FAILURE."
        )
        assert r.ok, f"Failed: {r.stderr}"
        assert "fail" in r.full_output.lower(), (
            f"LLM should recognize command-not-found as failure. Got: {r.full_output}"
        )


# ---------------------------------------------------------------------------
# P0: bash.output status "running" vs "success"/"failed"
# ---------------------------------------------------------------------------

class TestBashBackgroundSignals:
    """LLM should understand background shell lifecycle."""

    def test_background_bash_returns_poll_instructions(self):
        """Starting a background bash should give the LLM instructions to poll."""
        setup_provider()
        r = eli_run(
            "Run `sleep 1 && echo DONE` in the background using bash with background=true. "
            "Then immediately check its output with bash.output. "
            "Tell me: what was the status when you checked? Was it running or finished? "
            "Reply in one sentence."
        )
        assert r.ok, f"Failed: {r.stderr}"
        out = r.full_output.lower()
        # LLM should mention either running or finished/done/completed
        assert any(w in out for w in ["running", "finish", "done", "complet", "success"]), (
            f"LLM should report background task status. Got: {r.full_output}"
        )


# ---------------------------------------------------------------------------
# P1: subagent spawn message
# ---------------------------------------------------------------------------

# Skipped — subagent tests are covered in test_subagent.py and require external CLIs.


# ---------------------------------------------------------------------------
# P1: web.fetch response too large — recovery suggestions
# ---------------------------------------------------------------------------

# Skipped — hard to trigger reliably without a controlled server.


# ---------------------------------------------------------------------------
# P2: fs.write feedback includes size
# ---------------------------------------------------------------------------

class TestFsWriteFeedback:
    """LLM should see line/byte count after writing a file."""

    def test_write_reports_size(self):
        """After fs.write, LLM gets line count and byte count."""
        setup_provider()
        with tempfile.TemporaryDirectory() as tmpdir:
            target = os.path.join(tmpdir, "test_signal.txt")
            r = eli_run(
                f"Write the following 3 lines to {target} using fs.write:\n"
                f"line one\nline two\nline three\n\n"
                f"After writing, tell me: how many lines and how many bytes did the tool "
                f"report were written? Reply with the exact numbers from the tool result."
            )
            assert r.ok, f"Failed: {r.stderr}"
            out = r.full_output.lower()
            # LLM should mention "3" lines (the tool result says "3 lines, N bytes")
            assert "3" in out, (
                f"LLM should report line count from tool result. Got: {r.full_output}"
            )


# ---------------------------------------------------------------------------
# P2: fs.edit feedback includes line change
# ---------------------------------------------------------------------------

class TestFsEditFeedback:
    """LLM should see old→new line count after editing."""

    def test_edit_reports_line_change(self):
        """After fs.edit, LLM gets old_lines → new_lines."""
        setup_provider()
        with tempfile.TemporaryDirectory() as tmpdir:
            target = os.path.join(tmpdir, "edit_signal.txt")
            # Pre-create a file
            with open(target, "w") as f:
                f.write("hello world\n")

            r = eli_run(
                f"Read the file at {target}, then use fs.edit to replace 'hello world' "
                f"with 'goodbye\ncruel\nworld'. "
                f"After editing, tell me exactly what the tool result said about the edit. "
                f"How many lines changed to how many lines?"
            )
            assert r.ok, f"Failed: {r.stderr}"
            out = r.full_output.lower()
            # The edit went from 1 line to 3 lines
            assert "1" in out and "3" in out, (
                f"LLM should report line count change (1→3). Got: {r.full_output}"
            )


# ---------------------------------------------------------------------------
# P2: decision.set reports active count
# ---------------------------------------------------------------------------

class TestDecisionFeedback:
    """LLM should see active decision count after recording."""

    def test_decision_set_reports_count(self):
        """After decision.set, LLM should know the total active count."""
        setup_provider()
        r = eli_run(
            "Use the decision.set tool to record: 'Use Python for scripting'. "
            "Then tell me: how many active decisions does the tool result say there are? "
            "Reply with just the number."
        )
        assert r.ok, f"Failed: {r.stderr}"
        out = r.full_output.strip()
        # Should mention at least "1"
        assert any(c.isdigit() for c in out), (
            f"LLM should report decision count. Got: {r.full_output}"
        )


# ---------------------------------------------------------------------------
# Integration: LLM correctly reacts to syntax check warning
# ---------------------------------------------------------------------------

class TestSyntaxCheckFeedback:
    """LLM should fix syntax errors when the tool warns about them."""

    def test_edit_with_syntax_error_triggers_fix(self):
        """After fs.edit introduces a syntax error, the LLM should attempt a fix."""
        setup_provider()
        with tempfile.TemporaryDirectory() as tmpdir:
            target = os.path.join(tmpdir, "syntax_test.py")
            with open(target, "w") as f:
                f.write("def hello():\n    return 42\n")

            r = eli_run(
                f"Read {target}, then use fs.edit to replace 'return 42' with "
                f"'return (42'. This will break Python syntax. "
                f"If the tool warns about a syntax error, fix it by replacing "
                f"'return (42' with 'return (42)'. "
                f"Tell me what happened step by step."
            )
            assert r.ok, f"Failed: {r.stderr}"
            # Read the final file to verify it's valid Python
            with open(target) as f:
                content = f.read()
            # Should have been fixed
            assert "return (42)" in content or "return 42" in content, (
                f"LLM should fix the syntax error. File content: {content}"
            )


# ---------------------------------------------------------------------------
# Integration: truncation pagination signal
# ---------------------------------------------------------------------------

class TestTruncationFeedback:
    """LLM should understand truncation signal and paginate."""

    def test_large_file_triggers_pagination(self):
        """When fs.read truncates, the LLM gets offset instructions and can continue."""
        setup_provider()
        with tempfile.TemporaryDirectory() as tmpdir:
            target = os.path.join(tmpdir, "big.txt")
            with open(target, "w") as f:
                for i in range(600):
                    f.write(f"line-{i:04d}: padding data here\n")
                f.write("FINAL_MARKER_XYZ\n")

            r = eli_run(
                f"Read the file at {target} using fs.read. "
                f"If the output is truncated, read the remaining lines using the "
                f"offset and limit values from the truncation message. "
                f"What is the very last line of the file? Quote it exactly."
            )
            assert r.ok, f"Failed: {r.stderr}"
            assert "FINAL_MARKER_XYZ" in r.full_output, (
                f"LLM should paginate and find the last line. Got: {r.full_output}"
            )
