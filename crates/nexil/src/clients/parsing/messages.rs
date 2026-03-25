//! Anthropic Messages API response format parsing.

use serde_json::Value;

use super::common::field;
use super::types::{BaseTransportParser, ToolCallDelta};

/// Parser for the Anthropic Messages response format.
pub struct MessagesTransportParser;

impl BaseTransportParser for MessagesTransportParser {
    fn is_non_stream_response(&self, response: &Value) -> bool {
        // Anthropic Messages responses have "role": "assistant" and "content" array at top level.
        response.get("role").is_some() && response.get("content").is_some()
    }

    fn extract_chunk_tool_call_deltas(&self, chunk: &Value) -> Vec<ToolCallDelta> {
        // Anthropic streaming uses content_block_start / content_block_delta events.
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
        // Anthropic Messages format: { "content": [{"type": "text", "text": "..."}] }
        if let Some(content) = field(response, "content").and_then(|c| c.as_array()) {
            let mut parts: Vec<String> = Vec::new();
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) == Some("text")
                    && let Some(text) = block.get("text").and_then(|t| t.as_str())
                {
                    parts.push(text.to_owned());
                }
            }
            return parts.join("");
        }
        String::new()
    }

    fn extract_tool_calls(&self, response: &Value) -> Vec<Value> {
        // Anthropic Messages format: { "content": [{"type": "tool_use", "id": "...", "name": "...", "input": {...}}] }
        let content = match field(response, "content").and_then(|c| c.as_array()) {
            Some(c) => c,
            None => return Vec::new(),
        };

        let mut calls = Vec::new();
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                let id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                let arguments = serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());

                // Convert to OpenAI-compatible tool_call format.
                let mut entry = serde_json::Map::new();
                let mut func_map = serde_json::Map::new();
                func_map.insert("name".to_owned(), Value::String(name.to_owned()));
                func_map.insert("arguments".to_owned(), Value::String(arguments));
                entry.insert("function".to_owned(), Value::Object(func_map));
                entry.insert("id".to_owned(), Value::String(id.to_owned()));
                entry.insert("type".to_owned(), Value::String("function".to_owned()));

                calls.push(Value::Object(entry));
            }
        }

        calls
    }

    fn extract_usage(&self, response: &Value) -> Option<Value> {
        let usage = field(response, "usage")?;
        if !usage.is_object() {
            return None;
        }
        let usage_obj = usage.as_object()?;
        let mut normalized = serde_json::Map::new();

        if let Some(v) = usage_obj.get("input_tokens") {
            normalized.insert("input_tokens".to_owned(), v.clone());
        }
        if let Some(v) = usage_obj.get("output_tokens") {
            normalized.insert("output_tokens".to_owned(), v.clone());
        }
        // Compute total if not present.
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

        if normalized.is_empty() {
            None
        } else {
            Some(Value::Object(normalized))
        }
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
