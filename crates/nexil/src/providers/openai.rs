use serde_json::Value;

use crate::adapter::ProviderAdapter;
use crate::clients::parsing::TransportKind;
use crate::core::errors::{ConduitError, ErrorKind};
use crate::core::execution::LLMCore;
use crate::core::request_builder::TransportCallRequest;

pub static OPENAI_ADAPTER: OpenAIAdapter = OpenAIAdapter;

pub struct OpenAIAdapter;

impl ProviderAdapter for OpenAIAdapter {
    fn build_request_url(&self, api_base: &str, transport: TransportKind) -> String {
        let base = api_base.trim_end_matches('/');
        match transport {
            TransportKind::Completion => format!("{}/chat/completions", base),
            TransportKind::Responses => format!("{}/responses", base),
            TransportKind::Messages => format!("{}/messages", base),
        }
    }

    fn build_request_body(
        &self,
        request: &TransportCallRequest,
        transport: TransportKind,
    ) -> Result<Value, ConduitError> {
        match transport {
            TransportKind::Completion => Ok(self.build_completion_body(request)),
            TransportKind::Responses => Ok(self.build_responses_body(request)),
            TransportKind::Messages => Err(ConduitError::new(
                ErrorKind::Config,
                "openai adapter does not support messages transport",
            )),
        }
    }
}

impl OpenAIAdapter {
    fn build_completion_body(&self, request: &TransportCallRequest) -> Value {
        let mut kwargs = LLMCore::decide_kwargs_for_provider(
            &request.provider_name,
            request.max_tokens,
            &request.kwargs,
        );
        kwargs = LLMCore::with_default_completion_stream_options(
            &request.provider_name,
            request.stream,
            &kwargs,
        );

        let mut body = serde_json::Map::new();
        body.insert("model".to_owned(), Value::String(request.model_id.clone()));
        body.insert(
            "messages".to_owned(),
            Value::Array(request.messages_payload.clone()),
        );
        if request.stream {
            body.insert("stream".to_owned(), Value::Bool(true));
        }
        if let Some(ref tools) = request.tools_payload
            && !tools.is_empty()
        {
            body.insert("tools".to_owned(), Value::Array(tools.clone()));
        }
        if let Some(ref effort) = request.reasoning_effort
            && !effort.is_null()
        {
            body.insert("reasoning_effort".to_owned(), effort.clone());
        }
        for (key, value) in kwargs {
            body.entry(key).or_insert(value);
        }
        Value::Object(body)
    }

    fn build_responses_body(&self, request: &TransportCallRequest) -> Value {
        let (instructions, input_items) =
            LLMCore::split_messages_for_responses(&request.messages_payload);

        let responses_kwargs =
            LLMCore::with_responses_reasoning(&request.kwargs, request.reasoning_effort.as_ref());
        let final_kwargs =
            LLMCore::decide_responses_kwargs(request.max_tokens, &responses_kwargs, true);

        let mut body = serde_json::Map::new();
        body.insert("model".to_owned(), Value::String(request.model_id.clone()));
        body.insert("input".to_owned(), Value::Array(input_items));
        body.insert(
            "instructions".to_owned(),
            Value::String(instructions.unwrap_or_default()),
        );
        if request.stream {
            body.insert("stream".to_owned(), Value::Bool(true));
        }
        if let Some(ref tools) = request.tools_payload
            && let Some(converted) = LLMCore::convert_tools_for_responses(Some(tools))
        {
            body.insert("tools".to_owned(), Value::Array(converted));
        }
        for (key, value) in final_kwargs {
            body.entry(key).or_insert(value);
        }

        if let Some(ref base) = request.api_base
            && base.contains("chatgpt.com")
        {
            body.insert("store".to_owned(), Value::Bool(false));
            body.insert("stream".to_owned(), Value::Bool(true));
            body.remove("max_output_tokens");
        }

        Value::Object(body)
    }
}
