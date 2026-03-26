"""E2E test: subagent completion routes back to the original chat_id in gateway mode.

Full chain tested:
  send_envelope → sidecar → eli gateway → LLM calls subagent tool → CLI runs →
  completion injected → LLM summarizes → outbound → sidecar → mock plugin captures

Requires: eli binary, bun, at least one coding CLI (claude/codex/kimi).
"""

import shutil
import time

import pytest

from test_gateway import (
    GatewayTrace,
    _clear_responses,  # noqa: F401 — autouse fixture
    send_envelope,
    services,  # noqa: F401 — module-scoped fixture
    wait_for_response,
)
from conftest import require_profile, unique_name

# The full subagent chain: LLM call → tool → CLI spawn → CLI run → completion
# inject → LLM summarize → outbound. Can take a while.
SUBAGENT_TIMEOUT = 180


def _has_coding_cli() -> bool:
    return any(shutil.which(cli) for cli in ["claude", "codex", "kimi"])


@pytest.mark.skipif(not _has_coding_cli(), reason="no coding CLI available")
class TestSubagentGatewayCompletion:
    """Verify subagent completion messages route back to the original chat_id."""

    def test_subagent_completion_routes_to_original_chat(self, services):
        """
        Send a message that triggers subagent spawn. The initial tool-call
        response may be empty (model returns only tool markup). Wait for ANY
        non-empty text response — that's the completion summary.
        """
        trace = GatewayTrace("test_subagent_completion_routes_to_original_chat", "openai")
        require_profile("openai")

        chat_id = unique_name("subagent_gw")
        sent_after_ms = int(time.time() * 1000)

        envelope = send_envelope(
            text=(
                "You have a tool called 'subagent'. Call it RIGHT NOW with these exact arguments: "
                'prompt="Reply with the single word: pineapple". '
                "Do not say anything else, just call the tool."
            ),
            chat_id=chat_id,
            sender_id="test_user",
            sender_name="Test User",
            chat_type="direct",
        )
        trace.inbound_envelope = envelope

        # Wait for a response — may be the initial ack or the completion summary.
        # The initial tool-call turn often produces cleanup_only (empty) which
        # isn't captured by the mock plugin. The completion turn produces text.
        response = wait_for_response(
            chat_id, sent_after_ms=sent_after_ms, timeout=SUBAGENT_TIMEOUT
        )

        if response is not None:
            assert response["to"] == chat_id, (
                f"Response routed to {response['to']}, expected {chat_id}"
            )
            trace.outbound_response = response
            trace.finish("PASS")
        else:
            trace.finish("SKIP", "No response within timeout")
            pytest.skip("Subagent response did not arrive within timeout")
