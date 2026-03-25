//! Common parsing utilities shared by completion and responses adapters.

use serde_json::Value;

pub fn field<'a>(data: &'a Value, key: &str) -> Option<&'a Value> {
    data.get(key)
}

pub fn field_or<'a>(data: &'a Value, key: &str, default: &'a Value) -> &'a Value {
    data.get(key).unwrap_or(default)
}

pub fn field_str<'a>(data: &'a Value, key: &str) -> &'a str {
    data.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

pub fn expand_tool_calls(calls: Vec<Value>) -> Vec<Value> {
    calls.iter().flat_map(expand_single_tool_call).collect()
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

    chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
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
            cloned
        })
        .collect()
}

fn split_concatenated_json_objects(raw: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut position = skip_whitespace(raw.as_bytes(), 0);

    while position < raw.len() {
        match parse_one_json_object(raw, position) {
            Some((chunk, end)) => {
                chunks.push(chunk);
                position = skip_whitespace(raw.as_bytes(), end);
            }
            None => return Vec::new(),
        }
    }

    if chunks.len() <= 1 {
        Vec::new()
    } else {
        chunks
    }
}

fn skip_whitespace(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

fn parse_one_json_object(raw: &str, position: usize) -> Option<(String, usize)> {
    let slice = &raw[position..];
    let mut deserializer = serde_json::Deserializer::from_str(slice).into_iter::<Value>();
    let val = deserializer.next()?.ok()?;
    if !val.is_object() {
        return None;
    }
    let end = deserializer.byte_offset();
    Some((raw[position..position + end].to_owned(), position + end))
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
