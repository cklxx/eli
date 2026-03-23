//! Small utility functions for the Eli framework.

use std::path::PathBuf;

use conduit::tape::TapeEntry;
use serde_json::Value;

use crate::types::State;

/// Remove all keys whose value is `Value::Null` from a JSON object map.
/// Non-object values are returned unchanged.
pub fn exclude_none(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let filtered: serde_json::Map<String, Value> = map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Value::Object(filtered)
        }
        other => other.clone(),
    }
}

/// Exclude null values from a `HashMap<String, Value>`.
pub fn exclude_none_map(
    map: &std::collections::HashMap<String, Value>,
) -> std::collections::HashMap<String, Value> {
    map.iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Extract the workspace path from framework state.
/// Falls back to the current working directory if `_runtime_workspace` is absent.
pub fn workspace_from_state(state: &State) -> PathBuf {
    if let Some(Value::String(raw)) = state.get("_runtime_workspace") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.is_absolute() {
                return path;
            }
            // Try to canonicalize relative paths
            if let Ok(canonical) = std::fs::canonicalize(&path) {
                return canonical;
            }
            return path;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Get a YAML text representation of a tape entry's payload.
pub fn get_entry_text(entry: &TapeEntry) -> String {
    serde_yaml::to_string(&entry.payload).unwrap_or_else(|_| entry.payload.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap as StdHashMap;

    // -- exclude_none tests ---------------------------------------------------

    #[test]
    fn test_exclude_none_removes_null_values() {
        let input = json!({"a": 1, "b": null, "c": "hello", "d": false});
        let output = exclude_none(&input);
        assert_eq!(output, json!({"a": 1, "c": "hello", "d": false}));
    }

    #[test]
    fn test_exclude_none_no_nulls() {
        let input = json!({"a": 1, "b": 2});
        let output = exclude_none(&input);
        assert_eq!(output, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn test_exclude_none_all_nulls() {
        let input = json!({"a": null, "b": null});
        let output = exclude_none(&input);
        assert_eq!(output, json!({}));
    }

    #[test]
    fn test_exclude_none_non_object_returned_unchanged() {
        assert_eq!(exclude_none(&json!("hello")), json!("hello"));
        assert_eq!(exclude_none(&json!(42)), json!(42));
        assert_eq!(exclude_none(&json!(null)), json!(null));
        assert_eq!(exclude_none(&json!([1, 2])), json!([1, 2]));
    }

    // -- exclude_none_map tests -----------------------------------------------

    #[test]
    fn test_exclude_none_map_removes_nulls() {
        let mut map = StdHashMap::new();
        map.insert("a".to_owned(), json!(1));
        map.insert("b".to_owned(), json!(null));
        map.insert("c".to_owned(), json!("x"));
        let output = exclude_none_map(&map);
        assert_eq!(output.len(), 2);
        assert_eq!(output.get("a"), Some(&json!(1)));
        assert_eq!(output.get("c"), Some(&json!("x")));
        assert!(!output.contains_key("b"));
    }

    // -- workspace_from_state tests -------------------------------------------

    #[test]
    fn test_workspace_from_state_with_absolute_path() {
        let mut state = State::new();
        state.insert(
            "_runtime_workspace".into(),
            Value::String("/tmp/test".into()),
        );
        let ws = workspace_from_state(&state);
        assert_eq!(ws, PathBuf::from("/tmp/test"));
    }

    #[test]
    fn test_workspace_from_state_without_workspace_falls_back_to_cwd() {
        let state = State::new();
        let ws = workspace_from_state(&state);
        assert!(ws.is_absolute() || ws == PathBuf::from("."));
    }

    #[test]
    fn test_workspace_from_state_blank_workspace_falls_back_to_cwd() {
        let mut state = State::new();
        state.insert("_runtime_workspace".into(), Value::String("   ".into()));
        let ws = workspace_from_state(&state);
        // Should fall back to cwd since trimmed is empty
        assert!(ws.is_absolute() || ws == PathBuf::from("."));
    }

    #[test]
    fn test_workspace_from_state_non_string_value_falls_back() {
        let mut state = State::new();
        state.insert("_runtime_workspace".into(), json!(42));
        let ws = workspace_from_state(&state);
        assert!(ws.is_absolute() || ws == PathBuf::from("."));
    }

    // -- get_entry_text tests -------------------------------------------------

    #[test]
    fn test_get_entry_text_renders_yaml() {
        let entry = TapeEntry::new(
            1,
            "message".into(),
            json!({"content": "hello"}),
            json!({}),
            "2024-01-01T00:00:00Z".into(),
        );
        let text = get_entry_text(&entry);
        assert!(text.contains("hello"));
    }

    #[test]
    fn test_get_entry_text_handles_nested_payload() {
        let entry = TapeEntry::new(
            2,
            "event".into(),
            json!({"name": "step", "data": {"x": 1}}),
            json!({}),
            "2024-01-01T00:00:00Z".into(),
        );
        let text = get_entry_text(&entry);
        assert!(text.contains("name"));
        assert!(text.contains("step"));
    }
}
