//! Tape entries for Conduit.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::core::results::ErrorPayload;
use crate::core::tool_calls::{normalize_message_tool_calls, normalize_tool_calls};

/// Return the current UTC time as an ISO-8601 string.
pub fn utc_now() -> String {
    Utc::now().to_rfc3339()
}

/// A single append-only entry in a tape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapeEntry {
    pub id: i64,
    pub kind: String,
    pub payload: Value,
    pub meta: Value,
    pub date: String,
}

impl TapeEntry {
    /// Create a new TapeEntry with the given fields.
    pub fn new(id: i64, kind: String, payload: Value, meta: Value, date: String) -> Self {
        Self {
            id,
            kind,
            payload,
            meta,
            date,
        }
    }

    /// Deep-copy this entry (clone payload and meta).
    pub fn copy(&self) -> Self {
        Self {
            id: self.id,
            kind: self.kind.clone(),
            payload: self.payload.clone(),
            meta: self.meta.clone(),
            date: self.date.clone(),
        }
    }

    /// Create a message entry.
    pub fn message(message: Value, meta: Value) -> Self {
        Self {
            id: 0,
            kind: "message".into(),
            payload: normalize_message_tool_calls(&message),
            meta,
            date: utc_now(),
        }
    }

    /// Create a system entry.
    pub fn system(content: &str, meta: Value) -> Self {
        let payload = serde_json::json!({ "content": content });
        Self {
            id: 0,
            kind: "system".into(),
            payload,
            meta,
            date: utc_now(),
        }
    }

    /// Create an anchor entry.
    pub fn anchor(name: &str, state: Option<Value>, meta: Value) -> Self {
        let mut map = Map::new();
        map.insert("name".into(), Value::String(name.into()));
        if let Some(s) = state {
            map.insert("state".into(), s);
        }
        Self {
            id: 0,
            kind: "anchor".into(),
            payload: Value::Object(map),
            meta,
            date: utc_now(),
        }
    }

    /// Create a tool_call entry.
    pub fn tool_call(calls: Vec<Value>, meta: Value) -> Self {
        let payload = serde_json::json!({ "calls": normalize_tool_calls(&calls) });
        Self {
            id: 0,
            kind: "tool_call".into(),
            payload,
            meta,
            date: utc_now(),
        }
    }

    /// Create a tool_result entry.
    pub fn tool_result(results: Vec<Value>, meta: Value) -> Self {
        let payload = serde_json::json!({ "results": results });
        Self {
            id: 0,
            kind: "tool_result".into(),
            payload,
            meta,
            date: utc_now(),
        }
    }

    /// Create an error entry from an ErrorPayload (matching Python's `ErrorPayload.as_dict()`).
    pub fn error(error: &ErrorPayload, meta: Value) -> Self {
        let payload = Value::Object(error.as_map());
        Self {
            id: 0,
            kind: "error".into(),
            payload,
            meta,
            date: utc_now(),
        }
    }

    /// Create an event entry.
    pub fn event(name: &str, data: Option<Value>, meta: Value) -> Self {
        let mut map = Map::new();
        map.insert("name".into(), Value::String(name.into()));
        if let Some(d) = data {
            map.insert("data".into(), d);
        }
        Self {
            id: 0,
            kind: "event".into(),
            payload: Value::Object(map),
            meta,
            date: utc_now(),
        }
    }
}
