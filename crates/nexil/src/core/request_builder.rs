//! Request building for each transport kind (completion, messages, responses).

use std::sync::Arc;

use reqwest::Client;
use serde_json::Value;

use super::errors::ConduitError;
use super::execution::LLMCore;
use super::provider_runtime::ProviderRuntime;
use super::request_adapters::normalize_responses_kwargs;
use super::tool_calls::{
    normalize_tool_calls, tool_call_arguments_string, tool_call_id, tool_call_name,
};
use crate::clients::parsing::TransportKind;
use crate::providers;

/// All the parameters needed for a single provider call.
#[derive(Debug, Clone)]
pub struct TransportCallRequest {
    pub client: Arc<Client>,
    pub provider_name: String,
    pub model_id: String,
    pub api_base: Option<String>,
    pub messages_payload: Vec<Value>,
    pub tools_payload: Option<Vec<Value>>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
    pub reasoning_effort: Option<Value>,
    pub kwargs: serde_json::Map<String, Value>,
    pub is_anthropic_oauth: bool,
}

impl LLMCore {
    /// Build kwargs with the correct max_tokens argument name for the provider.
    pub fn decide_kwargs_for_provider(
        provider: &str,
        max_tokens: Option<u32>,
        kwargs: &serde_json::Map<String, Value>,
    ) -> serde_json::Map<String, Value> {
        let max_tokens_arg = ProviderRuntime::completion_max_tokens_arg(provider);
        let mut clean = kwargs.clone();
        if clean.contains_key(&max_tokens_arg) {
            return clean;
        }
        if let Some(mt) = max_tokens {
            clean.insert(max_tokens_arg, Value::Number(mt.into()));
        }
        clean
    }

    /// Build kwargs for the responses transport format.
    pub fn decide_responses_kwargs(
        max_tokens: Option<u32>,
        kwargs: &serde_json::Map<String, Value>,
        drop_extra_headers: bool,
    ) -> serde_json::Map<String, Value> {
        let mut clean = kwargs.clone();
        if drop_extra_headers {
            clean.remove("extra_headers");
        }
        normalize_responses_kwargs(&mut clean);
        if clean.contains_key("max_output_tokens") || max_tokens.is_none() {
            return clean;
        }
        if let Some(mt) = max_tokens {
            clean.insert("max_output_tokens".to_owned(), Value::Number(mt.into()));
        }
        clean
    }

    /// Whether to include stream usage options for completion format.
    pub fn should_default_completion_stream_usage(provider_name: &str) -> bool {
        ProviderRuntime::should_include_completion_stream_usage(provider_name)
    }

    /// Add stream_options to kwargs if applicable.
    pub fn with_default_completion_stream_options(
        provider_name: &str,
        stream: bool,
        kwargs: &serde_json::Map<String, Value>,
    ) -> serde_json::Map<String, Value> {
        if !stream {
            return kwargs.clone();
        }
        if !Self::should_default_completion_stream_usage(provider_name) {
            return kwargs.clone();
        }
        if kwargs.contains_key("stream_options") {
            return kwargs.clone();
        }
        let mut result = kwargs.clone();
        result.insert(
            "stream_options".to_owned(),
            serde_json::json!({"include_usage": true}),
        );
        result
    }

    /// Add reasoning configuration to kwargs if applicable.
    pub fn with_responses_reasoning(
        kwargs: &serde_json::Map<String, Value>,
        reasoning_effort: Option<&Value>,
    ) -> serde_json::Map<String, Value> {
        let effort = match reasoning_effort {
            Some(e) if !e.is_null() => e,
            _ => return kwargs.clone(),
        };
        if kwargs.contains_key("reasoning") {
            return kwargs.clone();
        }
        let mut result = kwargs.clone();
        result.insert(
            "reasoning".to_owned(),
            serde_json::json!({"effort": effort}),
        );
        result
    }

    /// Convert completion-format tool schemas to responses format.
    pub fn convert_tools_for_responses(tools_payload: Option<&[Value]>) -> Option<Vec<Value>> {
        let tools = tools_payload?;
        if tools.is_empty() {
            return None;
        }

        let mut converted = Vec::with_capacity(tools.len());
        for tool in tools {
            if let Some(function) = tool.get("function").and_then(|f| f.as_object()) {
                let mut entry = serde_json::Map::new();
                entry.insert(
                    "type".to_owned(),
                    tool.get("type")
                        .cloned()
                        .unwrap_or(Value::String("function".to_owned())),
                );
                if let Some(name) = function.get("name") {
                    entry.insert("name".to_owned(), name.clone());
                }
                entry.insert(
                    "description".to_owned(),
                    function
                        .get("description")
                        .cloned()
                        .unwrap_or(Value::String(String::new())),
                );
                entry.insert(
                    "parameters".to_owned(),
                    function
                        .get("parameters")
                        .cloned()
                        .unwrap_or(serde_json::json!({})),
                );
                if let Some(strict) = function.get("strict") {
                    entry.insert("strict".to_owned(), strict.clone());
                }
                converted.push(Value::Object(entry));
            } else {
                converted.push(tool.clone());
            }
        }
        Some(converted)
    }

