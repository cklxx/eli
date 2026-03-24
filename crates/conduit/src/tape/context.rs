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
    for msg in messages.iter_mut() {
        if msg_role(msg) == "tool" {
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

/// Extract the role string from a message value.
fn msg_role(msg: &Value) -> &str {
    msg.get("role").and_then(|r| r.as_str()).unwrap_or("")
}

/// Count characters in a message's content field.
fn content_char_count(msg: &Value) -> usize {
    msg.get("content").and_then(|c| c.as_str()).map_or(0, str::len)
}

/// Walk backwards through `msgs` and return the index where the last `rounds`
/// user-message rounds begin. Returns 0 if fewer than `rounds` user messages exist.
fn find_trim_boundary(msgs: &[Value], rounds: usize) -> usize {
    let mut seen = 0;
    for (i, m) in msgs.iter().enumerate().rev() {
        if msg_role(m) == "user" {
            seen += 1;
            if seen >= rounds {
                return i;
            }
        }
    }
    0
}

const TRIM_NOTICE: &str =
    "[Earlier tool interactions trimmed to fit context window. Use tape.search to review full history.]";

/// Aggressive trim: keep system messages + last N complete rounds.
/// If the kept portion starts with an assistant message, the trim notice is
/// prepended to it rather than injected as a separate message (which would
/// violate alternating-roles API constraints).
fn aggressive_trim(messages: &mut Vec<Value>) {
    let (system, conversation): (Vec<_>, Vec<_>) =
        messages.drain(..).partition(|m| msg_role(m) == "system");

    let keep_from = find_trim_boundary(&conversation, AGGRESSIVE_TRIM_KEEP_ROUNDS);
    let mut recent: Vec<Value> = conversation.into_iter().skip(keep_from).collect();

    messages.extend(system);
    if keep_from > 0 {
        inject_trim_notice(&mut recent, messages);
    }
    messages.extend(recent);
}

/// Insert the trim notice — either prepended to an existing leading assistant
/// message, or as a new assistant message before the kept portion.
fn inject_trim_notice(recent: &mut [Value], before: &mut Vec<Value>) {
    if msg_role(&recent[0]) == "assistant" {
        let existing = recent[0].get("content").and_then(|c| c.as_str()).unwrap_or("");
        recent[0]["content"] = Value::String(format!("{TRIM_NOTICE}\n\n{existing}"));
    } else {
        before.push(serde_json::json!({"role": "assistant", "content": TRIM_NOTICE}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(role: &str, text: &str) -> Value {
        json!({"role": role, "content": text})
    }

    fn assert_no_consecutive_roles(messages: &[Value]) {
        for w in messages.windows(2) {
            let (a, b) = (msg_role(&w[0]), msg_role(&w[1]));
            assert!(a != b || a == "system", "consecutive '{a}': {:?} / {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn inject_trim_notice_prepends_to_leading_assistant() {
        let mut recent = vec![msg("assistant", "hello"), msg("user", "q")];
        let mut before = vec![msg("system", "sys")];
        inject_trim_notice(&mut recent, &mut before);

        assert_eq!(before.len(), 1); // no extra assistant injected
        let content = recent[0]["content"].as_str().unwrap();
        assert!(content.starts_with(TRIM_NOTICE));
        assert!(content.contains("hello"));
    }

    #[test]
    fn inject_trim_notice_adds_message_before_user() {
        let mut recent = vec![msg("user", "q"), msg("assistant", "a")];
        let mut before = vec![msg("system", "sys")];
        inject_trim_notice(&mut recent, &mut before);

        assert_eq!(before.len(), 2);
        assert_eq!(msg_role(&before[1]), "assistant");
        assert!(before[1]["content"].as_str().unwrap().contains("trimmed"));
    }

    #[test]
    fn aggressive_trim_injects_notice_before_user() {
        let mut msgs = vec![msg("system", "sys")];
        for i in 0..4 {
            msgs.push(msg("user", &format!("q{i}")));
            msgs.push(msg("assistant", &format!("a{i}")));
        }

        aggressive_trim(&mut msgs);
        assert_no_consecutive_roles(&msgs);
        assert_eq!(msg_role(&msgs[1]), "assistant");
        assert!(msgs[1]["content"].as_str().unwrap().contains("trimmed"));
        assert_eq!(msg_role(&msgs[2]), "user");
    }
}
