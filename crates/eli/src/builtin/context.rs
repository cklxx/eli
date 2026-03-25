//! Tape context helpers — builds LLM message lists from tape entries.

use std::collections::HashMap;

use nexil::TapeEntry;
use serde_json::Value;

/// Build the default list of LLM messages from a sequence of tape entries.
///
/// This is the Rust equivalent of `_select_messages` in the Python codebase.
pub fn select_messages(entries: &[TapeEntry]) -> Vec<HashMap<String, Value>> {
    let mut messages: Vec<HashMap<String, Value>> = Vec::new();
    let mut pending_calls: Vec<Value> = Vec::new();

    for entry in entries {
        match entry.kind.as_str() {
            "anchor" => append_anchor_entry(&mut messages, entry),
            "message" => append_message_entry(&mut messages, entry),
            "tool_call" => {
                pending_calls = append_tool_call_entry(&mut messages, entry);
            }
            "tool_result" => {
                append_tool_result_entry(&mut messages, &pending_calls, entry);
                pending_calls = Vec::new();
            }
            _ => {}
        }
    }

    messages
}

fn append_anchor_entry(messages: &mut Vec<HashMap<String, Value>>, entry: &TapeEntry) {
    let name = entry
        .payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let state = entry
        .payload
        .get("state")
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .unwrap_or_else(|| "null".to_owned());
    let content = format!("[Anchor created: {name}]: {state}");
    let mut msg: HashMap<String, Value> = HashMap::new();
    msg.insert("role".to_owned(), Value::String("assistant".to_owned()));
    msg.insert("content".to_owned(), Value::String(content));
    messages.push(msg);
}

fn append_message_entry(messages: &mut Vec<HashMap<String, Value>>, entry: &TapeEntry) {
    if let Some(obj) = entry.payload.as_object() {
        let msg: HashMap<String, Value> = obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        messages.push(msg);
    }
}

fn append_tool_call_entry(
    messages: &mut Vec<HashMap<String, Value>>,
    entry: &TapeEntry,
) -> Vec<Value> {
    let calls = normalize_tool_calls(entry.payload.get("calls"));
    if !calls.is_empty() {
        let mut msg: HashMap<String, Value> = HashMap::new();
        msg.insert("role".to_owned(), Value::String("assistant".to_owned()));
        let content = entry
            .payload
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        msg.insert("content".to_owned(), Value::String(content));
        msg.insert("tool_calls".to_owned(), Value::Array(calls.clone()));
        messages.push(msg);
    }
    calls
}

fn append_tool_result_entry(
    messages: &mut Vec<HashMap<String, Value>>,
    pending_calls: &[Value],
    entry: &TapeEntry,
) {
    let results = match entry.payload.get("results") {
        Some(Value::Array(arr)) => arr.clone(),
        _ => return,
    };

    for (index, result) in results.iter().enumerate() {
        let mut msg: HashMap<String, Value> = HashMap::new();
        msg.insert("role".to_owned(), Value::String("tool".to_owned()));
        msg.insert(
            "content".to_owned(),
            Value::String(render_tool_result(result)),
        );

        if index < pending_calls.len() {
            let call = &pending_calls[index];
            if let Some(call_id) = call.get("id").and_then(|v| v.as_str())
                && !call_id.is_empty()
            {
                msg.insert("tool_call_id".to_owned(), Value::String(call_id.to_owned()));
            }
            if let Some(function) = call.get("function").and_then(|v| v.as_object())
                && let Some(name) = function.get("name").and_then(|v| v.as_str())
                && !name.is_empty()
            {
                msg.insert("name".to_owned(), Value::String(name.to_owned()));
            }
        }

        messages.push(msg);
    }
}

fn normalize_tool_calls(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter(|item| item.is_object())
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

fn render_tool_result(result: &Value) -> String {
    match result {
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| format!("{other}")),
    }
}
