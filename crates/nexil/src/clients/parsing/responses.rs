//! OpenAI responses shape parsing.

use serde_json::Value;

use super::common::{expand_tool_calls, field, field_str};
use super::types::{BaseTransportParser, ToolCallDelta};

/// Parser for the OpenAI responses API format.
pub struct ResponseTransportParser;

fn responses_function_call_to_openai(item: &Value) -> Option<Value> {
    let name = field(item, "name")
        .and_then(|n| n.as_str())
        .filter(|n| !n.is_empty())?;
    let arguments = field(item, "arguments")
        .and_then(|a| a.as_str())
        .unwrap_or("")
        .to_owned();

    let call_id = field(item, "call_id")
        .and_then(|v| v.as_str())
        .or_else(|| field(item, "id").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty());

    let mut entry = serde_json::Map::new();
    entry.insert(
        "function".to_owned(),
        serde_json::json!({"name": name, "arguments": arguments}),
    );
    if let Some(cid) = call_id {
        entry.insert("id".to_owned(), Value::String(cid.to_owned()));
    }
    entry.insert("type".to_owned(), Value::String("function".to_owned()));
    Some(Value::Object(entry))
}

impl ResponseTransportParser {
    fn tool_delta_from_args_event(&self, chunk: &Value, event_type: &str) -> Vec<ToolCallDelta> {
        let item_id = match field(chunk, "item_id").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id.to_owned(),
            _ => return Vec::new(),
        };

