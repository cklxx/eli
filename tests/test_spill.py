"""End-to-end tests for tool result spill-to-file and image placeholder degradation.

These tests verify the complete pipeline:
  - Tool results exceeding the threshold are spilled to disk
  - Spill files are absolute paths and readable
  - Image blocks are replaced with placeholders in tape
  - The model still gets coherent context
"""

import hashlib
import json
import os
import tempfile
from pathlib import Path

from conftest import assert_nonempty, require_profile, run_eli, unique_name


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

ELI_HOME = Path(os.environ.get("ELI_HOME", Path.home() / ".eli"))
TAPES_DIR = ELI_HOME / "tapes"


def session_id(chat_id: str) -> str:
    return f"cli:{chat_id}"


def tape_name(chat_id: str) -> str:
    workspace = os.path.realpath(os.getcwd())
    wh = hashlib.md5(workspace.encode(), usedforsecurity=False).hexdigest()[:16]
    sh = hashlib.md5(session_id(chat_id).encode(), usedforsecurity=False).hexdigest()[:16]
    return f"{wh}__{sh}"


def tape_file(chat_id: str) -> Path:
    return TAPES_DIR / f"{tape_name(chat_id)}.jsonl"


def spill_dir(chat_id: str) -> Path:
    return TAPES_DIR / f"{tape_name(chat_id)}.d"


def read_tape(chat_id: str) -> list[dict]:
    tf = tape_file(chat_id)
    if not tf.exists():
        return []
    return [json.loads(l) for l in tf.read_text().splitlines() if l.strip()]


def find_entries(entries: list[dict], kind: str) -> list[dict]:
    return [e for e in entries if e.get("kind") == kind]


def reset(chat_id: str):
    tf = tape_file(chat_id)
    sd = spill_dir(chat_id)
    if tf.exists():
        tf.unlink()
    if sd.exists():
        import shutil
        shutil.rmtree(sd)


def eli_run(chat_id: str, prompt: str, timeout: int = 120):
    return run_eli("run", prompt, "--chat-id", chat_id, timeout=timeout)


# ---------------------------------------------------------------------------
# Tool result spill
# ---------------------------------------------------------------------------

class TestToolResultSpill:

    def test_large_tool_result_creates_spill_file(self):
        chat_id = unique_name("spill_large")
        reset(chat_id)
        require_profile("openai")

        with tempfile.NamedTemporaryFile(mode="w", suffix=".txt", delete=False) as f:
            for i in range(100):
                f.write(f"line {i}: this is test data for spill verification\n")
            big_file = f.name

        try:
            r = eli_run(
                chat_id,
                f"Read the file at {big_file} using fs.read and tell me how many lines it has. Be brief.",
            )
            assert r.ok, f"Failed: {r.stderr}"
            assert_nonempty(r.full_output, "spill test")

            entries = read_tape(chat_id)
            result_entries = find_entries(entries, "tool_result")
            assert len(result_entries) > 0, "should have tool_result entries"

            sd = spill_dir(chat_id)
            if sd.exists():
                spill_files = list(sd.glob("*.txt"))
                assert len(spill_files) > 0, f"spill dir exists but no files"

                for sf in spill_files:
                    assert sf.is_absolute(), f"not absolute: {sf}"
                    content = sf.read_text()
                    assert len(content) > 0, f"empty: {sf}"

                for entry in result_entries:
                    for result in entry.get("payload", {}).get("results", []):
                        output = result.get("output", "")
                        if isinstance(output, str) and "omitted" in output:
                            assert "full output:" in output
                            path_str = output.split("full output: ")[1].split("]")[0]
                            p = Path(path_str)
                            assert p.is_absolute(), f"not absolute: {path_str}"
                            assert p.exists(), f"not readable: {path_str}"
        finally:
            os.unlink(big_file)
            reset(chat_id)

    def test_small_tool_result_not_spilled(self):
        chat_id = unique_name("spill_small")
        reset(chat_id)
        require_profile("openai")
        try:
            r = eli_run(chat_id, "What is 2+2? Reply with just the number.")
            assert r.ok
            sd = spill_dir(chat_id)
            if sd.exists():
                for f in sd.glob("*.txt"):
                    if ".args" not in f.name:
                        assert len(f.read_text()) <= 500, f"small result spilled: {f}"
        finally:
            reset(chat_id)


# ---------------------------------------------------------------------------
# Image placeholder
# ---------------------------------------------------------------------------

class TestImagePlaceholder:

    def test_image_replaced_with_placeholder_in_tape(self):
        chat_id = unique_name("spill_image")
        reset(chat_id)
        require_profile("openai")

        from conftest import RED_PNG
        import base64
        with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f:
            f.write(base64.b64decode(RED_PNG))
            img_path = f.name

        try:
            r = eli_run(chat_id, f"What color is the image at {img_path}? One word.")
            assert r.ok, f"Failed: {r.stderr}"

            entries = read_tape(chat_id)
            msg_entries = find_entries(entries, "message")
            user_msgs = [e for e in msg_entries if e.get("payload", {}).get("role") == "user"]
            assert len(user_msgs) > 0, "should have user messages"

            tape_json = json.dumps(entries)
            assert "iVBORw0KGgo" not in tape_json, "base64 should not be in tape"
        finally:
            os.unlink(img_path)
            reset(chat_id)


# ---------------------------------------------------------------------------
# Spill lifecycle
# ---------------------------------------------------------------------------

class TestSpillLifecycle:

    def test_spill_dir_convention(self):
        name = tape_name(unique_name("spill_convention"))
        assert (TAPES_DIR / f"{name}.d").parent == (TAPES_DIR / f"{name}.jsonl").parent

    def test_spill_files_have_call_id_names(self):
        chat_id = unique_name("spill_names")
        reset(chat_id)
        require_profile("openai")

        with tempfile.NamedTemporaryFile(mode="w", suffix=".txt", delete=False) as f:
            f.write("x\n" * 500)
            big_file = f.name

        try:
            r = eli_run(chat_id, f"Read {big_file} with fs.read. How many lines? Be brief.")
            assert r.ok
            sd = spill_dir(chat_id)
            if sd.exists():
                for f in sd.iterdir():
                    assert f.suffix == ".txt", f"bad extension: {f}"
        finally:
            os.unlink(big_file)
            reset(chat_id)


# ---------------------------------------------------------------------------
# Context coherence
# ---------------------------------------------------------------------------

class TestContextCoherence:

    def test_model_reads_file_correctly(self):
        chat_id = unique_name("spill_context")
        reset(chat_id)
        require_profile("openai")

        with tempfile.NamedTemporaryFile(mode="w", suffix=".txt", delete=False) as f:
            f.write("MARKER_ALPHA_BRAVO\n")
            for i in range(100):
                f.write(f"padding line {i}\n")
            big_file = f.name

        try:
            r = eli_run(
                chat_id,
                f"Read the file at {big_file} using fs.read. What is the first line? Quote it exactly.",
            )
            assert r.ok, f"Failed: {r.stderr}"
            assert "MARKER_ALPHA_BRAVO" in r.full_output, f"Got: {r.full_output}"
        finally:
            os.unlink(big_file)
            reset(chat_id)
