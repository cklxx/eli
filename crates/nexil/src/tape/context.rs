//! Context building for tape entries.

use std::sync::Arc;

use serde_json::Value;

use crate::core::tool_calls::normalize_message_tool_calls;
use crate::tape::entries::{TapeEntry, TapeEntryKind};
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

fn default_messages(entries: &[TapeEntry]) -> Vec<Value> {
    entries
        .iter()
        .filter(|e| e.kind == TapeEntryKind::Message && e.payload.is_object())
        .map(|e| normalize_message_tool_calls(&e.payload))
        .collect()
}

/// Maximum total bytes across all messages before aggressive trimming kicks in
/// for predominantly ASCII / Latin content (English chars ≈ 4 chars/token,
/// so 400 KB ≈ 100K tokens).
const MAX_TOTAL_CONTEXT_CHARS: usize = 400_000;

/// Lower threshold used when the conversation is predominantly CJK text.
/// CJK chars are each roughly 1–1.5 tokens, so a 400K-char budget can exceed
/// a 128K-token context window. 200K chars keeps us safely within most limits.
const MAX_TOTAL_CONTEXT_CHARS_CJK: usize = 200_000;

/// CJK ratio threshold (0.0–1.0) above which the tighter budget applies.
const CJK_RATIO_THRESHOLD: f64 = 0.30;

/// Number of recent user-message rounds to keep during aggressive trimming.
const AGGRESSIVE_TRIM_KEEP_ROUNDS: usize = 2;

/// Compute the character threshold for context trimming.
///
/// When `context_window` is provided (in tokens), convert to an approximate
/// char budget:
///   - Predominantly CJK text: `context_window * 1.5` (CJK chars ≈ 1 token each,
///     but we allow 1.5× headroom since not all chars are CJK).
///   - Otherwise: `context_window * 4` (ASCII/Latin chars ≈ 4 chars/token).
///
/// When `context_window` is `None`, fall back to the hardcoded constants.
fn compute_char_threshold(messages: &[Value], context_window: Option<usize>) -> usize {
    let is_cjk_heavy = cjk_content_ratio(messages) > CJK_RATIO_THRESHOLD;
    match context_window {
        Some(cw) => {
            if is_cjk_heavy {
                // CJK: ~1 token/char, use 1.5× as char budget
                (cw as f64 * 1.5) as usize
            } else {
                // ASCII: ~4 chars/token
                cw * 4
            }
        }
        None => {
            if is_cjk_heavy {
                MAX_TOTAL_CONTEXT_CHARS_CJK
            } else {
                MAX_TOTAL_CONTEXT_CHARS
            }
        }
    }
}

pub fn apply_context_budget(messages: &mut Vec<Value>, context_window: Option<usize>) {
    let total_chars: usize = messages.iter().map(content_char_count).sum();
    let threshold = compute_char_threshold(messages, context_window);
    if total_chars > threshold {
        let before_count = messages.len();
        aggressive_trim(messages);
        let dropped = before_count.saturating_sub(messages.len());
        if dropped > 0 {
            tracing::warn!(
                dropped_messages = dropped,
                total_chars = total_chars,
                threshold = threshold,
                "context budget exceeded, trimmed conversation history"
            );
        }
    }
}

/// Estimate the fraction of Unicode scalar values in message content that are
/// CJK / wide characters (Japanese, Korean, Chinese, CJK punctuation, etc.).
fn cjk_content_ratio(messages: &[Value]) -> f64 {
    let mut total = 0usize;
    let mut cjk = 0usize;
    for msg in messages {
        if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
            for c in text.chars() {
                total += 1;
                if is_cjk(c) {
                    cjk += 1;
                }
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        cjk as f64 / total as f64
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF    // CJK Unified Ideographs
        | 0x3400..=0x4DBF  // CJK Extension A
        | 0x20000..=0x2A6DF// CJK Extension B
        | 0x3000..=0x303F  // CJK Symbols and Punctuation
        | 0xFF00..=0xFFEF  // Halfwidth and Fullwidth Forms
        | 0x3040..=0x309F  // Hiragana
        | 0x30A0..=0x30FF  // Katakana
        | 0xAC00..=0xD7AF  // Hangul Syllables
    )
}

fn msg_role(msg: &Value) -> &str {
    msg.get("role").and_then(|r| r.as_str()).unwrap_or("")
}

fn content_char_count(msg: &Value) -> usize {
    msg.get("content")
        .and_then(|c| c.as_str())
        .map_or(0, str::len)
}

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

const TRIM_NOTICE: &str = "[Earlier tool interactions trimmed to fit context window. Use tape.search to review full history.]";

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

fn inject_trim_notice(recent: &mut [Value], before: &mut Vec<Value>) {
    if msg_role(&recent[0]) == "assistant" {
        let existing = recent[0]
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("");
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
            assert!(
                a != b || a == "system",
                "consecutive '{a}': {:?} / {:?}",
                w[0],
                w[1]
            );
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
