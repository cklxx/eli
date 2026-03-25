use std::collections::HashSet;

use serde_json::Value;

use crate::core::tool_calls::{
    normalize_tool_calls, tool_call_arguments_string, tool_call_id, tool_call_name,
};

struct ToolResolution {
    next_index: usize,
    resolved_ids: HashSet<String>,
    tool_result_blocks: Vec<Value>,
    trailing_user_blocks: Vec<Value>,
}

pub fn split_system_and_conversation(messages_payload: &[Value]) -> (Vec<String>, Vec<Value>) {
    let mut system_parts = Vec::new();
    let mut conversation = Vec::new();
    let mut index = 0;

    while index < messages_payload.len() {
        index = split_message(
            messages_payload,
            index,
            &mut system_parts,
            &mut conversation,
        );
    }

    (system_parts, normalize_messages(conversation))
}

fn split_message(
    messages_payload: &[Value],
    index: usize,
    system_parts: &mut Vec<String>,
    conversation: &mut Vec<Value>,
) -> usize {
    let msg = &messages_payload[index];
    if collect_system_block(msg, system_parts) {
        return index + 1;
    }
    if is_tool_assistant(msg) {
        return process_tool_assistant(messages_payload, index, msg, conversation);
    }
    if message_role(msg) != "tool" {
        push_conversation_entry(msg, conversation);
    }
    index + 1
}

fn collect_system_block(msg: &Value, system_parts: &mut Vec<String>) -> bool {
    if !matches!(message_role(msg), "system" | "developer") {
        return false;
    }
    if let Some(text) = extract_system_text(msg.get("content"))
        && !text.is_empty()
    {
        system_parts.push(text);
    }
    true
}

fn is_tool_assistant(msg: &Value) -> bool {
    message_role(msg) == "assistant" && has_tool_calls(msg)
}

fn process_tool_assistant(
    messages_payload: &[Value],
    index: usize,
    msg: &Value,
    conversation: &mut Vec<Value>,
) -> usize {
    let resolution = resolve_tool_results(messages_payload, index + 1, &tool_call_ids(msg));
    if resolution.resolved_ids.is_empty() {
        push_assistant_entry(msg, conversation, &HashSet::new());
        return index + 1;
    }
    push_assistant_entry(msg, conversation, &resolution.resolved_ids);
    push_tool_result_entry(
        resolution.tool_result_blocks,
        resolution.trailing_user_blocks,
        conversation,
    );
    resolution.next_index
}

fn resolve_tool_results(
    messages_payload: &[Value],
    mut lookahead: usize,
    call_ids: &HashSet<String>,
) -> ToolResolution {
    let mut resolved_ids = HashSet::new();
    let mut tool_result_blocks = Vec::new();
    let mut trailing_user_blocks = Vec::new();

    while lookahead < messages_payload.len() {
        let next = &messages_payload[lookahead];
        if message_role(next) == "assistant" {
            break;
        }
        process_followup_message(
            next,
            call_ids,
            &mut resolved_ids,
            &mut tool_result_blocks,
            &mut trailing_user_blocks,
        );
        lookahead += 1;
    }

    ToolResolution {
        next_index: lookahead,
        resolved_ids,
        tool_result_blocks,
        trailing_user_blocks,
    }
}

fn process_followup_message(
    msg: &Value,
    call_ids: &HashSet<String>,
    resolved_ids: &mut HashSet<String>,
    tool_result_blocks: &mut Vec<Value>,
    trailing_user_blocks: &mut Vec<Value>,
) {
    match message_role(msg) {
        "tool" => resolve_tool_result(msg, call_ids, resolved_ids, tool_result_blocks),
        "system" | "developer" => {}
        _ => trailing_user_blocks.extend(message_blocks(msg)),
    }
}

fn resolve_tool_result(
    msg: &Value,
    call_ids: &HashSet<String>,
    resolved_ids: &mut HashSet<String>,
    tool_result_blocks: &mut Vec<Value>,
) {
    if let Some((tool_use_id, block)) = tool_result_block(msg)
        && call_ids.contains(&tool_use_id)
    {
        resolved_ids.insert(tool_use_id);
        tool_result_blocks.push(block);
    }
}

