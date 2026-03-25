//! Structured message types exchanged between channels and the framework.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// MediaType
// ---------------------------------------------------------------------------

/// The kind of media attached to a channel message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,
    Audio,
    Video,
    Document,
}

// ---------------------------------------------------------------------------
// MessageKind
// ---------------------------------------------------------------------------

/// Semantic kind of a channel message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MessageKind {
    #[default]
    Normal,
    Error,
    Command,
}

// ---------------------------------------------------------------------------
// DataFetcher
// ---------------------------------------------------------------------------

/// An async closure that fetches raw media bytes on demand.
pub type DataFetcher = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Vec<u8>> + Send>> + Send + Sync>;

// ---------------------------------------------------------------------------
// MediaItem
// ---------------------------------------------------------------------------

/// A single media attachment on a [`ChannelMessage`].
#[derive(Clone)]
pub struct MediaItem {
    pub media_type: MediaType,
    pub mime_type: String,
    pub filename: Option<String>,
    pub data_fetcher: Option<DataFetcher>,
}

impl std::fmt::Debug for MediaItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaItem")
            .field("media_type", &self.media_type)
            .field("mime_type", &self.mime_type)
            .field("filename", &self.filename)
            .field(
                "data_fetcher",
                &if self.data_fetcher.is_some() {
                    "Some(<fn>)"
                } else {
                    "None"
                },
            )
            .finish()
    }
}

// ---------------------------------------------------------------------------
// ChannelMessage
// ---------------------------------------------------------------------------

/// Structured message data flowing between channels and the framework.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub session_id: String,
    pub channel: String,
    pub content: String,
    #[serde(default = "default_chat_id")]
    pub chat_id: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub kind: MessageKind,
    #[serde(default)]
    pub context: serde_json::Map<String, Value>,
    #[serde(skip)]
    pub media: Vec<MediaItem>,
    #[serde(default)]
    pub output_channel: String,
}

fn default_chat_id() -> String {
    "default".to_owned()
}

impl ChannelMessage {
    /// Create a new [`ChannelMessage`], automatically populating `context` and
    /// `output_channel` in the same way as the Python `__post_init__`.
    pub fn new(
        session_id: impl Into<String>,
        channel: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let channel = channel.into();
        let session_id = session_id.into();
        let content = content.into();
        let chat_id = "default".to_owned();

        let context = serde_json::Map::from_iter([
            ("channel".to_owned(), Value::String(format!("${channel}"))),
            ("chat_id".to_owned(), Value::String(chat_id.clone())),
        ]);

        Self {
            output_channel: channel.clone(),
            session_id,
            channel,
            content,
            chat_id,
            is_active: false,
            kind: MessageKind::Normal,
            context,
            media: Vec::new(),
        }
    }

    /// Builder: set the chat id (also updates context).
    pub fn with_chat_id(mut self, chat_id: impl Into<String>) -> Self {
        self.chat_id = chat_id.into();
        self.context
            .insert("chat_id".to_owned(), Value::String(self.chat_id.clone()));
        self
    }

    /// Builder: mark the message as active.
    pub fn with_is_active(mut self, active: bool) -> Self {
        self.is_active = active;
        self
    }

    /// Builder: set the message kind.
    pub fn with_kind(mut self, kind: MessageKind) -> Self {
        self.kind = kind;
        self
    }

    /// Builder: merge extra context entries.
    pub fn with_context(mut self, extra: serde_json::Map<String, Value>) -> Self {
        for (k, v) in extra {
            self.context.insert(k, v);
        }
        self
    }

    /// Builder: attach media items.
    pub fn with_media(mut self, media: Vec<MediaItem>) -> Self {
        self.media = media;
        self
    }

    /// Builder: override the output channel name.
    pub fn with_output_channel(mut self, ch: impl Into<String>) -> Self {
        self.output_channel = ch.into();
        self
    }

    /// Finalize context (called after all builders). Ensures `channel` and
    /// `chat_id` keys are present.
    pub fn finalize(mut self) -> Self {
        self.context.insert(
            "channel".to_owned(),
            Value::String(format!("${}", self.channel)),
        );
        self.context
            .insert("chat_id".to_owned(), Value::String(self.chat_id.clone()));
        if self.output_channel.is_empty() {
            self.output_channel = self.channel.clone();
        }
        self
    }

