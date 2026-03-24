//! Decision injection — collects active decisions from tape entries and injects
//! them into the system prompt.

use std::collections::HashSet;

use serde_json::Value;

use crate::tape::entries::TapeEntry;

/// Collect active (non-revoked) decisions from tape entries.
///
/// Scans ALL entries regardless of anchor slicing. Decisions revoked by a
/// matching `decision_revoked` tombstone are excluded. Returns the text of
/// each active decision in chronological order.
pub fn collect_active_decisions(entries: &[TapeEntry]) -> Vec<String> {
    let mut decisions: Vec<String> = Vec::new();
    let mut revoked: HashSet<String> = HashSet::new();

    // First pass: collect all revocations
    for entry in entries {
        if entry.kind == "decision_revoked"
            && let Some(text) = entry.payload.get("text").and_then(|v| v.as_str())
        {
            revoked.insert(text.to_string());
        }
    }

    // Second pass: collect decisions not revoked
    for entry in entries {
        if entry.kind == "decision"
            && let Some(text) = entry.payload.get("text").and_then(|v| v.as_str())
            && !revoked.contains(text)
            && !text.is_empty()
        {
            decisions.push(text.to_string());
        }
    }

    decisions
}

/// Inject active decisions into the system prompt of a message list.
///
/// Finds the last system message and appends a decision block to its content.
/// If no system message exists, creates one. The decision block is formatted as
/// a numbered list under an "Active decisions:" header.
pub fn inject_decisions_into_system_prompt(messages: &mut Vec<Value>, decisions: &[String]) {
    if decisions.is_empty() {
        return;
    }

    let mut block = String::from("\n\nActive decisions:");
    for (i, decision) in decisions.iter().enumerate() {
        block.push_str(&format!("\n{}. {}", i + 1, decision));
    }

    // Find the last system message and append to it
    let mut last_system_idx = None;
    for (i, msg) in messages.iter().enumerate() {
        if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
            last_system_idx = Some(i);
        }
    }

    if let Some(idx) = last_system_idx {
        if let Some(obj) = messages[idx].as_object_mut() {
            let existing = obj.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let new_content = format!("{}{}", existing, block);
            obj.insert("content".to_owned(), Value::String(new_content));
        }
    } else {
        // No system message exists — create one
        messages.insert(
            0,
            serde_json::json!({"role": "system", "content": block.trim_start()}),
        );
    }
}
