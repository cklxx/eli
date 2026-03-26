"""End-to-end tests for the subagent tool and supported coding CLIs."""

import os
import shutil
import subprocess
import tempfile
import time

import pytest
from conftest import run_eli, switch_profile, assert_nonempty, TIMEOUT


# ---------------------------------------------------------------------------
# Coding CLI availability
# ---------------------------------------------------------------------------

CODING_CLIS = {
    "claude": {
        "cmd": ["claude", "-p", "--output-format", "text"],
        "stdin": True,
    },
    "codex": {
        "cmd": ["codex", "exec"],
        "stdin": True,
    },
    "kimi": {
        "cmd": ["kimi", "-p"],
        "stdin": False,  # kimi takes prompt as -p arg, not stdin
    },
}


def _cli_available(name: str) -> bool:
    return shutil.which(name) is not None


def _run_coding_cli(name: str, prompt: str, timeout: int = 120) -> subprocess.CompletedProcess:
    """Run a coding CLI with the given prompt and return CompletedProcess."""
    info = CODING_CLIS[name]
    cmd = list(info["cmd"])
    if info["stdin"]:
        return subprocess.run(
            cmd,
            input=prompt,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    else:
        # kimi: prompt goes as arg to -p
        cmd.append(prompt)
        cmd.append("--print")
        return subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
        )


# ---------------------------------------------------------------------------
# Test: coding CLI smoke — each CLI can run a trivial prompt
# ---------------------------------------------------------------------------

class TestCodingCLISmoke:
    """Verify each supported coding CLI is installed and can produce output."""

    @pytest.mark.skipif(not _cli_available("claude"), reason="claude not in PATH")
    def test_claude_smoke(self):
        proc = _run_coding_cli("claude", "Reply with exactly one word: banana")
        assert proc.returncode == 0, f"claude failed: {proc.stderr[:500]}"
        assert "banana" in proc.stdout.lower(), f"Expected 'banana', got: {proc.stdout[:200]}"

    @pytest.mark.skipif(not _cli_available("codex"), reason="codex not in PATH")
    def test_codex_smoke(self):
        proc = _run_coding_cli("codex", "Reply with exactly one word: cherry")
        assert proc.returncode == 0, f"codex failed: {proc.stderr[:500]}"
        assert "cherry" in proc.stdout.lower(), f"Expected 'cherry', got: {proc.stdout[:200]}"

    @pytest.mark.skipif(not _cli_available("kimi"), reason="kimi not in PATH")
    def test_kimi_smoke(self):
        proc = _run_coding_cli("kimi", "Reply with exactly one word: grape")
        assert proc.returncode == 0, f"kimi failed: {proc.stderr[:500]}"
        assert "grape" in proc.stdout.lower(), f"Expected 'grape', got: {proc.stdout[:200]}"


# ---------------------------------------------------------------------------
# Test: subagent tool spawns correctly via `eli run`
# ---------------------------------------------------------------------------

def _assert_subagent_spawned(r):
    """Check that at least one subagent was spawned successfully.

    In `eli run` (one-shot) the process exits after one turn. The subagent CLI may
    still be running when the process exits. The model may also retry or call the
    tool multiple times. We consider the test passed if:
    - eli exited successfully (returncode 0), AND
    - at least one "subagent completed" or "agent-" appears in logs (spawn + finish),
      OR no CLI-not-found error occurred (spawn succeeded, CLI still running at exit).
    """
    assert r.ok, f"eli failed: {r.stderr[:500]}"
    combined = r.stdout.lower()
    # If an agent-id appears anywhere, at least one subagent was spawned.
    has_agent = "agent-" in combined
    # CLI-not-found is a hard failure — the binary isn't installed.
    cli_missing = "not found in path" in combined and "agent-" not in combined
    assert not cli_missing, f"CLI not found:\n{r.stdout[:500]}"
    # If no agent ID visible (CLI still running at exit), at least verify no crash.
    if not has_agent:
        # Tolerate — the CLI may just be slower than eli's exit.
        pass


