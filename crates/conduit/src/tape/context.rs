//! Context building for tape entries.

use std::sync::Arc;

use serde_json::Value;

use crate::core::tool_calls::normalize_message_tool_calls;
use crate::tape::entries::TapeEntry;
use crate::tape::query::TapeQuery;

/// Selector for which anchor to use when building context.
#[derive(Debug, Clone, Default)]
pub enum AnchorSelector {
    /// Use the most recent anchor in the tape.
    #[default]
    LastAnchor,
    /// Use a specific named anchor.
    Named(String),
    /// No anchor filtering; use the full tape.
    None,
}

/// Custom selector function type: takes entries + context, returns messages.
/// Wrapped in Arc so that TapeContext can be Clone.
pub type SelectFn = Arc<dyn Fn(&[TapeEntry], &TapeContext) -> Vec<Value> + Send + Sync>;

/// Rules for selecting tape entries into a prompt context.
#[derive(Clone)]
pub struct TapeContext {
    /// Which anchor strategy to use.
    pub anchor: AnchorSelector,
    /// Optional custom selector called after anchor slicing.
    pub select: Option<SelectFn>,
    /// Optional state dictionary passed along with the context.
    pub state: Value,
}

impl Default for TapeContext {
    fn default() -> Self {
        Self {
            anchor: AnchorSelector::default(),
            select: None,
            state: Value::Object(serde_json::Map::new()),
        }
    }
}

impl std::fmt::Debug for TapeContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TapeContext")
            .field("anchor", &self.anchor)
            .field("select", &self.select.as_ref().map(|_| "<fn>"))
            .field("state", &self.state)
            .finish()
    }
}

impl TapeContext {
    /// Create a new TapeContext with defaults (LastAnchor, no selector, empty state).
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply the anchor selector to a TapeQuery, returning the modified query.
    pub fn build_query(&self, query: TapeQuery) -> TapeQuery {
        match &self.anchor {
            AnchorSelector::None => query,
            AnchorSelector::LastAnchor => query.last_anchor(),
            AnchorSelector::Named(name) => query.after_anchor(name.clone()),
        }
    }
}

/// Build message dicts from tape entries using the given context.
///
/// If the context has a custom `select` function, delegate to it.
/// Otherwise use the default: extract entries with kind == "message" and return their payloads.
pub fn build_messages(entries: &[TapeEntry], context: &TapeContext) -> Vec<Value> {
    if let Some(ref select) = context.select {
        return select(entries, context);
    }
    default_messages(entries)
}

/// Default message extraction: filter to kind == "message" entries and return their payloads.
fn default_messages(entries: &[TapeEntry]) -> Vec<Value> {
    let mut messages = Vec::new();
    for entry in entries {
        if entry.kind != "message" {
            continue;
        }
        if !entry.payload.is_object() {
            continue;
        }
        messages.push(normalize_message_tool_calls(&entry.payload));
    }
    messages
}

/// Maximum characters for a single tool result content before truncation.
const MAX_TOOL_RESULT_CHARS: usize = 16_000;

/// Maximum total characters across all messages before aggressive trimming kicks in.
const MAX_TOTAL_CONTEXT_CHARS: usize = 400_000;

/// Number of recent user-message rounds to keep during aggressive trimming.
const AGGRESSIVE_TRIM_KEEP_ROUNDS: usize = 2;

/// Apply context budget: truncate large tool results and trim if total exceeds budget.
pub fn apply_context_budget(messages: &mut Vec<Value>) {
    // Phase 1: Truncate individual tool results
    for msg in messages.iter_mut() {
        let is_tool = msg.get("role").and_then(|r| r.as_str()) == Some("tool");
        if is_tool {
            truncate_tool_result_content(msg, MAX_TOOL_RESULT_CHARS);
        }
    }

    // Phase 2: If total still exceeds budget, aggressive trim
    let total_chars: usize = messages.iter().map(content_char_count).sum();
    if total_chars > MAX_TOTAL_CONTEXT_CHARS {
        aggressive_trim(messages);
    }
}

/// Truncate a tool result message's content to `limit` bytes, cutting at a
/// char-safe line boundary.
fn truncate_tool_result_content(msg: &mut Value, limit: usize) {
    let content = match msg.get("content").and_then(|c| c.as_str()) {
        Some(s) => s,
        None => return,
    };
    if content.len() <= limit {
        return;
    }

    // Find the largest char boundary <= limit.
    let safe_limit = (0..=limit)
        .rev()
        .find(|&i| content.is_char_boundary(i))
        .unwrap_or(0);

    // Cut at last newline before that boundary.
    let cut = content[..safe_limit].rfind('\n').unwrap_or(safe_limit);
    let shown_lines = content[..cut].matches('\n').count() + 1;
    let total_lines = content.matches('\n').count() + 1;

    let truncated = format!(
        "{}\n\n[Truncated: {}/{} lines shown ({}/{} chars). Use tape.search to see full output.]",
        &content[..cut],
        shown_lines,
        total_lines,
        cut,
        content.len()
    );

    if let Some(obj) = msg.as_object_mut() {
        obj.insert("content".to_owned(), Value::String(truncated));
    }
}

