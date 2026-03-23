//! Framework-neutral data aliases and core types.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

/// An envelope is any JSON value representing a message flowing through the framework.
pub type Envelope = Value;

/// Session state is a string-keyed map of arbitrary JSON values.
pub type State = HashMap<String, Value>;

/// An async message handler that receives inbound envelopes.
pub type MessageHandler =
    Arc<dyn Fn(Envelope) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// An async outbound dispatcher that sends a single envelope and reports success.
pub type OutboundDispatcher =
    Arc<dyn Fn(Envelope) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// Router for dispatching outbound messages to the correct channel.
#[async_trait]
pub trait OutboundChannelRouter: Send + Sync {
    /// Dispatch one outbound envelope. Returns `true` if delivered.
    async fn dispatch(&self, message: Envelope) -> bool;

    /// Signal a session to quit.
    async fn quit(&self, session_id: &str);
}

/// Result of one complete message turn through the framework.
#[derive(Debug, Clone)]
pub struct TurnResult {
    pub session_id: String,
    pub prompt: PromptValue,
    pub model_output: String,
    pub outbounds: Vec<Envelope>,
}

/// A prompt can be plain text or a list of multimodal content parts.
#[derive(Debug, Clone)]
pub enum PromptValue {
    Text(String),
    Parts(Vec<Value>),
}

impl PromptValue {
    /// Return a plain-text representation regardless of variant.
    pub fn as_text(&self) -> String {
        match self {
            PromptValue::Text(s) => s.clone(),
            PromptValue::Parts(parts) => {
                let mut texts = Vec::new();
                for part in parts {
                    if let Some(text) = part.as_str() {
                        texts.push(text.to_string());
                    } else if let Some(obj) = part.as_object()
                        && let Some(text) = obj.get("text").and_then(|v| v.as_str())
                    {
                        texts.push(text.to_string());
                    }
                }
                texts.join("\n")
            }
        }
    }

    /// Return `true` if the prompt is empty or blank.
    pub fn is_empty(&self) -> bool {
        match self {
            PromptValue::Text(s) => s.is_empty(),
            PromptValue::Parts(parts) => parts.is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- TurnResult tests -----------------------------------------------------

    #[test]
    fn test_turn_result_creation() {
        let result = TurnResult {
            session_id: "cli:default".into(),
            prompt: PromptValue::Text("hello".into()),
            model_output: "world".into(),
            outbounds: vec![json!({"content": "world"})],
        };
        assert_eq!(result.session_id, "cli:default");
        assert_eq!(result.model_output, "world");
        assert_eq!(result.outbounds.len(), 1);
    }

    #[test]
    fn test_turn_result_clone() {
        let result = TurnResult {
            session_id: "s1".into(),
            prompt: PromptValue::Text("p".into()),
            model_output: "o".into(),
            outbounds: vec![],
        };
        let cloned = result.clone();
        assert_eq!(cloned.session_id, "s1");
        assert_eq!(cloned.model_output, "o");
    }

    // -- PromptValue tests ----------------------------------------------------

    #[test]
    fn test_prompt_value_text_as_text() {
        let pv = PromptValue::Text("hello world".into());
        assert_eq!(pv.as_text(), "hello world");
    }

    #[test]
    fn test_prompt_value_parts_as_text_with_strings() {
        let pv = PromptValue::Parts(vec![json!("line1"), json!("line2")]);
        assert_eq!(pv.as_text(), "line1\nline2");
    }

    #[test]
    fn test_prompt_value_parts_as_text_with_text_objects() {
        let pv = PromptValue::Parts(vec![json!({"text": "part1"}), json!({"text": "part2"})]);
        assert_eq!(pv.as_text(), "part1\npart2");
    }

    #[test]
    fn test_prompt_value_parts_as_text_mixed() {
        let pv = PromptValue::Parts(vec![
            json!("plain"),
            json!({"text": "object"}),
            json!({"image": "data"}), // no text field, should be skipped
        ]);
        assert_eq!(pv.as_text(), "plain\nobject");
    }

    #[test]
    fn test_prompt_value_is_empty_text() {
        assert!(PromptValue::Text("".into()).is_empty());
        assert!(!PromptValue::Text("hello".into()).is_empty());
    }

    #[test]
    fn test_prompt_value_is_empty_parts() {
        assert!(PromptValue::Parts(vec![]).is_empty());
        assert!(!PromptValue::Parts(vec![json!("a")]).is_empty());
    }
}
