//! Structured results and errors for Conduit.

use std::pin::Pin;

use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::errors::ErrorKind;

// ---------------------------------------------------------------------------
// ErrorPayload
// ---------------------------------------------------------------------------

/// A serializable error payload carried inside streams and results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub kind: ErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl std::fmt::Display for ErrorPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.kind, self.message)
    }
}

impl std::error::Error for ErrorPayload {}

impl ErrorPayload {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Serialize to a JSON-like map, matching the Python `as_dict` method.
    pub fn as_map(&self) -> serde_json::Map<String, Value> {
        let mut map = serde_json::Map::new();
        map.insert(
            "kind".to_owned(),
            Value::String(self.kind.as_str().to_owned()),
        );
        map.insert("message".to_owned(), Value::String(self.message.clone()));
        if let Some(ref details) = self.details {
            map.insert("details".to_owned(), details.clone());
        }
        map
    }
}

// ---------------------------------------------------------------------------
// StreamState
// ---------------------------------------------------------------------------

/// Mutable state that accompanies a stream, populated after the stream ends.
#[derive(Debug, Clone, Default)]
pub struct StreamState {
    pub error: Option<ErrorPayload>,
    pub usage: Option<Value>,
}

impl StreamState {
    pub fn new() -> Self {
        Self::default()
    }
}

// ---------------------------------------------------------------------------
// TextStream  (sync)
// ---------------------------------------------------------------------------

/// A synchronous stream of text chunks, backed by any `Iterator<Item = String>`.
pub struct TextStream {
    iterator: Box<dyn Iterator<Item = String> + Send>,
    state: StreamState,
}

impl TextStream {
    pub fn new(
        iterator: impl Iterator<Item = String> + Send + 'static,
        state: Option<StreamState>,
    ) -> Self {
        Self {
            iterator: Box::new(iterator),
            state: state.unwrap_or_default(),
        }
    }

    pub fn error(&self) -> Option<&ErrorPayload> {
        self.state.error.as_ref()
    }

    pub fn usage(&self) -> Option<&Value> {
        self.state.usage.as_ref()
    }

    /// Obtain a mutable reference to the underlying state so that the
    /// producer can set `error` or `usage` after iteration finishes.
    pub fn state_mut(&mut self) -> &mut StreamState {
        &mut self.state
    }
}

impl Iterator for TextStream {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        self.iterator.next()
    }
}

// ---------------------------------------------------------------------------
// AsyncTextStream
// ---------------------------------------------------------------------------

/// An asynchronous stream of text chunks.
pub struct AsyncTextStream {
    stream: Pin<Box<dyn Stream<Item = String> + Send>>,
    state: StreamState,
}

impl AsyncTextStream {
    pub fn new(
        stream: impl Stream<Item = String> + Send + 'static,
        state: Option<StreamState>,
    ) -> Self {
        Self {
            stream: Box::pin(stream),
            state: state.unwrap_or_default(),
        }
    }

    pub fn error(&self) -> Option<&ErrorPayload> {
        self.state.error.as_ref()
    }

    pub fn usage(&self) -> Option<&Value> {
        self.state.usage.as_ref()
    }

    pub fn state_mut(&mut self) -> &mut StreamState {
        &mut self.state
    }

    /// Consume self and return the inner `Stream`.
    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = String> + Send>> {
        self.stream
    }
}

// ---------------------------------------------------------------------------
// StreamEvent
// ---------------------------------------------------------------------------

/// The kind tag for a `StreamEvent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamEventKind {
    Text,
    ToolCall,
    ToolResult,
    Usage,
    Error,
    Final,
}

/// A single event produced by a structured event stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    pub kind: StreamEventKind,
    pub data: Value,
}

impl StreamEvent {
    pub fn new(kind: StreamEventKind, data: Value) -> Self {
        Self { kind, data }
    }
}

// ---------------------------------------------------------------------------
// StreamEvents  (sync)
// ---------------------------------------------------------------------------

/// A synchronous iterator of `StreamEvent` values.
pub struct StreamEvents {
    iterator: Box<dyn Iterator<Item = StreamEvent> + Send>,
    state: StreamState,
}

impl StreamEvents {
    pub fn new(
        iterator: impl Iterator<Item = StreamEvent> + Send + 'static,
        state: Option<StreamState>,
    ) -> Self {
        Self {
            iterator: Box::new(iterator),
            state: state.unwrap_or_default(),
        }
    }

    pub fn error(&self) -> Option<&ErrorPayload> {
        self.state.error.as_ref()
    }

    pub fn usage(&self) -> Option<&Value> {
        self.state.usage.as_ref()
    }

    pub fn state_mut(&mut self) -> &mut StreamState {
        &mut self.state
    }
}

impl Iterator for StreamEvents {
    type Item = StreamEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.iterator.next()
    }
}

// ---------------------------------------------------------------------------
// AsyncStreamEvents
// ---------------------------------------------------------------------------

/// An asynchronous stream of `StreamEvent` values.
pub struct AsyncStreamEvents {
    stream: Pin<Box<dyn Stream<Item = StreamEvent> + Send>>,
    state: StreamState,
}

impl AsyncStreamEvents {
    pub fn new(
        stream: impl Stream<Item = StreamEvent> + Send + 'static,
        state: Option<StreamState>,
    ) -> Self {
        Self {
            stream: Box::pin(stream),
            state: state.unwrap_or_default(),
        }
    }

    pub fn error(&self) -> Option<&ErrorPayload> {
        self.state.error.as_ref()
    }

    pub fn usage(&self) -> Option<&Value> {
        self.state.usage.as_ref()
    }

    pub fn state_mut(&mut self) -> &mut StreamState {
        &mut self.state
    }

    /// Consume self and return the inner `Stream`.
    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send>> {
        self.stream
    }
}

// ---------------------------------------------------------------------------
// ToolExecution
// ---------------------------------------------------------------------------

/// The result of executing tool calls in a single round.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolExecution {
    #[serde(default)]
    pub tool_calls: Vec<Value>,
    #[serde(default)]
    pub tool_results: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
}

// ---------------------------------------------------------------------------
// ToolAutoResult
// ---------------------------------------------------------------------------

/// The kind tag for a `ToolAutoResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolAutoResultKind {
    Text,
    Tools,
    Error,
}

/// Final result of an automatic tool-execution loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolAutoResult {
    pub kind: ToolAutoResultKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub tool_calls: Vec<Value>,
    pub tool_results: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
}

impl ToolAutoResult {
    /// Construct a text-only result.
    pub fn text_result(text: impl Into<String>) -> Self {
        Self {
            kind: ToolAutoResultKind::Text,
            text: Some(text.into()),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            error: None,
        }
    }

    /// Construct a tools result (successful tool-call round).
    pub fn tools_result(tool_calls: Vec<Value>, tool_results: Vec<Value>) -> Self {
        Self {
            kind: ToolAutoResultKind::Tools,
            text: None,
            tool_calls,
            tool_results,
            error: None,
        }
    }

    /// Construct an error result, optionally carrying partial tool data.
    pub fn error_result(
        error: ErrorPayload,
        tool_calls: Option<Vec<Value>>,
        tool_results: Option<Vec<Value>>,
    ) -> Self {
        Self {
            kind: ToolAutoResultKind::Error,
            text: None,
            tool_calls: tool_calls.unwrap_or_default(),
            tool_results: tool_results.unwrap_or_default(),
            error: Some(error),
        }
    }
}
