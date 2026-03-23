//! Request-shape adapters for different upstream APIs.

use serde_json::Value;

/// Normalize completion-style kwargs into responses-compatible shapes.
///
/// Specifically, converts `tool_choice` from the completion format
/// (`{ "function": { "name": "..." } }`) to the responses format
/// (`{ "type": "function", "name": "..." }`).
pub fn normalize_responses_kwargs(
    kwargs: &mut serde_json::Map<String, Value>,
) -> &mut serde_json::Map<String, Value> {
    let tool_choice = match kwargs.get("tool_choice") {
        Some(tc) if tc.is_object() => tc.clone(),
        _ => return kwargs,
    };

    let function = match tool_choice.get("function") {
        Some(f) if f.is_object() => f,
        _ => return kwargs,
    };

    let function_name = match function.get("name").and_then(|n| n.as_str()) {
        Some(n) if !n.is_empty() => n.to_owned(),
        _ => return kwargs,
    };

    let mut normalized = tool_choice.as_object().cloned().unwrap_or_default();
    normalized.remove("function");
    if !normalized.contains_key("type") {
        normalized.insert("type".to_owned(), Value::String("function".to_owned()));
    }
    normalized.insert("name".to_owned(), Value::String(function_name));
    kwargs.insert("tool_choice".to_owned(), Value::Object(normalized));
    kwargs
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize_responses_kwargs_with_function() {
        let mut map = serde_json::Map::new();
        map.insert(
            "tool_choice".to_owned(),
            json!({
                "type": "function",
                "function": { "name": "my_tool" }
            }),
        );
        normalize_responses_kwargs(&mut map);
        let tc = map.get("tool_choice").unwrap();
        assert_eq!(tc.get("name").unwrap(), "my_tool");
        assert!(tc.get("function").is_none());
    }

    #[test]
    fn test_normalize_responses_kwargs_without_function() {
        let mut map = serde_json::Map::new();
        map.insert("tool_choice".to_owned(), json!("auto"));
        normalize_responses_kwargs(&mut map);
        assert_eq!(map.get("tool_choice").unwrap(), "auto");
    }
}