        let arguments = if event_type == "response.function_call_arguments.done" {
            field(chunk, "arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned()
        } else {
            match field(chunk, "delta").and_then(|v| v.as_str()) {
                Some(s) => s.to_owned(),
                None => return Vec::new(),
            }
        };

        let call_id = field(chunk, "call_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned());
        let name = field_str(chunk, "name").to_owned();

        vec![ToolCallDelta {
            id: call_id,
            index: Some(Value::String(item_id)),
            call_type: Some("function".to_owned()),
            name,
            arguments,
            arguments_complete: event_type == "response.function_call_arguments.done",
        }]
    }

    fn tool_delta_from_output_item_event(
        &self,
        chunk: &Value,
        event_type: &str,
    ) -> Vec<ToolCallDelta> {
        let item = match field(chunk, "item") {
            Some(i) => i,
            None => return Vec::new(),
        };
        if field_str(item, "type") != "function_call" {
            return Vec::new();
        }

        let item_id = field(item, "id").and_then(|v| v.as_str()).unwrap_or("");
        let call_id_raw = field(item, "call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let call_id = if call_id_raw.is_empty() {
            item_id
        } else {
            call_id_raw
        };
        if call_id.is_empty() {
            return Vec::new();
        }

        let arguments = field(item, "arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let name = field_str(item, "name").to_owned();
        let index_val = if item_id.is_empty() {
            Value::String(call_id.to_owned())
        } else {
            Value::String(item_id.to_owned())
        };

        vec![ToolCallDelta {
            id: Some(call_id.to_owned()),
            index: Some(index_val),
            call_type: Some("function".to_owned()),
            name,
            arguments,
            arguments_complete: event_type == "response.output_item.done",
        }]
    }

    fn extract_text_from_output(&self, output: &Value) -> String {
        let items = match output.as_array() {
            Some(a) => a,
            None => return String::new(),
        };
        items
            .iter()
            .filter(|item| field_str(item, "type") == "message")
            .filter_map(|item| field(item, "content").and_then(|c| c.as_array()))
            .flatten()
            .filter(|entry| field_str(entry, "type") == "output_text")
            .filter_map(|entry| field(entry, "text").and_then(|t| t.as_str()))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("")
    }
}

impl BaseTransportParser for ResponseTransportParser {
    fn is_non_stream_response(&self, response: &Value) -> bool {
        response.is_string()
            || response.get("choices").is_some()
            || response.get("output").is_some()
            || response.get("output_text").is_some()
    }

    fn extract_chunk_tool_call_deltas(&self, chunk: &Value) -> Vec<ToolCallDelta> {
        let event_type = field_str(chunk, "type");
        match event_type {
            "response.function_call_arguments.delta" | "response.function_call_arguments.done" => {
                self.tool_delta_from_args_event(chunk, event_type)
            }
            "response.output_item.added" | "response.output_item.done" => {
                self.tool_delta_from_output_item_event(chunk, event_type)
            }
            _ => Vec::new(),
        }
    }

    fn extract_chunk_text(&self, chunk: &Value) -> String {
        if field_str(chunk, "type") != "response.output_text.delta" {
            return String::new();
        }
        match field(chunk, "delta").and_then(|d| d.as_str()) {
            Some(s) => s.to_owned(),
            None => String::new(),
        }
    }

    fn extract_text(&self, response: &Value) -> String {
        if let Some(text) = field(response, "output_text").and_then(|t| t.as_str()) {
            return text.to_owned();
        }
        field(response, "output")
            .map(|o| self.extract_text_from_output(o))
            .unwrap_or_default()
    }

    fn extract_tool_calls(&self, response: &Value) -> Vec<Value> {
        let output = if response.is_array() {
            response
        } else {
            match field(response, "output") {
                Some(o) => o,
                None => return Vec::new(),
            }
        };
        let items = match output.as_array() {
            Some(a) => a,
            None => return Vec::new(),
        };

        let calls = items
            .iter()
            .filter(|item| field_str(item, "type") == "function_call")
            .filter_map(responses_function_call_to_openai)
            .collect();

        expand_tool_calls(calls)
    }

    fn extract_usage(&self, response: &Value) -> Option<Value> {
        let event_type = field(response, "type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let usage = match event_type {
            "response.completed"
            | "response.in_progress"
            | "response.failed"
            | "response.incomplete" => {
                let inner_response = field(response, "response")?;
                field(inner_response, "usage")?
            }
            _ => field(response, "usage")?,
        };

        if usage.is_object() {
            Some(usage.clone())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_text_output_text() {
        let parser = ResponseTransportParser;
        let response = json!({"output_text": "Hello"});
        assert_eq!(parser.extract_text(&response), "Hello");
    }

    #[test]
    fn test_extract_text_from_output_items() {
        let parser = ResponseTransportParser;
        let response = json!({
            "output": [{
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "world"
                }]
            }]
        });
        assert_eq!(parser.extract_text(&response), "world");
    }

    #[test]
    fn test_extract_chunk_text_delta() {
        let parser = ResponseTransportParser;
        let chunk = json!({
            "type": "response.output_text.delta",
            "delta": "hello"
        });
        assert_eq!(parser.extract_chunk_text(&chunk), "hello");
    }

    #[test]
    fn test_extract_chunk_text_wrong_type() {
        let parser = ResponseTransportParser;
        let chunk = json!({"type": "response.completed"});
        assert_eq!(parser.extract_chunk_text(&chunk), "");
    }

    #[test]
    fn test_is_non_stream_response() {
        let parser = ResponseTransportParser;
        assert!(parser.is_non_stream_response(&json!({"output": []})));
        assert!(parser.is_non_stream_response(&json!({"output_text": "hi"})));
        assert!(!parser.is_non_stream_response(&json!({"type": "chunk"})));
    }

    #[test]
    fn test_extract_tool_calls() {
        let parser = ResponseTransportParser;
        let response = json!({
            "output": [{
                "type": "function_call",
                "name": "greet",
                "arguments": "{\"name\":\"Bob\"}",
                "call_id": "call_abc"
            }]
        });
        let calls = parser.extract_tool_calls(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "greet");
        assert_eq!(calls[0]["id"], "call_abc");
    }

    #[test]
    fn test_extract_usage_completed() {
        let parser = ResponseTransportParser;
        let response = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 5,
                    "output_tokens": 10
                }
            }
        });
        let usage = parser.extract_usage(&response).unwrap();
        assert_eq!(usage["input_tokens"], 5);
    }

    #[test]
    fn test_extract_chunk_tool_call_deltas_args_delta() {
        let parser = ResponseTransportParser;
        let chunk = json!({
            "type": "response.function_call_arguments.delta",
            "item_id": "item_1",
            "delta": "{\"x\":",
            "call_id": "call_1",
            "name": "test_fn"
        });
        let deltas = parser.extract_chunk_tool_call_deltas(&chunk);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].arguments, "{\"x\":");
        assert!(!deltas[0].arguments_complete);
    }

    #[test]
    fn test_extract_chunk_tool_call_deltas_args_done() {
        let parser = ResponseTransportParser;
        let chunk = json!({
            "type": "response.function_call_arguments.done",
            "item_id": "item_1",
            "arguments": "{\"x\": 1}",
            "call_id": "call_1",
            "name": "test_fn"
        });
        let deltas = parser.extract_chunk_tool_call_deltas(&chunk);
        assert_eq!(deltas.len(), 1);
        assert!(deltas[0].arguments_complete);
        assert_eq!(deltas[0].arguments, "{\"x\": 1}");
    }

    #[test]
    fn test_extract_chunk_tool_call_deltas_output_item() {
        let parser = ResponseTransportParser;
        let chunk = json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "id": "item_1",
                "call_id": "call_1",
                "name": "test_fn",
                "arguments": "{\"x\": 1}"
            }
        });
        let deltas = parser.extract_chunk_tool_call_deltas(&chunk);
        assert_eq!(deltas.len(), 1);
        assert!(deltas[0].arguments_complete);
    }
}