/// Count characters in a message's content field.
fn content_char_count(msg: &Value) -> usize {
    msg.get("content")
        .and_then(|c| c.as_str())
        .map(|s| s.len())
        .unwrap_or(0)
}

/// Aggressive trim: keep system messages + last 2 complete tool interaction rounds.
fn aggressive_trim(messages: &mut Vec<Value>) {
    // Separate system messages from conversation
    let system_msgs: Vec<Value> = messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
        .cloned()
        .collect();

    let conversation: Vec<Value> = messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"))
        .cloned()
        .collect();

    // Find the start of the last AGGRESSIVE_TRIM_KEEP_ROUNDS complete rounds.
    // A round = user msg + assistant msg (possibly with tool_calls) + tool results.
    // Walk backwards to find where the Nth-to-last user message starts.
    let mut user_count = 0;
    let mut keep_from = conversation.len();
    for (i, msg) in conversation.iter().enumerate().rev() {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role == "user" {
            user_count += 1;
            if user_count >= AGGRESSIVE_TRIM_KEEP_ROUNDS {
                keep_from = i;
                break;
            }
        }
    }
    // If we didn't find enough user messages, keep everything
    if user_count < AGGRESSIVE_TRIM_KEEP_ROUNDS {
        keep_from = 0;
    }

    let mut recent: Vec<Value> = conversation[keep_from..].to_vec();

    messages.clear();
    messages.extend(system_msgs);
    if keep_from > 0 {
        const TRIM_NOTICE: &str = "[Earlier tool interactions trimmed to fit context window. Use tape.search to review full history.]";
        // If the kept conversation starts with an assistant message, prepend
        // the trim notice to its content instead of injecting a separate
        // assistant message (which would violate alternating-roles constraints).
        if recent
            .first()
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str())
            == Some("assistant")
        {
            let first = &mut recent[0];
            let existing = first
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            first["content"] =
                Value::String(format!("{TRIM_NOTICE}\n\n{existing}"));
        } else {
            messages.push(serde_json::json!({
                "role": "assistant",
                "content": TRIM_NOTICE
            }));
        }
    }
    messages.extend(recent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Verify that aggressive_trim does not produce consecutive assistant messages
    /// when the kept conversation starts with an assistant message.
    #[test]
    fn aggressive_trim_no_consecutive_assistant_when_recent_starts_with_assistant() {
        // Build a conversation with many rounds so trimming kicks in.
        // After the old user messages, the kept portion will start with
        // an assistant message (the tool_calls response).
        let mut messages = vec![
            json!({"role": "system", "content": "You are helpful."}),
        ];

        // Add enough rounds so the first ones get trimmed.
        // AGGRESSIVE_TRIM_KEEP_ROUNDS is 2, so we need >2 user messages.
        for i in 0..4 {
            messages.push(json!({"role": "user", "content": format!("question {i}")}));
            messages.push(json!({"role": "assistant", "content": format!("answer {i}")}));
        }

        // Now make the kept portion start with assistant by adding a tool round:
        // user -> assistant (tool_call) -> tool -> assistant
        // We want 2 user rounds at the end where the boundary falls on an assistant msg.
        let mut messages2 = vec![
            json!({"role": "system", "content": "You are helpful."}),
        ];
        // Old rounds that will be trimmed
        for i in 0..3 {
            messages2.push(json!({"role": "user", "content": format!("old q{i}")}));
            messages2.push(json!({"role": "assistant", "content": format!("old a{i}")}));
        }
        // This assistant msg will be the start of `recent` if keep_from lands here
        messages2.push(json!({"role": "assistant", "content": "continued thought"}));
        messages2.push(json!({"role": "user", "content": "recent q1"}));
        messages2.push(json!({"role": "assistant", "content": "recent a1"}));
        messages2.push(json!({"role": "user", "content": "recent q2"}));
        messages2.push(json!({"role": "assistant", "content": "recent a2"}));

        aggressive_trim(&mut messages2);

        // Check no two consecutive messages share the same role
        for window in messages2.windows(2) {
            let role_a = window[0].get("role").and_then(|r| r.as_str()).unwrap_or("");
            let role_b = window[1].get("role").and_then(|r| r.as_str()).unwrap_or("");
            assert!(
                role_a != role_b || role_a == "system",
                "consecutive messages with role '{role_a}' found: {:?} and {:?}",
                window[0],
                window[1],
            );
        }
    }

    /// When kept conversation starts with a user message, the trim notice
    /// should be injected as a separate assistant message (original behavior).
    #[test]
    fn aggressive_trim_injects_notice_before_user() {
        let mut messages = vec![
            json!({"role": "system", "content": "system"}),
        ];
        for i in 0..4 {
            messages.push(json!({"role": "user", "content": format!("q{i}")}));
            messages.push(json!({"role": "assistant", "content": format!("a{i}")}));
        }

        aggressive_trim(&mut messages);

        // Should have system, then assistant trim notice, then user/assistant pairs
        assert_eq!(
            messages[0].get("role").and_then(|r| r.as_str()),
            Some("system")
        );
        assert_eq!(
            messages[1].get("role").and_then(|r| r.as_str()),
            Some("assistant")
        );
        assert!(messages[1]
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap()
            .contains("trimmed"));
        assert_eq!(
            messages[2].get("role").and_then(|r| r.as_str()),
            Some("user")
        );
    }
}