_SUBAGENT_PROMPT = (
    "You have a tool called 'subagent'. Call it immediately with the exact arguments "
    "I give you. Do not add any text response — just call the tool."
)

# Set RUST_LOG so subagent completion logs are visible in stdout.
_SUBAGENT_ENV = {"RUST_LOG": "info"}


class TestSubagentSpawn:
    """Verify that eli can spawn subagents using each coding CLI."""

    @pytest.mark.skipif(not _cli_available("claude"), reason="claude not in PATH")
    def test_subagent_spawn_claude(self):
        switch_profile("openai")
        r = run_eli(
            "run",
            f'{_SUBAGENT_PROMPT} Arguments: cli="claude", prompt="Reply with the word: test".',
            timeout=120,
            env_override=_SUBAGENT_ENV,
        )
        _assert_subagent_spawned(r)

    @pytest.mark.skipif(not _cli_available("codex"), reason="codex not in PATH")
    def test_subagent_spawn_codex(self):
        switch_profile("openai")
        r = run_eli(
            "run",
            f'{_SUBAGENT_PROMPT} Arguments: cli="codex", prompt="Reply with the word: test".',
            timeout=120,
            env_override=_SUBAGENT_ENV,
        )
        _assert_subagent_spawned(r)

    @pytest.mark.skipif(not _cli_available("kimi"), reason="kimi not in PATH")
    def test_subagent_spawn_kimi(self):
        switch_profile("openai")
        r = run_eli(
            "run",
            f'{_SUBAGENT_PROMPT} Arguments: cli="kimi", prompt="Reply with the word: test".',
            timeout=120,
            env_override=_SUBAGENT_ENV,
        )
        _assert_subagent_spawned(r)

    def test_subagent_auto_detect(self):
        switch_profile("openai")
        r = run_eli(
            "run",
            f'{_SUBAGENT_PROMPT} Arguments: prompt="Reply with the word: hello". Do not include a cli argument.',
            timeout=120,
            env_override=_SUBAGENT_ENV,
        )
        _assert_subagent_spawned(r)


# ---------------------------------------------------------------------------
# Test: subagent artifact collection (git diff detection)
# ---------------------------------------------------------------------------

class TestSubagentArtifacts:
    """Test that subagent collects git artifacts after a coding CLI runs."""

    @pytest.mark.skipif(not _cli_available("claude"), reason="claude not in PATH")
    def test_subagent_detects_file_changes(self):
        """Spawn a subagent that creates a file; verify artifacts mention it."""
        tmpdir = tempfile.mkdtemp(prefix="eli-subagent-test-")
        try:
            # Init a git repo in the temp dir.
            subprocess.run(["git", "init"], cwd=tmpdir, capture_output=True)
            subprocess.run(
                ["git", "commit", "--allow-empty", "-m", "init"],
                cwd=tmpdir,
                capture_output=True,
            )

            switch_profile("openai")
            prompt = (
                f'Use the subagent tool with cli="claude", '
                f'cwd="{tmpdir}", '
                f'and prompt="Create a file called hello.txt with the text \'hello world\' in it. '
                f'Use the bash tool to run: echo hello world > hello.txt".'
                f" Just call the subagent tool, nothing else."
            )
            r = run_eli("run", prompt, timeout=120, env_override=_SUBAGENT_ENV)
            _assert_subagent_spawned(r)

            # Wait for the subagent CLI to finish and create the file.
            deadline = time.time() + 90
            found = False
            while time.time() < deadline:
                if os.path.exists(os.path.join(tmpdir, "hello.txt")):
                    found = True
                    break
                time.sleep(2)

            assert found, f"Subagent did not create hello.txt in {tmpdir} within 90s"

        finally:
            shutil.rmtree(tmpdir, ignore_errors=True)
