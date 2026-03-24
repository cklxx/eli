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

/// Iterate over parsed JSON payloads from SSE `data:` lines.
fn sse_events(buffer: &str) -> impl Iterator<Item = Value> + '_ {
    buffer.lines().filter_map(|line| {
        let data = line.trim().strip_prefix("data: ")?;
        serde_json::from_str::<Value>(data).ok()
    })
}

/// Append `suffix` to a string-valued map entry, inserting if absent.
fn concat_str_entry(map: &mut serde_json::Map<String, Value>, key: &str, suffix: &str) {
    let val = map
        .entry(key)
        .or_insert_with(|| Value::String(String::new()));
    if let Some(s) = val.as_str() {
        *val = Value::String(format!("{s}{suffix}"));
    }
}

/// Merge one OpenAI tool_call delta (by `index`) into the accumulator.
fn merge_tool_call_delta(map: &mut BTreeMap<u64, serde_json::Map<String, Value>>, delta: &Value) {
    let idx = delta.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
    let entry = map.entry(idx).or_default();
    // Keep first-seen scalars (id, type)
    for key in ["id", "type"] {
        if let Some(v) = delta.get(key).filter(|_| !entry.contains_key(key)) {
            entry.insert(key.to_owned(), v.clone());
        }
    }
    // Merge function sub-object: name once, arguments concatenated
    let Some(fd) = delta.get("function").and_then(|f| f.as_object()) else {
        return;
    };
    let fn_obj = entry
        .entry("function")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .unwrap();
    if let Some(n) = fd.get("name").and_then(|n| n.as_str()) {
        fn_obj.entry("name").or_insert(Value::String(n.to_owned()));
    }
    if let Some(a) = fd.get("arguments").and_then(|a| a.as_str()) {
        concat_str_entry(fn_obj, "arguments", a);
    }
}

/// Build a Completion-format response JSON from accumulated parts.
fn build_completion_response(
    content: &str,
    tc_map: BTreeMap<u64, serde_json::Map<String, Value>>,
) -> Value {
    let tc_vec: Vec<Value> = tc_map.into_values().map(Value::Object).collect();
    let mut msg = serde_json::json!({"role": "assistant", "content": content});
    if !tc_vec.is_empty() {
        msg["tool_calls"] = Value::Array(tc_vec);
    }
    serde_json::json!({"choices": [{"message": msg}]})
}

/// Parse OpenAI Completion-format SSE into assembled response.
fn parse_completion_sse(buffer: &str) -> Result<Value, ConduitError> {
    let mut content = String::new();
    let mut tc_map: BTreeMap<u64, serde_json::Map<String, Value>> = BTreeMap::new();

    for event in sse_events(buffer) {
        let Some(choices) = event.get("choices").and_then(|c| c.as_array()) else {
            continue;
        };
        for choice in choices {
            let Some(delta) = choice.get("delta") else {
                continue;
            };
            if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                content.push_str(c);
            }
            if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                tcs.iter()
                    .for_each(|d| merge_tool_call_delta(&mut tc_map, d));
            }
        }
    }
    Ok(build_completion_response(&content, tc_map))
}

/// Parse OpenAI Responses-format SSE (look for `response.completed`).
fn parse_responses_sse(buffer: &str) -> Result<Value, ConduitError> {
    for event in sse_events(buffer) {
        if event.get("type").and_then(|t| t.as_str()) == Some("response.completed")
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

/// Parse Anthropic Messages-format SSE into assembled response.
fn parse_messages_sse(buffer: &str) -> Result<Value, ConduitError> {
    let mut content = String::new();
    let mut tool_use_blocks: Vec<Value> = Vec::new();
    let mut current_tool: Option<serde_json::Map<String, Value>> = None;
    let mut tool_args = String::new();
    let mut usage: Option<Value> = None;

    for event in sse_events(buffer) {
        match event.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "content_block_start" => {
                if let Some(block) = event
                    .get("content_block")
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
                {
                    let mut tool = serde_json::Map::new();
                    for key in ["id", "name"] {
                        if let Some(v) = block.get(key) {
                            tool.insert(key.to_owned(), v.clone());
                        }
                    }
                    tool_args.clear();
                    current_tool = Some(tool);
                }
            }
            "content_block_delta" => match event.pointer("/delta/type").and_then(|t| t.as_str()) {
                Some("text_delta") => {
                    if let Some(t) = event.pointer("/delta/text").and_then(|t| t.as_str()) {
                        content.push_str(t);
                    }
                }
                Some("input_json_delta") => {
                    if let Some(p) = event
                        .pointer("/delta/partial_json")
                        .and_then(|p| p.as_str())
                    {
                        tool_args.push_str(p);
                    }
                }
                _ => {}
            },
            "content_block_stop" => {
                if let Some(mut tool) = current_tool.take() {
                    let input: Value =
                        serde_json::from_str(&tool_args).unwrap_or(serde_json::json!({}));
                    tool.insert("input".to_owned(), input);
                    tool.insert("type".to_owned(), Value::String("tool_use".to_owned()));
                    tool_use_blocks.push(Value::Object(tool));
                    tool_args.clear();
                }
            }
            "message_delta" => {
                usage = event.get("usage").cloned();
            }
            _ => {}
        }
    }

    let mut blocks: Vec<Value> = Vec::new();
    if !content.is_empty() {
        blocks.push(serde_json::json!({"type": "text", "text": content}));
    }
    blocks.extend(tool_use_blocks);
    let mut result = serde_json::json!({"role": "assistant", "content": blocks});
    if let Some(u) = usage {
        result
            .as_object_mut()
            .unwrap()
            .insert("usage".to_owned(), u);
    }
    Ok(result)
}

/// Parse a collected SSE buffer into a single assembled JSON response.
pub(crate) fn parse_sse_buffer(
    buffer: &str,
    transport: TransportKind,
) -> Result<Value, ConduitError> {
    match transport {
        TransportKind::Messages => parse_messages_sse(buffer),
        TransportKind::Responses => parse_responses_sse(buffer),
        _ => parse_completion_sse(buffer),
    }
}

impl LLMCore {
    /// Collect an SSE streaming response into a single JSON value.
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
        let sse = "\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"lo\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"cation\\\": \\\"NYC\\\"}\"}}]}}]}\n\
data: [DONE]\n";

        let result = parse_sse_buffer(sse, TransportKind::Completion).unwrap();
        let tool_calls = result["choices"][0]["message"]["tool_calls"]
            .as_array()
            .expect("tool_calls should be an array");

        assert_eq!(tool_calls.len(), 1, "deltas with same index should merge");

        let tc = &tool_calls[0];
        assert_eq!(tc["id"], "call_abc");
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "get_weather");
        assert_eq!(
            tc["function"]["arguments"], "{\"location\": \"NYC\"}",
            "arguments from two deltas should be concatenated"
        );
    }

    #[test]
    fn test_tool_call_delta_merge_multiple_indices() {
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
        assert_eq!(tool_calls[0]["function"]["name"], "foo");
        assert_eq!(tool_calls[0]["function"]["arguments"], "{\"a\": 1}");
        assert_eq!(tool_calls[1]["function"]["name"], "bar");
        assert_eq!(tool_calls[1]["function"]["arguments"], "{\"b\": 2}");
    }
}
