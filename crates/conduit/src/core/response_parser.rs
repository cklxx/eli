//! SSE stream collection and response assembly.

use serde::{Deserialize, Serialize};
use serde_json::Value;
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

        // Parse SSE events from the buffer.
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
                let mut content = String::new();
                let mut tool_calls: Vec<Value> = Vec::new();

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
                                        tool_calls.extend(tc.iter().cloned());
                                    }
                                }
                            }
                        }
                    }
                }

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
}
