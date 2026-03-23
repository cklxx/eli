//! Structured text helpers for Conduit.

use serde_json::Value;

use crate::clients::chat::ChatClient;
use crate::core::errors::ErrorKind;
use crate::core::results::ErrorPayload;
use crate::tools::schema::ToolSet;

/// Structured helpers built on chat tool calls.
///
/// Provides `if_` (boolean questions) and `classify` (label selection).
pub struct TextClient<'a> {
    chat: &'a mut ChatClient,
}

impl<'a> TextClient<'a> {
    /// Create a new `TextClient` wrapping a `ChatClient`.
    pub fn new(chat: &'a mut ChatClient) -> Self {
        Self { chat }
    }

    /// Build the prompt for an if/boolean question.
    fn build_if_prompt(input_text: &str, question: &str) -> String {
        format!(
            "Here is an input:\n<input>\n{}\n</input>\n\nAnd a question:\n<question>\n{}\n</question>\n\nAnswer by calling the tool with a boolean `value`.",
            input_text.trim(),
            question.trim(),
        )
    }

    /// Build the prompt for a classification question.
    fn build_classify_prompt(input_text: &str, choices_str: &str) -> String {
        format!(
            "You are given this input:\n<input>\n{}\n</input>\n\nAnd the following choices:\n<choices>\n{}\n</choices>\n\nAnswer by calling the tool with `label` set to one of the choices.",
            input_text.trim(),
            choices_str,
        )
    }

    /// Normalize and validate a list of choices.
    fn normalize_choices(choices: &[String]) -> Result<Vec<String>, ErrorPayload> {
        if choices.is_empty() {
            return Err(ErrorPayload::new(
                ErrorKind::InvalidInput,
                "choices must not be empty.",
            ));
        }
        Ok(choices.iter().map(|c| c.trim().to_owned()).collect())
    }

    /// Build the tool schema for the if_decision tool.
    fn if_decision_tool_schema() -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "if_decision",
                "description": "Return a boolean.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "value": {
                            "type": "boolean",
                            "description": "The boolean answer."
                        }
                    },
                    "required": ["value"]
                }
            }
        })
    }

    /// Build the tool schema for the classify_decision tool.
    fn classify_decision_tool_schema() -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "classify_decision",
                "description": "Return one label.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "label": {
                            "type": "string",
                            "description": "One of the provided choices."
                        }
                    },
                    "required": ["label"]
                }
            }
        })
    }

    /// Parse the first tool call result, extracting a specific field.
    fn parse_tool_call_field(calls: &[Value], field_name: &str) -> Result<Value, ErrorPayload> {
        if calls.is_empty() {
            return Err(ErrorPayload::new(
                ErrorKind::InvalidInput,
                "tool call is missing.",
            ));
        }
        let call = &calls[0];
        let args = call
            .get("function")
            .and_then(|f| f.get("arguments"))
            .ok_or_else(|| ErrorPayload::new(ErrorKind::InvalidInput, "tool call is missing."))?;

        let args_obj = if let Some(s) = args.as_str() {
            serde_json::from_str::<Value>(s).map_err(|e| {
                ErrorPayload::new(
                    ErrorKind::InvalidInput,
                    "tool arguments are not valid JSON.",
                )
                .with_details(serde_json::json!({"error": e.to_string()}))
            })?
        } else {
            args.clone()
        };

        if !args_obj.is_object() {
            return Err(ErrorPayload::new(
                ErrorKind::InvalidInput,
                "tool arguments must be an object.",
            ));
        }

        args_obj.get(field_name).cloned().ok_or_else(|| {
            ErrorPayload::new(
                ErrorKind::InvalidInput,
                format!("tool arguments missing field '{}'.", field_name),
            )
        })
    }

    /// Ask a yes/no question about the given input text.
    pub async fn if_(
        &mut self,
        input_text: &str,
        question: &str,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> Result<bool, ErrorPayload> {
        let prompt = Self::build_if_prompt(input_text, question);
        let tool_schema = Self::if_decision_tool_schema();
        let toolset = ToolSet {
            schemas: vec![tool_schema],
            runnable: Vec::new(),
        };

        let calls = self
            .chat
            .tool_calls(
                Some(&prompt),
                None,
                None,
                model,
                provider,
                None,
                &toolset,
                serde_json::Map::new(),
            )
            .await?;

        let value = Self::parse_tool_call_field(&calls, "value")?;
        value.as_bool().ok_or_else(|| {
            ErrorPayload::new(
                ErrorKind::InvalidInput,
                "tool call 'value' field is not a boolean.",
            )
        })
    }

    /// Classify the input text into one of the given choices.
    pub async fn classify(
        &mut self,
        input_text: &str,
        choices: &[String],
        model: Option<&str>,
        provider: Option<&str>,
    ) -> Result<String, ErrorPayload> {
        let normalized = Self::normalize_choices(choices)?;
        let choices_str = normalized.join(", ");
        let prompt = Self::build_classify_prompt(input_text, &choices_str);
        let tool_schema = Self::classify_decision_tool_schema();
        let toolset = ToolSet {
            schemas: vec![tool_schema],
            runnable: Vec::new(),
        };

        let calls = self
            .chat
            .tool_calls(
                Some(&prompt),
                None,
                None,
                model,
                provider,
                None,
                &toolset,
                serde_json::Map::new(),
            )
            .await?;

        let value = Self::parse_tool_call_field(&calls, "label")?;
        let label = value.as_str().ok_or_else(|| {
            ErrorPayload::new(
                ErrorKind::InvalidInput,
                "tool call 'label' field is not a string.",
            )
        })?;

        if !normalized.contains(&label.to_owned()) {
            return Err(ErrorPayload::new(
                ErrorKind::InvalidInput,
                "classification label is not in the allowed choices.",
            )
            .with_details(serde_json::json!({
                "label": label,
                "choices": normalized,
            })));
        }

        Ok(label.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_if_prompt() {
        let prompt = TextClient::build_if_prompt("some text", "is it good?");
        assert!(prompt.contains("<input>"));
        assert!(prompt.contains("some text"));
        assert!(prompt.contains("is it good?"));
    }

    #[test]
    fn test_build_classify_prompt() {
        let prompt = TextClient::build_classify_prompt("some text", "A, B, C");
        assert!(prompt.contains("<choices>"));
        assert!(prompt.contains("A, B, C"));
    }

    #[test]
    fn test_normalize_choices_empty() {
        let result = TextClient::normalize_choices(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_choices_trims() {
        let result = TextClient::normalize_choices(&["  A ".to_owned(), " B".to_owned()]).unwrap();
        assert_eq!(result, vec!["A", "B"]);
    }

    #[test]
    fn test_parse_tool_call_field_ok() {
        let calls = vec![serde_json::json!({
            "function": {
                "name": "if_decision",
                "arguments": "{\"value\": true}"
            }
        })];
        let val = TextClient::parse_tool_call_field(&calls, "value").unwrap();
        assert_eq!(val, serde_json::json!(true));
    }

    #[test]
    fn test_parse_tool_call_field_empty() {
        let result = TextClient::parse_tool_call_field(&[], "value");
        assert!(result.is_err());
    }

    #[test]
    fn test_if_decision_tool_schema() {
        let schema = TextClient::if_decision_tool_schema();
        assert_eq!(schema["function"]["name"], "if_decision");
    }

    #[test]
    fn test_classify_decision_tool_schema() {
        let schema = TextClient::classify_decision_tool_schema();
        assert_eq!(schema["function"]["name"], "classify_decision");
    }
}