fn push_assistant_entry(
    msg: &Value,
    conversation: &mut Vec<Value>,
    allowed_tool_ids: &HashSet<String>,
) {
    let blocks = assistant_content_blocks(msg, Some(allowed_tool_ids));
    push_blocks("assistant", blocks, conversation);
}

fn push_tool_result_entry(
    mut tool_result_blocks: Vec<Value>,
    trailing_user_blocks: Vec<Value>,
    conversation: &mut Vec<Value>,
) {
    tool_result_blocks.extend(trailing_user_blocks);
    push_blocks("user", tool_result_blocks, conversation);
}

fn push_conversation_entry(msg: &Value, conversation: &mut Vec<Value>) {
    push_blocks(message_role(msg), message_blocks(msg), conversation);
}

fn push_blocks(role: &str, blocks: Vec<Value>, conversation: &mut Vec<Value>) {
    if !blocks.is_empty() {
        conversation.push(serde_json::json!({
            "role": role,
            "content": blocks,
        }));
    }
}

pub fn normalize_messages(messages: Vec<Value>) -> Vec<Value> {
    if messages.is_empty() {
        return vec![
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "Continue."}]}),
        ];
    }

    let mut merged: Vec<Value> = Vec::with_capacity(messages.len());
    for msg in messages {
        let role = message_role(&msg).to_owned();
        let last_role = merged
            .last()
            .map(message_role)
            .unwrap_or_default()
            .to_owned();

        if !merged.is_empty() && role == last_role {
            if let Some(last) = merged.last_mut() {
                let existing = last.get("content").cloned().unwrap_or(Value::Null);
                let new_content = msg.get("content").cloned().unwrap_or(Value::Null);
                last["content"] = Value::Array(merge_blocks(existing, new_content));
            }
            continue;
        }
        merged.push(msg);
    }

    if merged.first().map(message_role) != Some("user") {
        merged.insert(
            0,
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "Continue."}]}),
        );
    }

    if merged.last().map(message_role) != Some("user") {
        merged.push(
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "Continue."}]}),
        );
    }

    merged
}

fn has_tool_calls(msg: &Value) -> bool {
    msg.get("tool_calls")
        .and_then(|value| value.as_array())
        .map(|calls| !calls.is_empty())
        .unwrap_or(false)
}

fn tool_call_ids(msg: &Value) -> HashSet<String> {
    msg.get("tool_calls")
        .and_then(|value| value.as_array())
        .map(|calls| normalize_tool_calls(calls))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|call| tool_call_id(&call).map(ToOwned::to_owned))
        .collect()
}

fn assistant_content_blocks(msg: &Value, allowed_tool_ids: Option<&HashSet<String>>) -> Vec<Value> {
    let mut blocks = content_blocks(msg.get("content").cloned().unwrap_or(Value::Null));

    if let Some(tool_calls) = msg.get("tool_calls").and_then(|value| value.as_array()) {
        for tool_call in normalize_tool_calls(tool_calls) {
            let id = tool_call_id(&tool_call).unwrap_or_default();

            if let Some(allowed) = allowed_tool_ids
                && !allowed.contains(id)
            {
                continue;
            }

            let name = tool_call_name(&tool_call).unwrap_or_default();
            let arguments = tool_call_arguments_string(&tool_call);
            let input = serde_json::from_str(&arguments).unwrap_or_else(|_| serde_json::json!({}));

            blocks.push(serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            }));
        }
    }

    blocks
}

fn tool_result_block(msg: &Value) -> Option<(String, Value)> {
    let tool_use_id = msg
        .get("tool_call_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned();
    if tool_use_id.is_empty() {
        return None;
    }

    let content = msg
        .get("content")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned();

    Some((
        tool_use_id.clone(),
        serde_json::json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
        }),
    ))
}

