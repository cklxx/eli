//! Context payload for tool execution.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Context passed to tools that opt into receiving execution metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContext {
    /// The tape name associated with this execution, if any.
    pub tape: Option<String>,
    /// Unique identifier for this execution run.
    pub run_id: String,
    /// Arbitrary metadata provided by the caller.
    pub meta: HashMap<String, Value>,
    /// Mutable state that tools can read and write across calls within a run.
    pub state: HashMap<String, Value>,
}

impl ToolContext {
    /// Create a new `ToolContext` with the given run ID and optional tape name.
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            tape: None,
            run_id: run_id.into(),
            meta: HashMap::new(),
            state: HashMap::new(),
        }
    }

    /// Set the tape name.
    pub fn with_tape(mut self, tape: impl Into<String>) -> Self {
        self.tape = Some(tape.into());
        self
    }

    /// Insert a metadata entry.
    pub fn with_meta(mut self, key: impl Into<String>, value: Value) -> Self {
        self.meta.insert(key.into(), value);
        self
    }

    /// Insert a state entry.
    pub fn with_state(mut self, key: impl Into<String>, value: Value) -> Self {
        self.state.insert(key.into(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tool_context_new() {
        let ctx = ToolContext::new("run-1");
        assert_eq!(ctx.run_id, "run-1");
        assert!(ctx.tape.is_none());
        assert!(ctx.meta.is_empty());
        assert!(ctx.state.is_empty());
    }

    #[test]
    fn test_tool_context_with_tape() {
        let ctx = ToolContext::new("run-1").with_tape("my-tape");
        assert_eq!(ctx.tape.as_deref(), Some("my-tape"));
    }

    #[test]
    fn test_tool_context_with_meta() {
        let ctx = ToolContext::new("run-1")
            .with_meta("key1", json!("value1"))
            .with_meta("key2", json!(42));
        assert_eq!(ctx.meta.get("key1"), Some(&json!("value1")));
        assert_eq!(ctx.meta.get("key2"), Some(&json!(42)));
    }

    #[test]
    fn test_tool_context_with_state() {
        let ctx = ToolContext::new("run-1").with_state("workspace", json!("/tmp/test"));
        assert_eq!(ctx.state.get("workspace"), Some(&json!("/tmp/test")));
    }

    #[test]
    fn test_tool_context_builder_chaining() {
        let ctx = ToolContext::new("run-x")
            .with_tape("tape-y")
            .with_meta("m", json!(true))
            .with_state("s", json!("val"));

        assert_eq!(ctx.run_id, "run-x");
        assert_eq!(ctx.tape.as_deref(), Some("tape-y"));
        assert_eq!(ctx.meta.len(), 1);
        assert_eq!(ctx.state.len(), 1);
    }

    #[test]
    fn test_tool_context_serialization_round_trip() {
        let ctx = ToolContext::new("run-1")
            .with_tape("tape-1")
            .with_meta("env", json!("prod"))
            .with_state("counter", json!(0));

        let serialized = serde_json::to_string(&ctx).unwrap();
        let deserialized: ToolContext = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.run_id, "run-1");
        assert_eq!(deserialized.tape.as_deref(), Some("tape-1"));
        assert_eq!(deserialized.meta.get("env"), Some(&json!("prod")));
        assert_eq!(deserialized.state.get("counter"), Some(&json!(0)));
    }

    #[test]
    fn test_tool_context_serialization_without_tape() {
        let ctx = ToolContext::new("r");
        let serialized = serde_json::to_string(&ctx).unwrap();
        let deserialized: ToolContext = serde_json::from_str(&serialized).unwrap();
        assert!(deserialized.tape.is_none());
    }

    #[test]
    fn test_tool_context_clone() {
        let ctx = ToolContext::new("run-1")
            .with_tape("t")
            .with_state("k", json!("v"));
        let cloned = ctx.clone();
        assert_eq!(cloned.run_id, ctx.run_id);
        assert_eq!(cloned.tape, ctx.tape);
        assert_eq!(cloned.state, ctx.state);
    }

    #[test]
    fn test_tool_context_debug() {
        let ctx = ToolContext::new("run-1");
        let debug_str = format!("{:?}", ctx);
        assert!(debug_str.contains("run-1"));
    }
}
