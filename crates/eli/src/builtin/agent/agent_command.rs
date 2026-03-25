//! Command parsing helpers for internal slash-commands.

use std::collections::HashMap;

use serde_json::Value;

pub(super) fn parse_internal_command(line: &str) -> (String, Vec<String>) {
    let parts: Vec<String> = shell_words::split(line)
        .unwrap_or_else(|_| line.split_whitespace().map(|s| s.to_owned()).collect());
    if parts.is_empty() {
        return (String::new(), Vec::new());
    }
    let name = parts[0].clone();
    let rest = parts[1..].to_vec();
    (name, rest)
}

pub(super) struct Args {
    pub positional: Vec<String>,
    pub kwargs: HashMap<String, String>,
}

pub(super) fn parse_args(tokens: &[String]) -> Args {
    let mut positional: Vec<String> = Vec::new();
    let mut kwargs: HashMap<String, String> = HashMap::new();
    let mut seen_kwarg = false;

    for token in tokens {
        if let Some(eq_pos) = token.find('=') {
            let key = token[..eq_pos].to_owned();
            let value = token[eq_pos + 1..].to_owned();
            kwargs.insert(key, value);
            seen_kwarg = true;
        } else if seen_kwarg {
            tracing::warn!("positional argument '{}' after keyword arguments", token);
        } else {
            positional.push(token.clone());
        }
    }

    Args { positional, kwargs }
}

pub(super) fn args_to_json(args: &Args) -> Value {
    let mut map = serde_json::Map::new();
    for (k, v) in &args.kwargs {
        map.insert(k.clone(), Value::String(v.clone()));
    }
    if !args.positional.is_empty() && map.is_empty() {
        map.insert("value".to_owned(), Value::String(args.positional.join(" ")));
    }
    Value::Object(map)
}