fn extract_system_text(content: Option<&Value>) -> Option<String> {
    match content {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(items)) => {
            let joined = items
                .iter()
                .filter_map(|item| {
                    if item.get("type").and_then(|value| value.as_str()) == Some("text") {
                        item.get("text")
                            .and_then(|value| value.as_str())
                            .map(ToOwned::to_owned)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        _ => None,
    }
}

fn merge_blocks(existing: Value, new_content: Value) -> Vec<Value> {
    let mut blocks = content_blocks(existing);
    blocks.extend(content_blocks(new_content));
    blocks
}

fn message_blocks(msg: &Value) -> Vec<Value> {
    content_blocks(msg.get("content").cloned().unwrap_or(Value::Null))
}

fn content_blocks(content: Value) -> Vec<Value> {
    match content {
        Value::Null => Vec::new(),
        Value::String(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![serde_json::json!({"type": "text", "text": text})]
            }
        }
        Value::Array(items) => items,
        Value::Object(_) => vec![content],
        other => vec![serde_json::json!({"type": "text", "text": other.to_string()})],
    }
}

fn message_role(msg: &Value) -> &str {
    msg.get("role")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_split_system_and_conversation_keeps_tool_results_adjacent() {
        let messages = vec![
            json!({"role": "system", "content": "system rules"}),
            json!({"role": "user", "content": "find papers"}),
            json!({
                "role": "assistant",
                "tool_calls": [
                    {"id": "toolu_1", "type": "function", "function": {"name": "bash", "arguments": "{\"cmd\":\"pwd\"}"}},
                    {"id": "toolu_2", "type": "function", "function": {"name": "skill", "arguments": "{\"name\":\"active-research\"}"}}
                ]
            }),
            json!({"role": "tool", "tool_call_id": "toolu_1", "content": "ok-1"}),
            json!({"role": "tool", "tool_call_id": "toolu_2", "content": "ok-2"}),
            json!({"role": "user", "content": "继续"}),
        ];

        let (system_parts, conversation) = split_system_and_conversation(&messages);
        assert_eq!(system_parts, vec!["system rules".to_owned()]);
        assert_eq!(conversation.len(), 3);
        assert_eq!(conversation[0]["role"], "user");
        assert_eq!(conversation[1]["role"], "assistant");
        assert_eq!(conversation[2]["role"], "user");

        let assistant_blocks = conversation[1]["content"].as_array().unwrap();
        assert_eq!(assistant_blocks.len(), 2);
        assert_eq!(assistant_blocks[0]["type"], "tool_use");
        assert_eq!(assistant_blocks[1]["type"], "tool_use");

        let user_blocks = conversation[2]["content"].as_array().unwrap();
        assert_eq!(user_blocks[0]["type"], "tool_result");
        assert_eq!(user_blocks[1]["type"], "tool_result");
        assert_eq!(user_blocks[2]["type"], "text");
        assert_eq!(user_blocks[2]["text"], "继续");
    }

    #[test]
    fn test_split_system_and_conversation_drops_unresolved_tool_use_blocks() {
        let messages = vec![
            json!({"role": "user", "content": "hello"}),
            json!({
                "role": "assistant",
                "content": "I will inspect that.",
                "tool_calls": [
                    {"id": "toolu_1", "type": "function", "function": {"name": "bash", "arguments": "{\"cmd\":\"pwd\"}"}}
                ]
            }),
            json!({"role": "user", "content": "继续"}),
        ];

        let (_system_parts, conversation) = split_system_and_conversation(&messages);
        assert_eq!(conversation.len(), 3);
        assert_eq!(conversation[0]["role"], "user");
        assert_eq!(conversation[1]["role"], "assistant");
        assert_eq!(conversation[2]["role"], "user");
        let assistant_blocks = conversation[1]["content"].as_array().unwrap();
        assert_eq!(assistant_blocks.len(), 1);
        assert_eq!(assistant_blocks[0]["type"], "text");
        assert_eq!(conversation[2]["content"][0]["text"], "继续");
    }

    #[test]
    fn test_normalize_messages_merges_tool_results_and_user_text() {
        let messages = vec![
            json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok"}]
            }),
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "continue"}]
            }),
        ];

        let normalized = normalize_messages(messages);
        assert_eq!(normalized.len(), 1);
        let blocks = normalized[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[1]["type"], "text");
    }
}
