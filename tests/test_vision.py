"""Vision/multimodal integration tests — send images through the CLI.

These tests write temporary image files, pass them via the framework's
media_paths mechanism, and verify the model actually sees the image content.
"""

import os
import base64
import tempfile
from conftest import (
    BLUE_KEYWORDS,
    BLUE_PNG,
    MAX_VISION_RETRIES,
    RED_KEYWORDS,
    RED_PNG,
    assert_nonempty,
    assert_response_contains,
    require_profile,
    run_eli,
)


def _write_temp_png(b64_data: str, suffix: str = ".png") -> str:
    """Write base64 PNG data to a temp file and return the path."""
    fd, path = tempfile.mkstemp(suffix=suffix)
    with os.fdopen(fd, "wb") as f:
        f.write(base64.b64decode(b64_data))
    return path


def _run_until(prompt: str, matches, context: str) -> str:
    last_output = ""
    for _ in range(MAX_VISION_RETRIES + 1):
        result = run_eli("run", prompt)
        assert result.ok, f"[{context}] Failed: {result.stderr}"
        last_output = result.full_output
        assert_nonempty(last_output, context)
        if matches(last_output.lower()):
            return last_output
    return last_output


# ---------------------------------------------------------------------------
# Single image tests
# ---------------------------------------------------------------------------

class TestSingleImage:
    """Send one image + text prompt, verify the model describes it correctly."""

    def test_openai_red_image(self):
        require_profile("openai")
        img = _write_temp_png(RED_PNG)
        try:
            output = _run_until(
                f"What color is the image at {img}? Answer in one word.",
                lambda text: any(kw in text for kw in RED_KEYWORDS),
                "openai single red",
            )
            assert_response_contains(output, RED_KEYWORDS, "openai single red")
        finally:
            os.unlink(img)

    def test_anthropic_red_image(self):
        require_profile("anthropic")
        img = _write_temp_png(RED_PNG)
        try:
            output = _run_until(
                f"What color is the image at {img}? Answer in one word.",
                lambda text: any(kw in text for kw in RED_KEYWORDS),
                "anthropic single red",
            )
            assert_response_contains(output, RED_KEYWORDS, "anthropic single red")
        finally:
            os.unlink(img)

    def test_openai_blue_image(self):
        require_profile("openai")
        img = _write_temp_png(BLUE_PNG)
        try:
            output = _run_until(
                f"What color is the image at {img}? Answer in one word.",
                lambda text: any(kw in text for kw in BLUE_KEYWORDS),
                "openai single blue",
            )
            assert_response_contains(output, BLUE_KEYWORDS, "openai single blue")
        finally:
            os.unlink(img)


# ---------------------------------------------------------------------------
# Image-only (no text) — does the model actually process the image?
# ---------------------------------------------------------------------------

class TestImageOnly:
    """Send an image with minimal prompt to verify image processing, not hallucination."""

    def test_openai_describe_blue(self):
        """Send blue image, ask to describe — should NOT say red."""
        require_profile("openai")
        img = _write_temp_png(BLUE_PNG)
        try:
            r = run_eli("run", f"Describe the image at {img} in one sentence.")
            assert r.ok, f"Failed: {r.stderr}"
            output = r.full_output
            assert_nonempty(output, "openai describe blue")
            # Should mention blue, should NOT hallucinate red
            lower = output.lower()
            mentions_blue = any(kw in lower for kw in BLUE_KEYWORDS)
            mentions_red = any(kw in lower for kw in RED_KEYWORDS)
            assert mentions_blue or not mentions_red, (
                f"Model may be hallucinating — says red but image is blue:\n{output}"
            )
        finally:
            os.unlink(img)

    def test_anthropic_describe_blue(self):
        require_profile("anthropic")
        img = _write_temp_png(BLUE_PNG)
        try:
            r = run_eli("run", f"Describe the image at {img} in one sentence.")
            assert r.ok, f"Failed: {r.stderr}"
            output = r.full_output
            assert_nonempty(output, "anthropic describe blue")
        finally:
            os.unlink(img)


# ---------------------------------------------------------------------------
# Multi-image
# ---------------------------------------------------------------------------

class TestMultiImage:
    """Send multiple images and verify the model sees both."""

    def test_openai_two_colors(self):
        require_profile("openai")
        red = _write_temp_png(RED_PNG)
        blue = _write_temp_png(BLUE_PNG)
        try:
            output = _run_until(
                f"I have two images: {red} and {blue}. What colors are they? Answer briefly.",
                lambda text: any(kw in text for kw in RED_KEYWORDS)
                and any(kw in text for kw in BLUE_KEYWORDS),
                "openai multi",
            )
            assert_response_contains(output, RED_KEYWORDS, "openai multi red")
            assert_response_contains(output, BLUE_KEYWORDS, "openai multi blue")
        finally:
            os.unlink(red)
            os.unlink(blue)

    def test_anthropic_two_colors(self):
        require_profile("anthropic")
        red = _write_temp_png(RED_PNG)
        blue = _write_temp_png(BLUE_PNG)
        try:
            output = _run_until(
                f"I have two images: {red} and {blue}. What colors are they? Answer briefly.",
                lambda text: any(kw in text for kw in RED_KEYWORDS)
                and any(kw in text for kw in BLUE_KEYWORDS),
                "anthropic multi",
            )
            assert_response_contains(output, RED_KEYWORDS, "anthropic multi red")
            assert_response_contains(output, BLUE_KEYWORDS, "anthropic multi blue")
        finally:
            os.unlink(red)
            os.unlink(blue)
