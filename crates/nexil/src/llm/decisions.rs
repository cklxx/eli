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
    let revoked: HashSet<String> = entries
        .iter()
        .filter(|e| e.kind == "decision_revoked")
        .filter_map(|e| e.payload.get("text").and_then(|v| v.as_str()))
        .map(str::to_owned)
        .collect();

    entries
        .iter()
        .filter(|e| e.kind == "decision")
        .filter_map(|e| e.payload.get("text").and_then(|v| v.as_str()))
        .filter(|text| !text.is_empty() && !revoked.contains(*text))
        .map(str::to_owned)
        .collect()
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

    let block = decisions.iter().enumerate().fold(
        String::from("\n\nActive decisions:"),
        |mut acc, (i, d)| {
            acc.push_str(&format!("\n{}. {}", i + 1, d));
            acc
        },
    );

    let last_system_idx = messages
        .iter()
        .rposition(|msg| msg.get("role").and_then(|r| r.as_str()) == Some("system"));

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