    /// Determine which transport to use for a given request.
    pub fn selected_transport(
        &self,
        provider_name: &str,
        model_id: &str,
        tools_payload: Option<&[Value]>,
        supports_responses: bool,
        preferred_transport: Option<TransportKind>,
    ) -> Result<TransportKind, ConduitError> {
        ProviderRuntime::new(provider_name, model_id, None, None, self.api_format())
            .selected_transport(tools_payload, supports_responses, preferred_transport)
    }

    /// Split messages into (instructions, input_items) for the responses format.
    ///
    /// System/developer messages are joined into the instructions string.
    /// Other messages are converted to responses input items.
    pub fn split_messages_for_responses(messages: &[Value]) -> (Option<String>, Vec<Value>) {
        let mut instruction_parts: Vec<String> = Vec::new();
        let mut filtered: Vec<&Value> = Vec::new();

        for message in messages {
            let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role == "system" || role == "developer" {
                if let Some(content) = message.get("content").and_then(|c| c.as_str())
                    && !content.is_empty()
                {
                    instruction_parts.push(content.to_owned());
                }
                continue;
            }
            filtered.push(message);
        }

        let instructions = if instruction_parts.is_empty() {
            None
        } else {
            let joined = instruction_parts
                .into_iter()
                .filter(|p| !p.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        };

        let input_items = Self::convert_messages_to_responses_input(&filtered);
        (instructions, input_items)
    }

    /// Convert filtered messages to the responses input format.
    fn convert_messages_to_responses_input(messages: &[&Value]) -> Vec<Value> {
        let mut items: Vec<Value> = Vec::new();

        for message in messages {
            let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let content = message.get("content");
            let content_str = content.and_then(|c| c.as_str()).unwrap_or("");

            // user/assistant messages with content
            if (role == "user" || role == "assistant") && !content_str.is_empty() {
                items.push(serde_json::json!({
                    "role": role,
                    "content": content_str,
                    "type": "message",
                }));
            }

            // assistant tool calls
            if role == "assistant"
                && let Some(tool_calls) = message.get("tool_calls").and_then(|tc| tc.as_array())
            {
                for (index, tool_call) in normalize_tool_calls(tool_calls).into_iter().enumerate() {
                    let name = tool_call_name(&tool_call).unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    let call_id = tool_call_id(&tool_call)
                        .map(|s| s.to_owned())
                        .unwrap_or_else(|| format!("call_{}", index + 1));
                    let arguments = tool_call_arguments_string(&tool_call);
                    items.push(serde_json::json!({
                        "type": "function_call",
                        "name": name,
                        "arguments": arguments,
                        "call_id": call_id,
                    }));
                }
            }

            // tool result messages
            if role == "tool" {
                let call_id = message
                    .get("tool_call_id")
                    .or_else(|| message.get("call_id"))
                    .and_then(|v| v.as_str());
                if let Some(cid) = call_id {
                    let output = message
                        .get("content")
                        .and_then(|c| c.as_str())
                        .unwrap_or("");
                    items.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": cid,
                        "output": output,
                    }));
                }
            }
        }

        items
    }

    /// Build the request URL for a given provider and transport.
    pub fn build_request_url(api_base: &str, transport: TransportKind) -> String {
        providers::adapter_for_transport(transport).build_request_url(api_base, transport)
    }

    /// Build the JSON body for a completion-format request.
    pub fn build_completion_body(
        request: &TransportCallRequest,
        provider_name: &str,
    ) -> Result<Value, ConduitError> {
        let mut adapter_request = request.clone();
        adapter_request.provider_name = provider_name.to_owned();
        providers::adapter_for_transport(TransportKind::Completion)
            .build_request_body(&adapter_request, TransportKind::Completion)
    }

    /// Build the JSON body for an Anthropic Messages-format request.
    pub fn build_messages_body(request: &TransportCallRequest) -> Result<Value, ConduitError> {
        providers::adapter_for_transport(TransportKind::Messages)
            .build_request_body(request, TransportKind::Messages)
    }

    /// Build the JSON body for a responses-format request.
    pub fn build_responses_body(request: &TransportCallRequest) -> Result<Value, ConduitError> {
        providers::adapter_for_transport(TransportKind::Responses)
            .build_request_body(request, TransportKind::Responses)
    }
}
