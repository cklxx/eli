use serde_json::Value;

use crate::adapter::ProviderAdapter;
use crate::clients::parsing::TransportKind;
use crate::core::anthropic_messages;
use crate::core::errors::{ConduitError, ErrorKind};
use crate::core::request_builder::TransportCallRequest;

pub static ANTHROPIC_ADAPTER: AnthropicAdapter = AnthropicAdapter;

pub struct AnthropicAdapter;

impl ProviderAdapter for AnthropicAdapter {
    fn build_request_url(&self, api_base: &str, transport: TransportKind) -> String {
        // build_request_url doesn't return Result, so keep the debug_assert
        // as a development-time guard. Callers should not reach here with a
        // wrong transport because build_request_body validates first.
        debug_assert_eq!(transport, TransportKind::Messages);
        format!("{}/messages", api_base.trim_end_matches('/'))
    }

    fn build_request_body(
        &self,
        request: &TransportCallRequest,
        transport: TransportKind,
    ) -> Result<Value, ConduitError> {
        if transport != TransportKind::Messages {
            return Err(ConduitError::new(
                ErrorKind::Config,
                "anthropic adapter only supports messages transport",
            ));
        }

        let mut body = serde_json::Map::new();
        body.insert("model".to_owned(), Value::String(request.model_id.clone()));

        let max_tokens = request.max_tokens.unwrap_or(4096);
        body.insert("max_tokens".to_owned(), Value::Number(max_tokens.into()));

        let (system_parts, messages) =
            anthropic_messages::split_system_and_conversation(&request.messages_payload);

        if !system_parts.is_empty() {
            if request.is_anthropic_oauth {
                let mut system_list = vec![serde_json::json!({
                    "type": "text",
                    "text": "You are Claude Code, Anthropic's official CLI for Claude."
                })];
                let system_text = system_parts.join("\n\n");
                system_list.push(serde_json::json!({
                    "type": "text",
                    "text": system_text
                }));
                body.insert("system".to_owned(), Value::Array(system_list));
            } else {
                body.insert(
                    "system".to_owned(),
                    Value::String(system_parts.join("\n\n")),
                );
            }
        } else if request.is_anthropic_oauth {
            body.insert(
                "system".to_owned(),
                Value::Array(vec![serde_json::json!({
                    "type": "text",
                    "text": "You are Claude Code, Anthropic's official CLI for Claude."
                })]),
            );
        }

        body.insert("messages".to_owned(), Value::Array(messages));

        if request.is_anthropic_oauth {
            body.insert("stream".to_owned(), Value::Bool(true));
            body.insert(
                "thinking".to_owned(),
                serde_json::json!({"type": "adaptive"}),
            );
            body.insert(
                "output_config".to_owned(),
                serde_json::json!({"effort": "medium"}),
            );
        } else if request.stream {
            body.insert("stream".to_owned(), Value::Bool(true));
        }

        if let Some(ref tools) = request.tools_payload
            && !tools.is_empty()
        {
            let mut anthropic_tools: Vec<Value> = Vec::new();
            for tool in tools {
                if let Some(function) = tool.get("function").and_then(|f| f.as_object()) {
                    anthropic_tools.push(serde_json::json!({
                            "name": function.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                            "description": function.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                            "input_schema": function.get("parameters").cloned().unwrap_or(serde_json::json!({}))
                        }));
                } else {
                    anthropic_tools.push(tool.clone());
                }
            }
            body.insert("tools".to_owned(), Value::Array(anthropic_tools));
        }

        for (key, value) in &request.kwargs {
            if request.is_anthropic_oauth && key == "temperature" {
                continue;
            }
            body.entry(key.clone()).or_insert(value.clone());
        }

        Ok(Value::Object(body))
    }
}
