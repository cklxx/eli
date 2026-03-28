//! SSE stream collection and response assembly.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use tracing::info;

use super::errors::{ConduitError, ErrorKind};
use super::execution::LLMCore;
use crate::clients::parsing::TransportKind;

/// A transport kind paired with the raw response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportResponse {
    pub transport: TransportKind,
    pub payload: Value,
}

fn sse_events(buffer: &str) -> impl Iterator<Item = Value> + '_ {
    buffer.lines().filter_map(|line| {
        let data = line.trim().strip_prefix("data: ")?;
        serde_json::from_str::<Value>(data).ok()
    })
}

fn concat_str_entry(map: &mut serde_json::Map<String, Value>, key: &str, suffix: &str) {
    let val = map
        .entry(key)
        .or_insert_with(|| Value::String(String::new()));
    if let Some(s) = val.as_str() {
        *val = Value::String(format!("{s}{suffix}"));
    }
}

fn merge_tool_call_delta(map: &mut BTreeMap<u64, serde_json::Map<String, Value>>, delta: &Value) {
    let idx = delta.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
    let entry = map.entry(idx).or_default();
    for key in ["id", "type"] {
        if let Some(v) = delta.get(key).filter(|_| !entry.contains_key(key)) {
            entry.insert(key.to_owned(), v.clone());
        }
    }
    let Some(fd) = delta.get("function").and_then(|f| f.as_object()) else {
        return;
    };
    let Some(fn_obj) = entry
        .entry("function")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
    else {
        return;
    };
    if let Some(n) = fd.get("name").and_then(|n| n.as_str()) {
        fn_obj.entry("name").or_insert(Value::String(n.to_owned()));
    }
    if let Some(a) = fd.get("arguments").and_then(|a| a.as_str()) {
        concat_str_entry(fn_obj, "arguments", a);
    }
}

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

struct MessagesAccumulator {
    content: String,
    tool_use_blocks: Vec<Value>,
    current_tool: Option<serde_json::Map<String, Value>>,
    tool_args: String,
    usage: Option<Value>,
}

impl MessagesAccumulator {
    fn new() -> Self {
        Self {
            content: String::new(),
            tool_use_blocks: Vec::new(),
            current_tool: None,
            tool_args: String::new(),
            usage: None,
        }
    }

    fn process_event(&mut self, event: &Value) {
        match event.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "content_block_start" => self.handle_block_start(event),
            "content_block_delta" => self.handle_block_delta(event),
            "content_block_stop" => self.handle_block_stop(),
            "message_delta" => {
                self.usage = event.get("usage").cloned();
            }
            _ => {}
        }
    }

    fn handle_block_start(&mut self, event: &Value) {
        let Some(block) = event
            .get("content_block")
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
        else {
            return;
        };
        let tool = ["id", "name"]
            .iter()
            .filter_map(|key| Some(((*key).to_owned(), block.get(*key)?.clone())))
            .collect();
        self.tool_args.clear();
        self.current_tool = Some(tool);
    }

    fn handle_block_delta(&mut self, event: &Value) {
        match event.pointer("/delta/type").and_then(|t| t.as_str()) {
            Some("text_delta") => {
                if let Some(t) = event.pointer("/delta/text").and_then(|t| t.as_str()) {
                    self.content.push_str(t);
                }
            }
            Some("input_json_delta") => {
                if let Some(p) = event
                    .pointer("/delta/partial_json")
                    .and_then(|p| p.as_str())
                {
                    self.tool_args.push_str(p);
                }
            }
            _ => {}
        }
    }

    fn handle_block_stop(&mut self) {
        if let Some(mut tool) = self.current_tool.take() {
            let input: Value =
                serde_json::from_str(&self.tool_args).unwrap_or(serde_json::json!({}));
            tool.insert("input".to_owned(), input);
            tool.insert("type".to_owned(), Value::String("tool_use".to_owned()));
            self.tool_use_blocks.push(Value::Object(tool));
            self.tool_args.clear();
        }
    }

    fn into_response(self) -> Value {
        let text_block = (!self.content.is_empty())
            .then(|| serde_json::json!({"type": "text", "text": self.content}));
        let blocks: Vec<Value> = text_block.into_iter().chain(self.tool_use_blocks).collect();
        let mut result = serde_json::json!({"role": "assistant", "content": blocks});
        if let Some(u) = self.usage
            && let Value::Object(obj) = &mut result
        {
            obj.insert("usage".to_owned(), u);
        }
        result
    }
}

fn parse_messages_sse(buffer: &str) -> Result<Value, ConduitError> {
    let mut acc = MessagesAccumulator::new();
    for event in sse_events(buffer) {
        acc.process_event(&event);
    }
    Ok(acc.into_response())
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
                info!(
                    target: "eli_trace",
                    error = ?e,
                    bytes_received = buffer.len(),
                    "sse_stream_chunk_error"
                );
                ConduitError::new(ErrorKind::Provider, format!("SSE stream error: {e:?}"))
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