    pub fn context_str(&self) -> String {
        self.context
            .iter()
            .map(|(k, v)| match v {
                Value::String(s) => format!("{k}={s}"),
                other => format!("{k}={other}"),
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    /// Combine a batch of messages into a single message.
    ///
    /// Uses the *last* message as the template; content is newline-joined and
    /// media items are concatenated.
    ///
    /// Returns `None` when `batch` is empty.
    pub fn from_batch(batch: &[ChannelMessage]) -> Option<ChannelMessage> {
        let template = batch.last()?;
        let content = batch
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let media: Vec<MediaItem> = batch.iter().flat_map(|m| m.media.clone()).collect();

        let mut merged = template.clone();
        merged.content = content;
        merged.media = media;
        Some(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_message_new_sets_defaults() {
        let msg = ChannelMessage::new("session1", "telegram", "hello");
        assert_eq!(msg.session_id, "session1");
        assert_eq!(msg.channel, "telegram");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.chat_id, "default");
        assert!(!msg.is_active);
        assert_eq!(msg.kind, MessageKind::Normal);
        assert_eq!(msg.output_channel, "telegram");
        assert!(msg.media.is_empty());
    }

    #[test]
    fn test_channel_message_new_populates_context() {
        let msg = ChannelMessage::new("s1", "cli", "hi");
        assert!(msg.context.contains_key("channel"));
        assert!(msg.context.contains_key("chat_id"));
    }

    #[test]
    fn test_with_chat_id() {
        let msg = ChannelMessage::new("s1", "telegram", "hi").with_chat_id("42");
        assert_eq!(msg.chat_id, "42");
        assert_eq!(
            msg.context.get("chat_id").and_then(|v| v.as_str()),
            Some("42")
        );
    }

    #[test]
    fn test_with_is_active() {
        let msg = ChannelMessage::new("s1", "telegram", "hi").with_is_active(true);
        assert!(msg.is_active);
    }

    #[test]
    fn test_with_kind() {
        let msg = ChannelMessage::new("s1", "telegram", "hi").with_kind(MessageKind::Error);
        assert_eq!(msg.kind, MessageKind::Error);
    }

    #[test]
    fn test_with_output_channel() {
        let msg = ChannelMessage::new("s1", "telegram", "hi").with_output_channel("cli");
        assert_eq!(msg.output_channel, "cli");
    }

    #[test]
    fn test_with_context_merges() {
        let mut extra = serde_json::Map::new();
        extra.insert("source".into(), Value::String("test".into()));
        let msg = ChannelMessage::new("s1", "telegram", "hi").with_context(extra);
        assert_eq!(
            msg.context.get("source").and_then(|v| v.as_str()),
            Some("test")
        );
    }

    #[test]
    fn test_finalize_sets_output_channel_to_channel_when_empty() {
        let mut msg = ChannelMessage::new("s1", "telegram", "hi");
        msg.output_channel = String::new();
        let finalized = msg.finalize();
        assert_eq!(finalized.output_channel, "telegram");
    }

    #[test]
    fn test_context_str() {
        let msg = ChannelMessage::new("s1", "cli", "hi");
        let ctx = msg.context_str();
        // Should contain key=value pairs
        assert!(ctx.contains("channel="));
        assert!(ctx.contains("chat_id="));
    }

    #[test]
    fn test_from_batch_single() {
        let msg = ChannelMessage::new("s1", "telegram", "hello");
        let merged = ChannelMessage::from_batch(&[msg]).unwrap();
        assert_eq!(merged.content, "hello");
        assert_eq!(merged.channel, "telegram");
    }

    #[test]
    fn test_from_batch_multiple_joins_content() {
        let m1 = ChannelMessage::new("s1", "telegram", "line1");
        let m2 = ChannelMessage::new("s1", "telegram", "line2");
        let merged = ChannelMessage::from_batch(&[m1, m2]).unwrap();
        assert_eq!(merged.content, "line1\nline2");
        // Template is the last message
        assert_eq!(merged.channel, "telegram");
    }

    #[test]
    fn test_from_batch_empty_returns_none() {
        assert!(ChannelMessage::from_batch(&[]).is_none());
    }

    #[test]
    fn test_message_kind_default() {
        assert_eq!(MessageKind::default(), MessageKind::Normal);
    }

    #[test]
    fn test_message_kind_serialization() {
        let json = serde_json::to_string(&MessageKind::Error).unwrap();
        assert_eq!(json, "\"error\"");
        let deserialized: MessageKind = serde_json::from_str("\"command\"").unwrap();
        assert_eq!(deserialized, MessageKind::Command);
    }

    #[test]
    fn test_media_type_serialization() {
        let json = serde_json::to_string(&MediaType::Image).unwrap();
        assert_eq!(json, "\"image\"");
    }

    #[test]
    fn test_channel_message_json_roundtrip() {
        let msg = ChannelMessage::new("s1", "telegram", "hello")
            .with_chat_id("42")
            .with_kind(MessageKind::Command);
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ChannelMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.session_id, "s1");
        assert_eq!(deserialized.channel, "telegram");
        assert_eq!(deserialized.content, "hello");
        assert_eq!(deserialized.chat_id, "42");
        assert_eq!(deserialized.kind, MessageKind::Command);
    }
}
