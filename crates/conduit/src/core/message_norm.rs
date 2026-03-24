//! Message normalization, orphan pruning, and message rule enforcement.

use std::collections::HashSet;

use serde_json::Value;

use super::tool_calls::normalize_message_tool_calls;
use crate::clients::parsing::TransportKind;

/// Normalize messages to ensure protocol compliance before sending to any LLM API.
///
/// - Removes orphan tool_use blocks (no matching tool_result follows)
/// - Removes orphan tool_result messages (no matching tool_use precedes)
pub fn normalize_messages_for_api(messages: Vec<Value>, transport: TransportKind) -> Vec<Value> {
    let normalized_messages: Vec<Value> = messages
        .into_iter()
        .map(|message| normalize_message_tool_calls(&message))
        .collect();
    let result = prune_orphan_tool_messages(normalized_messages);

    // Anthropic-specific role merging is intentionally deferred to
    // `build_messages_body`, where tool results have already been converted into
    // Anthropic content blocks. Doing it earlier on the generic message shape
    // can collapse multiple `role=tool` messages and drop call IDs.
    if transport == TransportKind::Messages {
        return result;
    }

    result
}

/// Remove orphan tool_use assistant messages and orphan tool_result messages.
///
/// A tool_result is orphan when no assistant message has a matching tool_call id.
/// An assistant message with tool_calls is orphan when any of its calls lack a
/// matching tool_result.
pub(crate) fn prune_orphan_tool_messages(messages: Vec<Value>) -> Vec<Value> {
    // Collect all tool_call IDs from assistant messages
    let mut tool_call_ids: HashSet<String> = HashSet::new();
    for msg in &messages {
        if let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
            for call in calls {
                if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                    tool_call_ids.insert(id.to_owned());
                }
            }
        }
    }

    // Collect all tool_result IDs
    let mut tool_result_ids: HashSet<String> = HashSet::new();
    for msg in &messages {
        if msg.get("role").and_then(|r| r.as_str()) == Some("tool")
            && let Some(id) = msg.get("tool_call_id").and_then(|v| v.as_str())
        {
            tool_result_ids.insert(id.to_owned());
        }
    }

    // Filter: keep messages that are not orphans
    let mut filtered = Vec::new();
    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

        if role == "tool" {
            // Keep tool result only if its call_id has a matching tool_use
            let call_id = msg
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if call_id.is_empty() || !tool_call_ids.contains(call_id) {
                continue; // Drop orphan tool result
            }
        }

        if role == "assistant"
            && let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array())
        {
            // Check if ALL tool_calls have matching results
            let all_have_results = calls.iter().all(|call| {
                call.get("id")
                    .and_then(|v| v.as_str())
                    .map(|id| tool_result_ids.contains(id))
                    .unwrap_or(false)
            });
            if !all_have_results && !calls.is_empty() {
                // Drop assistant message with orphan tool_calls
                continue;
            }
        }

        filtered.push(msg);
    }

    filtered
}

/// Enforce Anthropic-specific message ordering rules.
///
/// - Merges consecutive same-role messages (except system).
/// - Inserts a synthetic "user" message at the start if needed.
/// - Appends a synthetic "user" message at the end if the last message is "assistant".
#[cfg(test)]
pub(crate) fn enforce_anthropic_message_rules(messages: Vec<Value>) -> Vec<Value> {
    if messages.is_empty() {
        return messages;
    }

    let mut result: Vec<Value> = Vec::new();

    for msg in messages {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_owned();

        // Skip system messages in this pass (they go separately in Anthropic API)
        if role == "system" {
            result.push(msg);
            continue;
        }

        // Merge consecutive same-role messages
        if let Some(last) = result.last() {
            let last_role = last.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if last_role == role && role != "system" {
                // Merge: append content to previous message
                let prev_content = last.get("content").and_then(|c| c.as_str()).unwrap_or("");
                let new_content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                if !new_content.is_empty() {
                    let merged = format!("{prev_content}\n\n{new_content}");
                    if let Some(last_mut) = result.last_mut() {
                        if let Some(obj) = last_mut.as_object_mut() {
                            obj.insert("content".to_owned(), Value::String(merged));
                        }
                    }
                }
                continue;
            }
        }

        result.push(msg);
    }

    // Ensure first non-system message is "user"
    let first_non_system = result
        .iter()
        .position(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"));
    if let Some(idx) = first_non_system {
        if result[idx].get("role").and_then(|r| r.as_str()) != Some("user") {
            result.insert(
                idx,
                serde_json::json!({"role": "user", "content": "Continue."}),
            );
        }
    }

    // Ensure last message is "user" (but NOT if it ends with tool results, which is valid)
    if let Some(last) = result.last() {
        let last_role = last.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if last_role == "assistant" {
            result.push(serde_json::json!({"role": "user", "content": "Continue."}));
        }
    }

    result
}
