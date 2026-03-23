//! OpenAI chat-completions shape parsing.

use serde_json::Value;

use super::common::{expand_tool_calls, field, field_str};
use super::types::{BaseTransportParser, ToolCallDelta};

/// Parser for the OpenAI chat-completions response format.
pub struct CompletionTransportParser;

impl BaseTransportParser for CompletionTransportParser {
    fn is_non_stream_response(&self, response: &Value) -> bool {
        response.is_string() || response.get("choices").is_some()
    }

    fn extract_chunk_tool_call_deltas(&self, chunk: &Value) -> Vec<ToolCallDelta> {
        let choices = match field(chunk, "choices").and_then(|c| c.as_array()) {
            Some(c) if !c.is_empty() => c,
            _ => return Vec::new(),
        };
        let delta = match field(&choices[0], "delta") {
            Some(d) => d,
            None => return Vec::new(),
        };
        let tool_calls = match field(delta, "tool_calls").and_then(|tc| tc.as_array()) {
            Some(tc) => tc,
            None => return Vec::new(),
        };

        tool_calls
            .iter()
            .map(|tc| {
                let func = tc.get("function");
                let name = func
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_owned();
                let arguments = func
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .to_owned();
                ToolCallDelta {
                    id: tc.get("id").and_then(|v| v.as_str()).map(|s| s.to_owned()),
                    index: tc.get("index").cloned(),
                    call_type: tc
                        .get("type")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned()),
                    name,
                    arguments,
                    arguments_complete: false,
                }
            })
            .collect()
    }

    fn extract_chunk_text(&self, chunk: &Value) -> String {
        let choices = match field(chunk, "choices").and_then(|c| c.as_array()) {
            Some(c) if !c.is_empty() => c,
            _ => return String::new(),
        };
        let delta = match field(&choices[0], "delta") {
            Some(d) => d,
            None => return String::new(),
        };
        field_str(delta, "content").to_owned()
    }

    fn extract_text(&self, response: &Value) -> String {
        if let Some(s) = response.as_str() {
            return s.to_owned();
        }
        let choices = match field(response, "choices").and_then(|c| c.as_array()) {
            Some(c) if !c.is_empty() => c,
            _ => return String::new(),
        };
        let message = match field(&choices[0], "message") {
            Some(m) => m,
            None => return String::new(),
        };
        field_str(message, "content").to_owned()
    }

    fn extract_tool_calls(&self, response: &Value) -> Vec<Value> {
        let choices = match field(response, "choices").and_then(|c| c.as_array()) {
            Some(c) if !c.is_empty() => c,
            _ => return Vec::new(),
        };
        let message = match field(&choices[0], "message") {
            Some(m) => m,
            None => return Vec::new(),
        };
        let tool_calls = match field(message, "tool_calls").and_then(|tc| tc.as_array()) {
            Some(tc) => tc,
            None => return Vec::new(),
        };

        let mut calls = Vec::new();
        for tool_call in tool_calls {
            let function = match field(tool_call, "function") {
                Some(f) if f.is_object() => f,
                _ => continue,
            };
            let name = field(function, "name").cloned().unwrap_or(Value::Null);
            let arguments = field(function, "arguments").cloned().unwrap_or(Value::Null);

            let mut entry = serde_json::Map::new();
            let mut func_map = serde_json::Map::new();
            func_map.insert("name".to_owned(), name);
            func_map.insert("arguments".to_owned(), arguments);
            entry.insert("function".to_owned(), Value::Object(func_map));

            if let Some(id) = field(tool_call, "id")
                && !id.is_null()
            {
                entry.insert("id".to_owned(), id.clone());
            }
            if let Some(t) = field(tool_call, "type")
                && !t.is_null()
            {
                entry.insert("type".to_owned(), t.clone());
            }

            calls.push(Value::Object(entry));
        }

        expand_tool_calls(calls)
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
        } else if let Some(v) = usage_obj.get("prompt_tokens") {
            normalized.insert("input_tokens".to_owned(), v.clone());
        }

        if let Some(v) = usage_obj.get("output_tokens") {
            normalized.insert("output_tokens".to_owned(), v.clone());
        } else if let Some(v) = usage_obj.get("completion_tokens") {
            normalized.insert("output_tokens".to_owned(), v.clone());
        }

        if let Some(v) = usage_obj.get("total_tokens") {
            normalized.insert("total_tokens".to_owned(), v.clone());
        }

        if let Some(v) = usage_obj.get("requests") {
            normalized.insert("requests".to_owned(), v.clone());
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
        let parser = CompletionTransportParser;
        let response = json!({
            "choices": [{
                "message": {
                    "content": "Hello world"
                }
            }]
        });
        assert_eq!(parser.extract_text(&response), "Hello world");
    }

    #[test]
    fn test_extract_text_string() {
        let parser = CompletionTransportParser;
        let response = Value::String("raw text".into());
        assert_eq!(parser.extract_text(&response), "raw text");
    }

    #[test]
    fn test_extract_chunk_text() {
        let parser = CompletionTransportParser;
        let chunk = json!({
            "choices": [{
                "delta": {
                    "content": "chunk"
                }
            }]
        });
        assert_eq!(parser.extract_chunk_text(&chunk), "chunk");
    }

    #[test]
    fn test_is_non_stream_response() {
        let parser = CompletionTransportParser;
        assert!(parser.is_non_stream_response(&json!({"choices": []})));
        assert!(!parser.is_non_stream_response(&json!({"other": true})));
    }

    #[test]
    fn test_extract_usage_prompt_tokens() {
        let parser = CompletionTransportParser;
        let response = json!({
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30
            }
        });
        let usage = parser.extract_usage(&response).unwrap();
        assert_eq!(usage["input_tokens"], 10);
        assert_eq!(usage["output_tokens"], 20);
        assert_eq!(usage["total_tokens"], 30);
    }

    #[test]
    fn test_extract_tool_calls() {
        let parser = CompletionTransportParser;
        let response = json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\": \"London\"}"
                        }
                    }]
                }
            }]
        });
        let calls = parser.extract_tool_calls(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "get_weather");
    }
}
