//! Anthropic Messages API response format parsing.

use serde_json::Value;

use super::common::field;
use super::types::{BaseTransportParser, ToolCallDelta};

/// Parser for the Anthropic Messages response format.
pub struct MessagesTransportParser;

fn anthropic_tool_use_to_openai(block: &Value) -> Value {
    let id = block
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
    let arguments = serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_owned());

    serde_json::json!({
        "function": {"name": name, "arguments": arguments},
        "id": id,
        "type": "function",
    })
}

impl BaseTransportParser for MessagesTransportParser {
    fn is_non_stream_response(&self, response: &Value) -> bool {
        response.get("role").is_some() && response.get("content").is_some()
    }

    fn extract_chunk_tool_call_deltas(&self, chunk: &Value) -> Vec<ToolCallDelta> {
        let event_type = chunk.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            "content_block_start" => {
                if let Some(block) = chunk.get("content_block")
                    && block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let index = chunk.get("index").cloned();
                    return vec![ToolCallDelta {
                        id: Some(id),
                        index,
                        call_type: Some("function".to_owned()),
                        name,
                        arguments: String::new(),
                        arguments_complete: false,
                    }];
                }
                Vec::new()
            }
            "content_block_delta" => {
                if let Some(delta) = chunk.get("delta")
                    && delta.get("type").and_then(|t| t.as_str()) == Some("input_json_delta")
                {
                    let partial = delta
                        .get("partial_json")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let index = chunk.get("index").cloned();
                    return vec![ToolCallDelta {
                        id: None,
                        index,
                        call_type: None,
                        name: String::new(),
                        arguments: partial,
                        arguments_complete: false,
                    }];
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn extract_chunk_text(&self, chunk: &Value) -> String {
        let event_type = chunk.get("type").and_then(|t| t.as_str()).unwrap_or("");

        if event_type == "content_block_delta"
            && let Some(delta) = chunk.get("delta")
            && delta.get("type").and_then(|t| t.as_str()) == Some("text_delta")
        {
            return delta
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_owned();
        }
        String::new()
    }

    fn extract_text(&self, response: &Value) -> String {
        field(response, "content")
            .and_then(|c| c.as_array())
            .map(|content| {
                content
                    .iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default()
    }

    fn extract_tool_calls(&self, response: &Value) -> Vec<Value> {
        let content = match field(response, "content").and_then(|c| c.as_array()) {
            Some(c) => c,
            None => return Vec::new(),
        };

        content
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
            .map(anthropic_tool_use_to_openai)
            .collect()
    }

    fn extract_usage(&self, response: &Value) -> Option<Value> {
        let usage_obj = field(response, "usage")?.as_object()?;
        let mut normalized: serde_json::Map<String, Value> = ["input_tokens", "output_tokens"]
            .iter()
            .filter_map(|key| Some(((*key).to_owned(), usage_obj.get(*key)?.clone())))
            .collect();

        let total = usage_obj
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            + usage_obj
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        if total > 0 {
            normalized.insert("total_tokens".to_owned(), Value::Number(total.into()));
        }

        (!normalized.is_empty()).then_some(Value::Object(normalized))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_text() {
        let parser = MessagesTransportParser;
        let response = json!({
            "role": "assistant",
            "content": [
                {"type": "text", "text": "Hello world"}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        assert_eq!(parser.extract_text(&response), "Hello world");
    }

    #[test]
    fn test_extract_tool_calls() {
        let parser = MessagesTransportParser;
        let response = json!({
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_123",
                    "name": "get_weather",
                    "input": {"city": "London"}
                }
            ]
        });
        let calls = parser.extract_tool_calls(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "get_weather");
        assert_eq!(calls[0]["id"], "toolu_123");
    }

    #[test]
    fn test_is_non_stream_response() {
        let parser = MessagesTransportParser;
        assert!(parser.is_non_stream_response(&json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "hi"}]
        })));
        assert!(!parser.is_non_stream_response(&json!({"choices": []})));
    }

    #[test]
    fn test_extract_usage() {
        let parser = MessagesTransportParser;
        let response = json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "hi"}],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let usage = parser.extract_usage(&response).unwrap();
        assert_eq!(usage["input_tokens"], 10);
        assert_eq!(usage["output_tokens"], 5);
        assert_eq!(usage["total_tokens"], 15);
    }
}
