//! Utilities for reading and normalizing envelopes (serde_json::Value).

use serde_json::Value;

use crate::types::Envelope;

/// Read a field from an envelope (must be a JSON object).
/// Returns the value at `key`, or `default` if not present or not an object.
pub fn field_of(message: &Envelope, key: &str, default: Option<&Value>) -> Option<Value> {
    match message.as_object() {
        Some(obj) => match obj.get(key) {
            Some(v) => Some(v.clone()),
            None => default.cloned(),
        },
        None => default.cloned(),
    }
}

/// Convenience: read a field as a string, with an optional default.
pub fn field_of_str(message: &Envelope, key: &str, default: &str) -> String {
    match message.as_object() {
        Some(obj) => match obj.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => default.to_string(),
        },
        None => default.to_string(),
    }
}

/// Get textual content from any envelope shape.
pub fn content_of(message: &Envelope) -> String {
    match message.as_object() {
        Some(obj) => match obj.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        },
        None => match message {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        },
    }
}

/// Convert an arbitrary envelope to a mutable JSON object.
/// If the envelope is already an object, it is cloned.
/// If it has some other shape, it is wrapped as `{"content": "<string>"}`.
pub fn normalize_envelope(message: &Envelope) -> Value {
    match message {
        Value::Object(_) => message.clone(),
        Value::String(s) => {
            serde_json::json!({ "content": s })
        }
        other => {
            serde_json::json!({ "content": other.to_string() })
        }
    }
}

/// Normalize one `render_outbound` return value to a flat list of envelopes.
/// Handles `null`, single objects, and arrays.
pub fn unpack_batch(batch: &Value) -> Vec<Envelope> {
    match batch {
        Value::Null => Vec::new(),
        Value::Array(arr) => arr.clone(),
        other => vec![other.clone()],
    }
}

/// Unpack a value that may itself be a batch (Vec) into a flat list of envelopes.
pub fn unpack_batch_vec(batches: Vec<Vec<Envelope>>) -> Vec<Envelope> {
    let mut out = Vec::new();
    for batch in batches {
        out.extend(batch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- field_of tests -------------------------------------------------------

    #[test]
    fn test_field_of_object() {
        let msg = json!({"channel": "telegram", "content": "hello"});
        assert_eq!(
            field_of(&msg, "channel", None),
            Some(Value::String("telegram".into()))
        );
        assert_eq!(field_of(&msg, "missing", None), None);
        assert_eq!(
            field_of(&msg, "missing", Some(&Value::String("fallback".into()))),
            Some(Value::String("fallback".into()))
        );
    }

    #[test]
    fn test_field_of_non_object_returns_default() {
        let msg = json!("just a string");
        assert_eq!(field_of(&msg, "content", None), None);
        assert_eq!(
            field_of(&msg, "content", Some(&json!("fb"))),
            Some(json!("fb"))
        );
    }

    #[test]
    fn test_field_of_numeric_value() {
        let msg = json!({"count": 3});
        assert_eq!(field_of(&msg, "count", None), Some(json!(3)));
    }

    // -- field_of_str tests ---------------------------------------------------

    #[test]
    fn test_field_of_str() {
        let msg = json!({"channel": "telegram"});
        assert_eq!(field_of_str(&msg, "channel", "default"), "telegram");
        assert_eq!(field_of_str(&msg, "missing", "default"), "default");
    }

    #[test]
    fn test_field_of_str_non_string_value_stringified() {
        let msg = json!({"count": 123});
        assert_eq!(field_of_str(&msg, "count", "0"), "123");
    }

    #[test]
    fn test_field_of_str_non_object_returns_default() {
        let msg = json!(42);
        assert_eq!(field_of_str(&msg, "anything", "fallback"), "fallback");
    }

    // -- content_of tests -----------------------------------------------------

    #[test]
    fn test_content_of_object() {
        let msg = json!({"content": "hello world"});
        assert_eq!(content_of(&msg), "hello world");
    }

    #[test]
    fn test_content_of_string() {
        let msg = json!("raw text");
        assert_eq!(content_of(&msg), "raw text");
    }

    #[test]
    fn test_content_of_no_content() {
        let msg = json!({"channel": "test"});
        assert_eq!(content_of(&msg), "");
    }

    #[test]
    fn test_content_of_numeric_content_stringified() {
        let msg = json!({"content": 123});
        assert_eq!(content_of(&msg), "123");
    }

    #[test]
    fn test_content_of_numeric_non_object() {
        let msg = json!(42);
        assert_eq!(content_of(&msg), "42");
    }

    #[test]
    fn test_content_of_bool_non_object() {
        let msg = json!(true);
        assert_eq!(content_of(&msg), "true");
    }

    // -- normalize_envelope tests ---------------------------------------------

    #[test]
    fn test_normalize_envelope_object() {
        let msg = json!({"a": 1});
        let out = normalize_envelope(&msg);
        assert_eq!(out, json!({"a": 1}));
    }

    #[test]
    fn test_normalize_envelope_object_is_clone_not_same() {
        let msg = json!({"content": "hello"});
        let out = normalize_envelope(&msg);
        assert_eq!(out, msg);
    }

    #[test]
    fn test_normalize_envelope_string() {
        let msg = json!("hello");
        let out = normalize_envelope(&msg);
        assert_eq!(out, json!({"content": "hello"}));
    }

    #[test]
    fn test_normalize_envelope_number() {
        let msg = json!(42);
        let out = normalize_envelope(&msg);
        assert_eq!(out, json!({"content": "42"}));
    }

    #[test]
    fn test_normalize_envelope_bool() {
        let msg = json!(true);
        let out = normalize_envelope(&msg);
        assert_eq!(out, json!({"content": "true"}));
    }

    #[test]
    fn test_normalize_envelope_null() {
        let msg = json!(null);
        let out = normalize_envelope(&msg);
        assert_eq!(out, json!({"content": "null"}));
    }

    // -- unpack_batch tests ---------------------------------------------------

    #[test]
    fn test_unpack_batch_null() {
        assert!(unpack_batch(&Value::Null).is_empty());
    }

    #[test]
    fn test_unpack_batch_array() {
        let batch = json!([{"content": "a"}, {"content": "b"}]);
        let out = unpack_batch(&batch);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], json!({"content": "a"}));
        assert_eq!(out[1], json!({"content": "b"}));
    }

    #[test]
    fn test_unpack_batch_single() {
        let batch = json!({"content": "one"});
        let out = unpack_batch(&batch);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], json!({"content": "one"}));
    }

    #[test]
    fn test_unpack_batch_empty_array() {
        let batch = json!([]);
        let out = unpack_batch(&batch);
        assert!(out.is_empty());
    }

    // -- unpack_batch_vec tests -----------------------------------------------

    #[test]
    fn test_unpack_batch_vec_flattens() {
        let batches = vec![
            vec![json!({"content": "a"})],
            vec![json!({"content": "b"}), json!({"content": "c"})],
        ];
        let out = unpack_batch_vec(batches);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], json!({"content": "a"}));
        assert_eq!(out[2], json!({"content": "c"}));
    }

    #[test]
    fn test_unpack_batch_vec_empty() {
        let out = unpack_batch_vec(Vec::new());
        assert!(out.is_empty());
    }
}
