//! SSE stream collection and response assembly.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use tracing::info;

use super::errors::{ConduitError, ErrorKind};
use super::execution::LLMCore;
use crate::clients::parsing::TransportKind;

/// Wrapper that pairs a transport kind with the raw response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportResponse {
    pub transport: TransportKind,
    pub payload: Value,
}

/// Parse a collected SSE buffer into a single assembled JSON response.
///
/// Extracted from `collect_sse_response` so buffer-parsing logic is testable
/// without constructing a `reqwest::Response`.
pub(crate) fn parse_sse_buffer(
    buffer: &str,
    transport: TransportKind,
) -> Result<Value, ConduitError> {
    match transport {
            TransportKind::Messages => {
                // Anthropic Messages streaming: assemble from content_block_delta events.
                // Look for message_stop event and assemble content blocks.
                let mut content = String::new();
                let mut tool_use_blocks: Vec<Value> = Vec::new();
                let mut current_tool: Option<serde_json::Map<String, Value>> = None;
                let mut tool_args_buffer = String::new();
                let mut usage: Option<Value> = None;

                for line in buffer.lines() {
                    let line = line.trim();
                    if let Some(data) = line.strip_prefix("data: ")
                        && let Ok(event) = serde_json::from_str::<Value>(data)
                    {
                        let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match event_type {
                            "content_block_start" => {
                                if let Some(block) = event.get("content_block") {
                                    let block_type =
                                        block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    if block_type == "tool_use" {
                                        let mut tool = serde_json::Map::new();
                                        if let Some(id) = block.get("id") {
                                            tool.insert("id".to_owned(), id.clone());
                                        }
                                        if let Some(name) = block.get("name") {
                                            tool.insert("name".to_owned(), name.clone());
                                        }
                                        tool_args_buffer.clear();
                                        current_tool = Some(tool);
                                    }
                                }
                            }
                            "content_block_delta" => {
                                if let Some(delta) = event.get("delta") {
                                    let delta_type =
                                        delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    if delta_type == "text_delta" {
                                        if let Some(text) =
                                            delta.get("text").and_then(|t| t.as_str())
                                        {
                                            content.push_str(text);
                                        }
                                    } else if delta_type == "input_json_delta"
                                        && let Some(partial) =
                                            delta.get("partial_json").and_then(|p| p.as_str())
                                    {
                                        tool_args_buffer.push_str(partial);
                                    }
                                }
                            }
                            "content_block_stop" => {
                                if let Some(mut tool) = current_tool.take() {
                                    let input: Value = serde_json::from_str(&tool_args_buffer)
                                        .unwrap_or(serde_json::json!({}));
                                    tool.insert("input".to_owned(), input);
                                    tool.insert(
                                        "type".to_owned(),
                                        Value::String("tool_use".to_owned()),
                                    );
                                    tool_use_blocks.push(Value::Object(tool));
                                    tool_args_buffer.clear();
                                }
                            }
                            "message_delta" => {
                                if let Some(u) = event.get("usage") {
                                    usage = Some(u.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Build an Anthropic Messages response object.
                let mut content_blocks: Vec<Value> = Vec::new();
                if !content.is_empty() {
                    content_blocks.push(serde_json::json!({"type": "text", "text": content}));
                }
                content_blocks.extend(tool_use_blocks);

                let mut result = serde_json::json!({
                    "role": "assistant",
                    "content": content_blocks
                });
                if let Some(u) = usage {
                    result
                        .as_object_mut()
                        .unwrap()
                        .insert("usage".to_owned(), u);
                }
                Ok(result)
            }
            TransportKind::Responses => {
                // Look for "response.completed" event which has the full response.
                for line in buffer.lines() {
                    let line = line.trim();
                    if let Some(data) = line.strip_prefix("data: ")
                        && let Ok(event) = serde_json::from_str::<Value>(data)
                        && event.get("type").and_then(|t| t.as_str()) == Some("response.completed")
                        && let Some(response) = event.get("response")
                    {
                        return Ok(response.clone());
                    }
                }
                Err(ConduitError::new(
                    ErrorKind::Provider,
                    "SSE stream ended without response.completed event",
                ))
            }
            _ => {
                // Completion format: assemble content from delta chunks.
                // OpenAI streams tool_calls as deltas with an `index` field —
                // deltas sharing the same index must be merged (concatenate
                // `arguments`, keep `id`/`name`/`type` from the first delta).
                let mut content = String::new();
                let mut tool_call_map: BTreeMap<u64, serde_json::Map<String, Value>> =
                    BTreeMap::new();

                for line in buffer.lines() {
                    let line = line.trim();
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            break;
                        }
                        if let Ok(event) = serde_json::from_str::<Value>(data)
                            && let Some(choices) = event.get("choices").and_then(|c| c.as_array())
                        {
                            for choice in choices {
                                if let Some(delta) = choice.get("delta") {
                                    if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                                        content.push_str(c);
                                    }
                                    if let Some(tc) =
                                        delta.get("tool_calls").and_then(|t| t.as_array())
                                    {
                                        for call_delta in tc {
                                            let idx =
                                                call_delta.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                                            let entry = tool_call_map
                                                .entry(idx)
                                                .or_default();

                                            // First-seen scalars: id, type
                                            for key in ["id", "type"] {
                                                if !entry.contains_key(key)
                                                    && let Some(v) = call_delta.get(key)
                                                {
                                                    entry.insert(key.to_owned(), v.clone());
                                                }
                                            }

                                            // Merge nested `function` object (name, arguments)
                                            if let Some(fn_delta) = call_delta.get("function").and_then(|f| f.as_object()) {
                                                let fn_entry = entry
                                                    .entry("function")
                                                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                                                if let Some(fn_obj) = fn_entry.as_object_mut() {
                                                    if let Some(name) = fn_delta.get("name").and_then(|n| n.as_str()) {
                                                        fn_obj
                                                            .entry("name")
                                                            .or_insert_with(|| Value::String(name.to_owned()));
                                                    }
                                                    if let Some(args) = fn_delta.get("arguments").and_then(|a| a.as_str()) {
                                                        let existing = fn_obj
                                                            .entry("arguments")
                                                            .or_insert_with(|| Value::String(String::new()));
                                                        if let Some(s) = existing.as_str() {
                                                            let mut combined = s.to_owned();
                                                            combined.push_str(args);
                                                            *existing = Value::String(combined);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let tool_calls: Vec<Value> = tool_call_map
                    .into_values()
                    .map(Value::Object)
                    .collect();

                let mut result = serde_json::json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": content
                        }
                    }]
                });
                if !tool_calls.is_empty() {
                    result["choices"][0]["message"]["tool_calls"] = Value::Array(tool_calls);
                }
                Ok(result)
            }
        }
}

impl LLMCore {
    /// Collect an SSE streaming response into a single JSON value.
    ///
    /// For Responses format, looks for a `response.completed` event and extracts
    /// the `response` field. For Completion format, assembles content from
    /// `delta.content` chunks.
    pub(crate) async fn collect_sse_response(
        resp: reqwest::Response,
        transport: TransportKind,
    ) -> Result<Value, ConduitError> {
        use futures::StreamExt;

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                ConduitError::new(ErrorKind::Provider, format!("SSE stream error: {e}"))
            })?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
        }

        info!(
            target: "eli_trace",
            transport = ?transport,
            raw_sse = ?buffer,
            "llm.raw_sse_response"
        );

        parse_sse_buffer(&buffer, transport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_call_delta_merge_same_index() {
        // Simulate two SSE chunks for the same tool call (index 0)
        // with partial `arguments` that should be concatenated.
        let sse = "\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"lo\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"cation\\\": \\\"NYC\\\"}\"}}]}}]}\n\
data: [DONE]\n";

        let result = parse_sse_buffer(sse, TransportKind::Completion).unwrap();
        let tool_calls = result["choices"][0]["message"]["tool_calls"]
            .as_array()
            .expect("tool_calls should be an array");

        assert_eq!(tool_calls.len(), 1, "deltas with same index should merge into one tool call");

        let tc = &tool_calls[0];
        assert_eq!(tc["id"], "call_abc");
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "get_weather");
        assert_eq!(
            tc["function"]["arguments"],
            "{\"location\": \"NYC\"}",
            "arguments from two deltas should be concatenated"
        );
    }

    #[test]
    fn test_tool_call_delta_merge_multiple_indices() {
        // Two distinct tool calls at index 0 and 1, each split across deltas.
        let sse = "\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"foo\",\"arguments\":\"{\\\"a\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_2\",\"type\":\"function\",\"function\":{\"name\":\"bar\",\"arguments\":\"{\\\"b\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\": 1}\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"arguments\":\"\\\": 2}\"}}]}}]}\n\
data: [DONE]\n";

        let result = parse_sse_buffer(sse, TransportKind::Completion).unwrap();
        let tool_calls = result["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();

        assert_eq!(tool_calls.len(), 2);

        // BTreeMap ordering: index 0 first, then index 1
        assert_eq!(tool_calls[0]["function"]["name"], "foo");
        assert_eq!(tool_calls[0]["function"]["arguments"], "{\"a\": 1}");

        assert_eq!(tool_calls[1]["function"]["name"], "bar");
        assert_eq!(tool_calls[1]["function"]["arguments"], "{\"b\": 2}");
    }
}
