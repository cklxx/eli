## Status
done

## Summary
Reviewed all 15 files under crates/conduit/src/core/ for logic bugs, race conditions, incorrect error handling, off-by-one errors, missing edge cases, and unsound state transitions. Found 11 issues across 7 files.

## Findings

### 1. response_parser.rs:161-198 — Severity: MEDIUM — Completion SSE tool call delta merging is broken
**Description:** In the completion-format SSE branch, `tool_calls` deltas are accumulated by simply extending a flat vec (`tool_calls.extend(tc.iter().cloned())`). Completion streaming sends incremental deltas with the same `index` field, where each delta contains a partial `function.arguments` fragment. The code never merges these deltas by index — it just appends every chunk as a separate entry. The assembled response will contain N entries per tool call (one per SSE chunk) instead of one merged entry. This will cause downstream tool execution to see duplicate/incomplete tool calls.

**Fix:** Merge deltas by their `index` field, accumulating `function.arguments` strings, similar to how the Messages branch handles `input_json_delta`.

### 2. response_parser.rs:113-118 — Severity: LOW — Messages SSE usage only captured from message_delta, not message_start
**Description:** The `message_delta` event captures usage, but Anthropic also sends usage in the `message_start` event (input_tokens). The current code only captures `message_delta` usage (output_tokens), so the assembled response may have incomplete usage data. Not a correctness bug but a data loss issue for token accounting.

### 3. message_norm.rs:76-89 — Severity: MEDIUM — Orphan pruning drops assistant message when ANY tool call lacks a result
**Description:** `prune_orphan_tool_messages` drops an entire assistant message (including its text content) if ANY of its tool_calls lack a matching tool result. If an assistant message has both text content AND tool calls, and one tool result is missing, the text content is silently lost. A more robust approach would strip only the orphan tool_calls from the assistant message while preserving the text content.

### 4. message_norm.rs:125-139 — Severity: LOW — Consecutive role merging loses tool_calls and structured content
**Description:** `enforce_anthropic_message_rules` merges consecutive same-role messages by concatenating `.content` as strings. If either message has structured content (arrays) or tool_calls, those are silently dropped during the merge. Only applies to the `#[cfg(test)]` version, so not a production bug, but the test helper produces incorrect results.

### 5. execution.rs:272-283 — Severity: LOW — Transport selection recomputed identically on every retry attempt
**Description:** Inside the retry loop, `runtime.selected_transport(...)` is called on every attempt with the same arguments. Since the result is deterministic, this is wasted work. More importantly, if it fails on the first attempt it breaks out of the inner loop — but `supports_responses` is hardcoded to `false`, which means callers cannot override it. This is a design limitation noted in the comment but could lead to incorrect transport selection.

### 6. execution.rs:288-289 — Severity: LOW — Messages cloned on every retry attempt
**Description:** `messages_payload.clone()` is called inside the retry loop for every attempt and every candidate. For large message histories, this creates unnecessary allocations. Consider cloning once per candidate or restructuring to avoid repeated clones.

### 7. anthropic_messages.rs:57-73 — Severity: MEDIUM — resolved_ids check allows partial tool result sets
**Description:** In `split_system_and_conversation`, when an assistant message has tool calls, the lookahead collects resolved tool result IDs. If only a subset of tool calls have results, the code still emits ALL tool_use blocks (via `assistant_content_blocks` with `Some(&resolved_ids)`) — but wait, looking more carefully, it passes `resolved_ids` as the allowed set, so it does filter. However, the unresolved tool_use blocks are silently dropped without the corresponding `tool_result` error blocks that Anthropic requires. This could cause an API error from Anthropic if tool_use blocks are present without matching tool_results.

### 8. anthropic_messages.rs:86-88 — Severity: LOW — Orphan tool messages silently skipped
**Description:** Messages with `role: "tool"` that appear outside of a tool-call lookahead window are silently dropped (line 86-88). No warning or logging. This is intentional but can make debugging difficult.

### 9. client_registry.rs:119-121 — Severity: LOW — OAuth token detection is case-insensitive but prefix check uses lowercase conversion
**Description:** `is_oauth_token` uses `k.to_ascii_lowercase().starts_with("sk-ant-oat")` but the actual token prefix is case-sensitive. The `to_ascii_lowercase()` call is unnecessary since OAuth tokens always start with lowercase `sk-ant-oat`. Not a bug, but inconsistent with `provider_runtime.rs:76` which uses `starts_with` directly without lowering.

### 10. request_builder.rs:228-234 — Severity: LOW — convert_messages_to_responses_input drops multimodal content
**Description:** `content_str` extracts content as a plain string via `as_str()`. If a user/assistant message has array-type content (e.g., with images or structured blocks), `content_str` will be empty and the message content will be lost in the responses conversion, while the message entry is still emitted with empty content.

### 11. error_classify.rs:43-48 — Severity: LOW — Text signature classifier has false positive risk for "auth" substring
**Description:** The classifier matches any error message containing the substring "auth" — this would match words like "authorization" (intended) but also "author", "authenticate" in non-error contexts, or provider error messages that mention "auth" in a different context (e.g., "auth token refreshed successfully"). The broad substring match could misclassify provider errors as Config errors, preventing retries.

## Changes
- .openmax/reports/review-conduit-core.md: Created review findings report

## Test Results
N/A — this is a review-only task, no code changes made.
