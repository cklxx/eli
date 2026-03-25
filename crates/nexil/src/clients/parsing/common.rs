//! Common parsing utilities shared by completion and responses adapters.

use serde_json::Value;

/// Safely access a field from a JSON value (object key lookup).
pub fn field<'a>(data: &'a Value, key: &str) -> Option<&'a Value> {
    data.get(key)
}

/// Access a field and return a default if missing.
pub fn field_or<'a>(data: &'a Value, key: &str, default: &'a Value) -> &'a Value {
    data.get(key).unwrap_or(default)
}

/// Access a string field, returning an empty string if missing or not a string.
pub fn field_str<'a>(data: &'a Value, key: &str) -> &'a str {
    data.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// Expand tool calls that have multiple concatenated JSON objects in their
/// `arguments` field into separate tool call entries.
pub fn expand_tool_calls(calls: Vec<Value>) -> Vec<Value> {
    let mut expanded = Vec::new();
    for call in calls {
        let mut items = expand_single_tool_call(&call);
        expanded.append(&mut items);
    }
    expanded
}

fn expand_single_tool_call(call: &Value) -> Vec<Value> {
    let function = match call.get("function") {
        Some(f) if f.is_object() => f,
        _ => return vec![call.clone()],
    };

    let arguments = match function.get("arguments").and_then(|a| a.as_str()) {
        Some(a) => a,
        None => return vec![call.clone()],
    };

    let chunks = split_concatenated_json_objects(arguments);
    if chunks.is_empty() {
        return vec![call.clone()];
    }

    let call_id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");

    let mut result = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.iter().enumerate() {
        let mut cloned = call.clone();
        if let Some(obj) = cloned.as_object_mut() {
            let mut func_clone = function.clone();
            if let Some(func_obj) = func_clone.as_object_mut() {
                func_obj.insert("arguments".to_owned(), Value::String(chunk.clone()));
            }
            obj.insert("function".to_owned(), func_clone);

            if !call_id.is_empty() && index > 0 {
                obj.insert(
                    "id".to_owned(),
                    Value::String(format!("{}__{}", call_id, index + 1)),
                );
            }
        }
        result.push(cloned);
    }
    result
}

/// Split a string that contains multiple concatenated JSON objects into
/// individual JSON object strings.  Returns an empty vec if the string
/// contains one or zero valid objects.
fn split_concatenated_json_objects(raw: &str) -> Vec<String> {
    let bytes = raw.as_bytes();
    let total = bytes.len();
    let mut chunks: Vec<String> = Vec::new();
    let mut position = 0;

    while position < total {
        // skip whitespace
        while position < total && bytes[position].is_ascii_whitespace() {
            position += 1;
        }
        if position >= total {
            break;
        }

        // Try to parse one JSON value starting at `position`.
        let slice = &raw[position..];
        let mut deserializer = serde_json::Deserializer::from_str(slice).into_iter::<Value>();
        match deserializer.next() {
            Some(Ok(val)) => {
                if !val.is_object() {
                    return Vec::new();
                }
                let end = deserializer.byte_offset();
                chunks.push(raw[position..position + end].to_owned());
                position += end;
            }
            _ => return Vec::new(),
        }
    }

    if chunks.len() <= 1 {
        return Vec::new();
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_single_object() {
        let result = split_concatenated_json_objects(r#"{"a": 1}"#);
        assert!(result.is_empty());
    }

    #[test]
    fn test_split_two_objects() {
        let result = split_concatenated_json_objects(r#"{"a": 1}{"b": 2}"#);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_split_invalid_json() {
        let result = split_concatenated_json_objects("not json");
        assert!(result.is_empty());
    }

    #[test]
    fn test_expand_tool_calls_no_concat() {
        let call = serde_json::json!({
            "id": "call_1",
            "function": {
                "name": "test",
                "arguments": "{\"x\": 1}"
            }
        });
        let expanded = expand_tool_calls(vec![call]);
        assert_eq!(expanded.len(), 1);
    }

    #[test]
    fn test_expand_tool_calls_with_concat() {
        let call = serde_json::json!({
            "id": "call_1",
            "function": {
                "name": "test",
                "arguments": "{\"x\": 1}{\"y\": 2}"
            }
        });
        let expanded = expand_tool_calls(vec![call]);
        assert_eq!(expanded.len(), 2);
        assert_eq!(
            expanded[0]["function"]["arguments"].as_str().unwrap(),
            "{\"x\": 1}"
        );
        assert_eq!(expanded[1]["id"].as_str().unwrap(), "call_1__2");
    }
}
