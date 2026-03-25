//! Canonical tool-call helpers shared across transports and tape history.

use serde_json::{Map, Value};

use crate::clients::parsing::common::expand_tool_calls;

pub(crate) fn normalize_tool_calls(calls: &[Value]) -> Vec<Value> {
    let normalized: Vec<Value> = calls
        .iter()
        .enumerate()
        .filter_map(|(index, call)| {
            normalize_tool_call_with_fallback(call, Some(format!("call_{}", index + 1)))
        })
        .collect();
    expand_tool_calls(normalized)
}

pub(crate) fn normalize_message_tool_calls(message: &Value) -> Value {
    let Some(obj) = message.as_object() else {
        return message.clone();
    };
    let Some(raw_calls) = obj.get("tool_calls").and_then(|value| value.as_array()) else {
        return message.clone();
    };

    let mut normalized = obj.clone();
    normalized.insert(
        "tool_calls".to_owned(),
        Value::Array(normalize_tool_calls(raw_calls)),
    );
    Value::Object(normalized)
}

pub(crate) fn tool_call_id(call: &Value) -> Option<&str> {
    call.get("id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            call.get("call_id")
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
        })
}

pub(crate) fn tool_call_name(call: &Value) -> Option<&str> {
    call.get("function")
        .and_then(|value| value.get("name"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            call.get("name")
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
        })
}

pub(crate) fn tool_call_arguments_string(call: &Value) -> String {
    call.get("function")
        .and_then(|value| value.get("arguments"))
        .map(json_field_to_string)
        .or_else(|| call.get("arguments").map(json_field_to_string))
        .or_else(|| call.get("input").map(json_field_to_string))
        .unwrap_or_else(|| "{}".to_owned())
}

fn normalize_tool_call_with_fallback(call: &Value, fallback_id: Option<String>) -> Option<Value> {
    let name = tool_call_name(call)?;
    let arguments = tool_call_arguments_string(call);

    let mut function = Map::new();
    function.insert("name".to_owned(), Value::String(name.to_owned()));
    function.insert("arguments".to_owned(), Value::String(arguments));

    let mut entry = Map::new();
    if let Some(id) = tool_call_id(call)
        .map(ToOwned::to_owned)
        .or(fallback_id)
        .filter(|value| !value.is_empty())
    {
        entry.insert("id".to_owned(), Value::String(id));
    }
    entry.insert("type".to_owned(), Value::String("function".to_owned()));
    entry.insert("function".to_owned(), Value::Object(function));

    Some(Value::Object(entry))
}

fn json_field_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize_tool_calls_canonicalizes_responses_shape() {
        let calls = normalize_tool_calls(&[json!({
            "type": "function_call",
            "call_id": "call_123",
            "name": "tape_info",
            "arguments": "{}"
        })]);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call_123");
        assert_eq!(calls[0]["type"], "function");
        assert_eq!(calls[0]["function"]["name"], "tape_info");
        assert_eq!(calls[0]["function"]["arguments"], "{}");
    }

    #[test]
    fn test_normalize_tool_calls_canonicalizes_anthropic_shape() {
        let calls = normalize_tool_calls(&[json!({
            "type": "tool_use",
            "id": "toolu_123",
            "name": "fs.read",
            "input": {"path": "AGENTS.md"}
        })]);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "toolu_123");
        assert_eq!(calls[0]["function"]["name"], "fs.read");
        assert_eq!(
            calls[0]["function"]["arguments"].as_str().unwrap(),
            r#"{"path":"AGENTS.md"}"#
        );
    }

    #[test]
    fn test_normalize_message_tool_calls_rewrites_message_payload() {
        let message = json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "type": "function_call",
                "call_id": "call_1",
                "name": "echo",
                "arguments": "{\"msg\":\"hi\"}"
            }]
        });

        let normalized = normalize_message_tool_calls(&message);

        assert_eq!(normalized["tool_calls"][0]["id"], "call_1");
        assert_eq!(normalized["tool_calls"][0]["function"]["name"], "echo");
    }
}
