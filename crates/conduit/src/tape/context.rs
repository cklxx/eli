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

/// Maximum characters for a single tool result content.
const MAX_TOOL_RESULT_CHARS: usize = 16_000;

/// Maximum total characters across all messages before aggressive trimming.
const MAX_TOTAL_CONTEXT_CHARS: usize = 400_000;

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

/// Truncate a tool result message's content to `limit` chars, cutting at line boundary.
fn truncate_tool_result_content(msg: &mut Value, limit: usize) {
    let content = match msg.get("content").and_then(|c| c.as_str()) {
        Some(s) => s,
        None => return,
    };
    if content.len() <= limit {
        return;
    }

    // Cut at last newline before limit
    let cut = content[..limit].rfind('\n').unwrap_or(limit);
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

    // Find the start of the last 2 complete rounds.
    // A round = user msg + assistant msg (possibly with tool_calls) + tool results.
    // Walk backwards to find where the 2nd-to-last user message starts.
    let mut user_count = 0;
    let mut keep_from = conversation.len();
    for (i, msg) in conversation.iter().enumerate().rev() {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role == "user" {
            user_count += 1;
            if user_count >= 2 {
                keep_from = i;
                break;
            }
        }
    }
    // If we didn't find 2 user messages, keep everything
    if user_count < 2 {
        keep_from = 0;
    }

    let recent: Vec<Value> = conversation[keep_from..].to_vec();

    messages.clear();
    messages.extend(system_msgs);
    if keep_from > 0 {
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": "[Earlier tool interactions trimmed to fit context window. Use tape.search to review full history.]"
        }));
    }
    messages.extend(recent);
}
