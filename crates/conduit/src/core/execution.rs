//! Core execution utilities for Conduit.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};

use super::api_format::ApiFormat;
use super::client_registry::ClientRegistry;
use super::errors::{ConduitError, ErrorKind};
use super::provider_runtime::ProviderRuntime;
use super::request_adapters::normalize_responses_kwargs;
use super::results::ErrorPayload;
use super::tool_calls::{
    normalize_message_tool_calls, normalize_tool_calls, tool_call_arguments_string, tool_call_id,
    tool_call_name,
};
use crate::clients::parsing::TransportKind;
use crate::providers;

/// What to do after one failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptDecision {
    RetrySameModel,
    TryNextModel,
}

/// Result of classifying and deciding how to handle one exception.
#[derive(Debug, Clone)]
pub struct AttemptOutcome {
    pub error: ConduitError,
    pub decision: AttemptDecision,
}

/// Wrapper that pairs a transport kind with the raw response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportResponse {
    pub transport: TransportKind,
    pub payload: Value,
}

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

/// Shared LLM execution utilities (provider resolution, retries, client cache).
pub struct LLMCore {
    provider: String,
    model: String,
    fallback_models: Vec<String>,
    max_retries: u32,
    api_key: ApiKeyConfig,
    api_base: ApiBaseConfig,
    client_registry: ClientRegistry,
    api_format: ApiFormat,
    verbose: u32,
    #[allow(clippy::type_complexity)]
    error_classifier: Option<Box<dyn Fn(&ConduitError) -> Option<ErrorKind> + Send + Sync>>,
}

/// How API keys are configured.
#[derive(Debug, Clone)]
pub enum ApiKeyConfig {
    None,
    Single(String),
    PerProvider(HashMap<String, String>),
}

/// How API bases are configured.
#[derive(Debug, Clone)]
pub enum ApiBaseConfig {
    None,
    Single(String),
    PerProvider(HashMap<String, String>),
}

/// Classify an error by scanning the message text for common patterns.
///
/// Returns `None` when no pattern matches, allowing the caller to fall
/// through to other classification strategies.
pub fn classify_by_text_signature(message: &str) -> Option<ErrorKind> {
    let lower = message.to_lowercase();

    // Authentication / configuration errors
    if lower.contains("auth")
        || lower.contains("unauthorized")
        || lower.contains("api key")
        || lower.contains("invalid key")
    {
        return Some(ErrorKind::Config);
    }

    // Rate-limit / quota errors
    if lower.contains("rate limit") || lower.contains("429") || lower.contains("quota") {
        return Some(ErrorKind::Temporary);
    }

    // Not-found errors
    if lower.contains("not found") || lower.contains("404") {
        return Some(ErrorKind::NotFound);
    }

    // Timeout errors
    if lower.contains("timeout") || lower.contains("timed out") {
        return Some(ErrorKind::Temporary);
    }

    // Server errors
    if lower.contains("server error")
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
    {
        return Some(ErrorKind::Temporary);
    }

    None
}

/// Normalize messages to ensure protocol compliance before sending to any LLM API.
///
/// - Removes orphan tool_use blocks (no matching tool_result follows)
/// - Removes orphan tool_result messages (no matching tool_use precedes)
pub fn normalize_messages_for_api(messages: Vec<Value>, transport: TransportKind) -> Vec<Value> {
    let normalized_messages: Vec<Value> = messages
        .into_iter()
        .map(|message| normalize_message_tool_calls(&message))
        .collect();
    let result = prune_orphan_tool_messages(normalized_messages);

    // Anthropic-specific role merging is intentionally deferred to
    // `build_messages_body`, where tool results have already been converted into
    // Anthropic content blocks. Doing it earlier on the generic message shape
    // can collapse multiple `role=tool` messages and drop call IDs.
    if transport == TransportKind::Messages {
        return result;
    }

    result
}

/// Remove orphan tool_use assistant messages and orphan tool_result messages.
///
/// A tool_result is orphan when no assistant message has a matching tool_call id.
/// An assistant message with tool_calls is orphan when any of its calls lack a
/// matching tool_result.
fn prune_orphan_tool_messages(messages: Vec<Value>) -> Vec<Value> {
    // Collect all tool_call IDs from assistant messages
    let mut tool_call_ids: HashSet<String> = HashSet::new();
    for msg in &messages {
        if let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
            for call in calls {
                if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                    tool_call_ids.insert(id.to_owned());
                }
            }
        }
    }

    // Collect all tool_result IDs
    let mut tool_result_ids: HashSet<String> = HashSet::new();
    for msg in &messages {
        if msg.get("role").and_then(|r| r.as_str()) == Some("tool")
            && let Some(id) = msg.get("tool_call_id").and_then(|v| v.as_str())
        {
            tool_result_ids.insert(id.to_owned());
        }
    }

    // Filter: keep messages that are not orphans
    let mut filtered = Vec::new();
    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

        if role == "tool" {
            // Keep tool result only if its call_id has a matching tool_use
            let call_id = msg
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if call_id.is_empty() || !tool_call_ids.contains(call_id) {
                continue; // Drop orphan tool result
            }
        }

        if role == "assistant"
            && let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array())
        {
            // Check if ALL tool_calls have matching results
            let all_have_results = calls.iter().all(|call| {
                call.get("id")
                    .and_then(|v| v.as_str())
                    .map(|id| tool_result_ids.contains(id))
                    .unwrap_or(false)
            });
            if !all_have_results && !calls.is_empty() {
                // Drop assistant message with orphan tool_calls
                continue;
            }
        }

        filtered.push(msg);
    }

    filtered
}

/// Enforce Anthropic-specific message ordering rules.
///
/// - Merges consecutive same-role messages (except system).
/// - Inserts a synthetic "user" message at the start if needed.
/// - Appends a synthetic "user" message at the end if the last message is "assistant".
#[cfg(test)]
fn enforce_anthropic_message_rules(messages: Vec<Value>) -> Vec<Value> {
    if messages.is_empty() {
        return messages;
    }

    let mut result: Vec<Value> = Vec::new();

    for msg in messages {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_owned();

        // Skip system messages in this pass (they go separately in Anthropic API)
        if role == "system" {
            result.push(msg);
            continue;
        }

        // Merge consecutive same-role messages
        if let Some(last) = result.last() {
            let last_role = last.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if last_role == role && role != "system" {
                // Merge: append content to previous message
                let prev_content = last.get("content").and_then(|c| c.as_str()).unwrap_or("");
                let new_content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                if !new_content.is_empty() {
                    let merged = format!("{prev_content}\n\n{new_content}");
                    if let Some(last_mut) = result.last_mut() {
                        if let Some(obj) = last_mut.as_object_mut() {
                            obj.insert("content".to_owned(), Value::String(merged));
                        }
                    }
                }
                continue;
            }
        }

        result.push(msg);
    }

    // Ensure first non-system message is "user"
    let first_non_system = result
        .iter()
        .position(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"));
    if let Some(idx) = first_non_system {
        if result[idx].get("role").and_then(|r| r.as_str()) != Some("user") {
            result.insert(
                idx,
                serde_json::json!({"role": "user", "content": "Continue."}),
            );
        }
    }

    // Ensure last message is "user" (but NOT if it ends with tool results, which is valid)
    if let Some(last) = result.last() {
        let last_role = last.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if last_role == "assistant" {
            result.push(serde_json::json!({"role": "user", "content": "Continue."}));
        }
    }

    result
}

impl LLMCore {
    /// Create a new `LLMCore` instance.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: String,
        model: String,
        fallback_models: Vec<String>,
        max_retries: u32,
        api_key: ApiKeyConfig,
        api_base: ApiBaseConfig,
        api_format: impl Into<ApiFormat>,
        verbose: u32,
    ) -> Self {
        Self {
            provider,
            model,
            fallback_models,
            max_retries,
            api_key,
            api_base,
            client_registry: ClientRegistry::new(),
            api_format: api_format.into(),
            verbose,
            error_classifier: None,
        }
    }

    /// Set a custom error classifier.
    pub fn with_error_classifier(
        mut self,
        classifier: impl Fn(&ConduitError) -> Option<ErrorKind> + Send + Sync + 'static,
    ) -> Self {
        self.error_classifier = Some(Box::new(classifier));
        self
    }

    /// The primary provider name.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The primary model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The fallback model list.
    pub fn fallback_models(&self) -> &[String] {
        &self.fallback_models
    }

    /// Configured maximum retries.
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// The API key configuration.
    pub fn api_key_config(&self) -> &ApiKeyConfig {
        &self.api_key
    }

    /// The API base configuration.
    pub fn api_base_config(&self) -> &ApiBaseConfig {
        &self.api_base
    }

    /// The configured API format / transport kind.
    pub fn api_format(&self) -> ApiFormat {
        self.api_format
    }

    /// The verbosity level.
    pub fn verbose(&self) -> u32 {
        self.verbose
    }

    /// Total number of attempts = 1 + max_retries, minimum 1.
    pub fn max_attempts(&self) -> u32 {
        1u32.max(1 + self.max_retries)
    }

    /// Resolve a `"provider:model"` string into `(provider, model)`.
    pub fn resolve_model_provider(
        model: &str,
        provider: Option<&str>,
    ) -> Result<(String, String), ConduitError> {
        if let Some(p) = provider {
            if model.contains(':') {
                return Err(ConduitError::new(
                    ErrorKind::InvalidInput,
                    "When provider is specified, model must not include a provider prefix.",
                ));
            }
            return Ok((p.to_owned(), model.to_owned()));
        }
        if !model.contains(':') {
            return Err(ConduitError::new(
                ErrorKind::InvalidInput,
                "Model must be in 'provider:model' format.",
            ));
        }
        let (prov, mdl) = model.split_once(':').unwrap();
        if prov.is_empty() || mdl.is_empty() {
            return Err(ConduitError::new(
                ErrorKind::InvalidInput,
                "Model must be in 'provider:model' format.",
            ));
        }
        Ok((prov.to_owned(), mdl.to_owned()))
    }

    /// Resolve a fallback model string into `(provider, model)`.
    pub fn resolve_fallback(&self, model: &str) -> Result<(String, String), ConduitError> {
        if model.contains(':') {
            let (prov, mdl) = model.split_once(':').unwrap();
            if prov.is_empty() || mdl.is_empty() {
                return Err(ConduitError::new(
                    ErrorKind::InvalidInput,
                    "Fallback models must be in 'provider:model' format.",
                ));
            }
            return Ok((prov.to_owned(), mdl.to_owned()));
        }
        if !self.provider.is_empty() {
            return Ok((self.provider.clone(), model.to_owned()));
        }
        Err(ConduitError::new(
            ErrorKind::InvalidInput,
            "Fallback models must include provider or LLM must be initialized with a provider.",
        ))
    }

    /// Build the ordered list of (provider, model) candidates to try.
    pub fn model_candidates(
        &self,
        override_model: Option<&str>,
        override_provider: Option<&str>,
    ) -> Result<Vec<(String, String)>, ConduitError> {
        if let Some(om) = override_model {
            let (p, m) = Self::resolve_model_provider(om, override_provider)?;
            return Ok(vec![(p, m)]);
        }
        let mut candidates = vec![(self.provider.clone(), self.model.clone())];
        for fallback in &self.fallback_models {
            candidates.push(self.resolve_fallback(fallback)?);
        }
        Ok(candidates)
    }

    /// Resolve the API key for a given provider.
    pub fn resolve_api_key(&self, provider: &str) -> Option<String> {
        match &self.api_key {
            ApiKeyConfig::None => None,
            ApiKeyConfig::Single(key) => Some(key.clone()),
            ApiKeyConfig::PerProvider(map) => map.get(provider).cloned(),
        }
    }

    /// Resolve the API base URL for a given provider.
    pub fn resolve_api_base(&self, provider: &str) -> Option<String> {
        match &self.api_base {
            ApiBaseConfig::None => None,
            ApiBaseConfig::Single(base) => Some(base.clone()),
            ApiBaseConfig::PerProvider(map) => map.get(provider).cloned(),
        }
    }

    /// Get or create an HTTP client for the given provider.
    pub fn get_client(&mut self, provider: &str) -> Arc<Client> {
        let api_key = self.resolve_api_key(provider);
        let api_base = self.resolve_api_base(provider);
        self.client_registry
            .get_or_create(provider, api_key.as_deref(), api_base.as_deref())
    }

    /// Log an error at the warning level if verbose mode is enabled.
    pub fn log_error(&self, error: &ConduitError, provider: &str, model: &str, attempt: u32) {
        if self.verbose == 0 {
            return;
        }
        let prefix = format!(
            "[{}:{}] attempt {}/{}",
            provider,
            model,
            attempt + 1,
            self.max_attempts()
        );
        if let Some(ref cause) = error.cause {
            warn!("{} failed: {} (cause={:?})", prefix, error, cause);
        } else {
            warn!("{} failed: {}", prefix, error);
        }
    }

    /// Classify an error into an `ErrorKind`.
    ///
    /// Resolution order:
    /// 1. Custom classifier (if set)
    /// 2. Text-signature heuristic on the error message
    /// 3. The error's own `kind` field
    pub fn classify_error(&self, error: &ConduitError) -> ErrorKind {
        if let Some(ref classifier) = self.error_classifier
            && let Some(kind) = classifier(error)
        {
            return kind;
        }
        if let Some(kind) = classify_by_text_signature(&error.message) {
            return kind;
        }
        error.kind
    }

    /// Classify an HTTP status code into an `ErrorKind`.
    pub fn classify_http_status(status: u16) -> Option<ErrorKind> {
        match status {
            401 | 403 => Some(ErrorKind::Config),
            400 | 404 | 413 | 422 => Some(ErrorKind::InvalidInput),
            408 | 409 | 425 | 429 => Some(ErrorKind::Temporary),
            s if (500..600).contains(&s) => Some(ErrorKind::Provider),
            _ => None,
        }
    }

    /// Whether the error kind should trigger a retry.
    pub fn should_retry(kind: ErrorKind) -> bool {
        matches!(kind, ErrorKind::Temporary | ErrorKind::Provider)
    }

    /// Wrap a raw error message into a `ConduitError` with provider/model context.
    pub fn wrap_error(
        &self,
        kind: ErrorKind,
        provider: &str,
        model: &str,
        message: &str,
    ) -> ConduitError {
        ConduitError::new(kind, format!("{}:{}: {}", provider, model, message))
    }

    /// Handle a single failed attempt and decide what to do next.
    pub fn handle_attempt_error(
        &self,
        error: ConduitError,
        provider_name: &str,
        model_id: &str,
        attempt: u32,
    ) -> AttemptOutcome {
        let kind = self.classify_error(&error);
        self.log_error(&error, provider_name, model_id, attempt);
        let can_retry = Self::should_retry(kind) && (attempt + 1) < self.max_attempts();
        let decision = if can_retry {
            AttemptDecision::RetrySameModel
        } else {
            AttemptDecision::TryNextModel
        };
        AttemptOutcome { error, decision }
    }

    /// Build an `ErrorPayload` with populated details from retry context.
    ///
    /// The `details` object includes `provider`, `model`, `attempt`,
    /// `max_attempts`, and optionally `http_status`.
    pub fn build_error_payload(
        &self,
        error: &ConduitError,
        provider_name: &str,
        model_id: &str,
        attempt: u32,
        http_status: Option<u16>,
    ) -> ErrorPayload {
        let mut details = serde_json::json!({
            "provider": provider_name,
            "model": model_id,
            "attempt": attempt + 1,
            "max_attempts": self.max_attempts(),
        });
        if let Some(status) = http_status {
            details
                .as_object_mut()
                .unwrap()
                .insert("http_status".to_owned(), Value::Number(status.into()));
        }
        ErrorPayload::new(error.kind, &error.message).with_details(details)
    }

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
        ProviderRuntime::new(provider_name, model_id, None, None, self.api_format)
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
    pub fn build_completion_body(request: &TransportCallRequest, provider_name: &str) -> Value {
        let mut adapter_request = request.clone();
        adapter_request.provider_name = provider_name.to_owned();
        providers::adapter_for_transport(TransportKind::Completion)
            .build_request_body(&adapter_request, TransportKind::Completion)
    }

    /// Build the JSON body for an Anthropic Messages-format request.
    pub fn build_messages_body(request: &TransportCallRequest) -> Value {
        providers::adapter_for_transport(TransportKind::Messages)
            .build_request_body(request, TransportKind::Messages)
    }

    /// Build the JSON body for a responses-format request.
    pub fn build_responses_body(request: &TransportCallRequest) -> Value {
        providers::adapter_for_transport(TransportKind::Responses)
            .build_request_body(request, TransportKind::Responses)
    }

    /// Collect an SSE streaming response into a single JSON value.
    ///
    /// For Responses format, looks for a `response.completed` event and extracts
    /// the `response` field. For Completion format, assembles content from
    /// `delta.content` chunks.
    async fn collect_sse_response(
        resp: reqwest::Response,
        transport: TransportKind,
    ) -> Result<Value, ConduitError> {
        use futures::StreamExt;

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                ConduitError::new(ErrorKind::Provider, format!("SSE stream error: {e}"))
            })?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
        }

        info!(
            target: "eli_trace",
            transport = ?transport,
            raw_sse = ?buffer,
            "llm.raw_sse_response"
        );

        // Parse SSE events from the buffer.
        match transport {
            TransportKind::Messages => {
                // Anthropic Messages streaming: assemble from content_block_delta events.
                // Look for message_stop event and assemble content blocks.
                let mut content = String::new();
                let mut tool_use_blocks: Vec<Value> = Vec::new();
                let mut current_tool: Option<serde_json::Map<String, Value>> = None;
                let mut tool_args_buffer = String::new();
                let mut usage: Option<Value> = None;

                for line in buffer.lines() {
                    let line = line.trim();
                    if let Some(data) = line.strip_prefix("data: ")
                        && let Ok(event) = serde_json::from_str::<Value>(data)
                    {
                        let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match event_type {
                            "content_block_start" => {
                                if let Some(block) = event.get("content_block") {
                                    let block_type =
                                        block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    if block_type == "tool_use" {
                                        let mut tool = serde_json::Map::new();
                                        if let Some(id) = block.get("id") {
                                            tool.insert("id".to_owned(), id.clone());
                                        }
                                        if let Some(name) = block.get("name") {
                                            tool.insert("name".to_owned(), name.clone());
                                        }
                                        tool_args_buffer.clear();
                                        current_tool = Some(tool);
                                    }
                                }
                            }
                            "content_block_delta" => {
                                if let Some(delta) = event.get("delta") {
                                    let delta_type =
                                        delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    if delta_type == "text_delta" {
                                        if let Some(text) =
                                            delta.get("text").and_then(|t| t.as_str())
                                        {
                                            content.push_str(text);
                                        }
                                    } else if delta_type == "input_json_delta"
                                        && let Some(partial) =
                                            delta.get("partial_json").and_then(|p| p.as_str())
                                    {
                                        tool_args_buffer.push_str(partial);
                                    }
                                }
                            }
                            "content_block_stop" => {
                                if let Some(mut tool) = current_tool.take() {
                                    let input: Value = serde_json::from_str(&tool_args_buffer)
                                        .unwrap_or(serde_json::json!({}));
                                    tool.insert("input".to_owned(), input);
                                    tool.insert(
                                        "type".to_owned(),
                                        Value::String("tool_use".to_owned()),
                                    );
                                    tool_use_blocks.push(Value::Object(tool));
                                    tool_args_buffer.clear();
                                }
                            }
                            "message_delta" => {
                                if let Some(u) = event.get("usage") {
                                    usage = Some(u.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Build an Anthropic Messages response object.
                let mut content_blocks: Vec<Value> = Vec::new();
                if !content.is_empty() {
                    content_blocks.push(serde_json::json!({"type": "text", "text": content}));
                }
                content_blocks.extend(tool_use_blocks);

                let mut result = serde_json::json!({
                    "role": "assistant",
                    "content": content_blocks
                });
                if let Some(u) = usage {
                    result
                        .as_object_mut()
                        .unwrap()
                        .insert("usage".to_owned(), u);
                }
                Ok(result)
            }
            TransportKind::Responses => {
                // Look for "response.completed" event which has the full response.
                for line in buffer.lines() {
                    let line = line.trim();
                    if let Some(data) = line.strip_prefix("data: ")
                        && let Ok(event) = serde_json::from_str::<Value>(data)
                        && event.get("type").and_then(|t| t.as_str()) == Some("response.completed")
                        && let Some(response) = event.get("response")
                    {
                        return Ok(response.clone());
                    }
                }
                Err(ConduitError::new(
                    ErrorKind::Provider,
                    "SSE stream ended without response.completed event",
                ))
            }
            _ => {
                // Completion format: assemble content from delta chunks.
                let mut content = String::new();
                let mut tool_calls: Vec<Value> = Vec::new();

                for line in buffer.lines() {
                    let line = line.trim();
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            break;
                        }
                        if let Ok(event) = serde_json::from_str::<Value>(data)
                            && let Some(choices) = event.get("choices").and_then(|c| c.as_array())
                        {
                            for choice in choices {
                                if let Some(delta) = choice.get("delta") {
                                    if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                                        content.push_str(c);
                                    }
                                    if let Some(tc) =
                                        delta.get("tool_calls").and_then(|t| t.as_array())
                                    {
                                        tool_calls.extend(tc.iter().cloned());
                                    }
                                }
                            }
                        }
                    }
                }

                let mut result = serde_json::json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": content
                        }
                    }]
                });
                if !tool_calls.is_empty() {
                    result["choices"][0]["message"]["tool_calls"] = Value::Array(tool_calls);
                }
                Ok(result)
            }
        }
    }

    /// Execute a synchronous (non-streaming) chat call with retry logic.
    ///
    /// Iterates over model candidates, retrying on transient errors.
    /// The `on_response` callback receives `(TransportResponse, provider, model, attempt)`
    /// and should return `Ok(T)` on success or `Err(None)` to signal a retry.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_chat<T, F>(
        &mut self,
        messages_payload: Vec<Value>,
        tools_payload: Option<Vec<Value>>,
        model: Option<&str>,
        provider: Option<&str>,
        max_tokens: Option<u32>,
        stream: bool,
        reasoning_effort: Option<Value>,
        kwargs: serde_json::Map<String, Value>,
        on_response: F,
    ) -> Result<T, ConduitError>
    where
        F: Fn(TransportResponse, &str, &str, u32) -> Result<T, Option<ConduitError>>,
    {
        let candidates = self.model_candidates(model, provider)?;
        let mut last_error: Option<ConduitError> = None;
        let max_attempts = self.max_attempts();

        for (provider_name, model_id) in &candidates {
            let client = self.get_client(provider_name);
            let api_base = self.resolve_api_base(provider_name);
            let api_key = self.resolve_api_key(provider_name);
            let runtime = ProviderRuntime::new(
                provider_name,
                model_id,
                api_key.as_deref(),
                api_base.as_deref(),
                self.api_format,
            );

            for attempt in 0..max_attempts {
                let transport = match runtime.selected_transport(
                    tools_payload.as_deref(),
                    false, // supports_responses -- caller can override
                    None,
                ) {
                    Ok(t) => t,
                    Err(e) => {
                        last_error = Some(e);
                        break;
                    }
                };

                let resolved_api_base = runtime.resolved_api_base();

                // Normalize messages for protocol compliance
                let normalized_messages =
                    normalize_messages_for_api(messages_payload.clone(), transport);

                let request = TransportCallRequest {
                    client: Arc::clone(&client),
                    provider_name: provider_name.clone(),
                    model_id: model_id.clone(),
                    api_base: Some(resolved_api_base.clone()),
                    messages_payload: normalized_messages,
                    tools_payload: tools_payload.clone(),
                    max_tokens,
                    stream,
                    reasoning_effort: reasoning_effort.clone(),
                    kwargs: kwargs.clone(),
                    is_anthropic_oauth: runtime.is_anthropic_oauth(),
                };

                let url = Self::build_request_url(&resolved_api_base, transport);
                let body = match transport {
                    TransportKind::Responses => Self::build_responses_body(&request),
                    TransportKind::Messages => Self::build_messages_body(&request),
                    TransportKind::Completion => {
                        Self::build_completion_body(&request, provider_name)
                    }
                };

                info!(
                    target: "eli_trace",
                    provider = %provider_name,
                    model = %model_id,
                    transport = ?transport,
                    stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false),
                    request_body = %body,
                    "llm.request"
                );

                let http_result = client.post(&url).json(&body).send().await;

                match http_result {
                    Ok(resp) => {
                        let status = resp.status();
                        if !status.is_success() {
                            let error_body = resp.text().await.unwrap_or_default();
                            let kind = Self::classify_http_status(status.as_u16())
                                .unwrap_or(ErrorKind::Provider);
                            let error = ConduitError::new(
                                kind,
                                format!(
                                    "{}:{}: HTTP {} - {}",
                                    provider_name, model_id, status, error_body
                                ),
                            );
                            let outcome =
                                self.handle_attempt_error(error, provider_name, model_id, attempt);
                            last_error = Some(outcome.error);
                            if outcome.decision == AttemptDecision::RetrySameModel {
                                continue;
                            }
                            break;
                        }

                        // If the body forced stream=true (e.g. Codex backend)
                        // but the caller asked for non-streaming, collect SSE
                        // chunks and extract the final response.
                        let body_forced_stream = !stream
                            && body
                                .get("stream")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);

                        let payload: Value = if body_forced_stream {
                            match Self::collect_sse_response(resp, transport).await {
                                Ok(v) => v,
                                Err(e) => {
                                    let outcome = self.handle_attempt_error(
                                        e,
                                        provider_name,
                                        model_id,
                                        attempt,
                                    );
                                    last_error = Some(outcome.error);
                                    if outcome.decision == AttemptDecision::RetrySameModel {
                                        continue;
                                    }
                                    break;
                                }
                            }
                        } else {
                            match resp.json().await {
                                Ok(v) => v,
                                Err(e) => {
                                    let error = ConduitError::new(
                                        ErrorKind::Provider,
                                        format!(
                                            "{}:{}: failed to parse response: {}",
                                            provider_name, model_id, e
                                        ),
                                    );
                                    let outcome = self.handle_attempt_error(
                                        error,
                                        provider_name,
                                        model_id,
                                        attempt,
                                    );
                                    last_error = Some(outcome.error);
                                    if outcome.decision == AttemptDecision::RetrySameModel {
                                        continue;
                                    }
                                    break;
                                }
                            }
                        };

                        info!(
                            target: "eli_trace",
                            provider = %provider_name,
                            model = %model_id,
                            transport = ?transport,
                            response_payload = %payload,
                            "llm.response"
                        );

                        let transport_response = TransportResponse { transport, payload };

                        match on_response(transport_response, provider_name, model_id, attempt) {
                            Ok(result) => return Ok(result),
                            Err(Some(e)) => {
                                last_error = Some(e);
                                break;
                            }
                            Err(None) => {
                                // Signal to retry
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        let kind = if e.is_timeout() {
                            ErrorKind::Temporary
                        } else if e.is_connect() {
                            ErrorKind::Provider
                        } else {
                            ErrorKind::Unknown
                        };
                        let error = ConduitError::new(
                            kind,
                            format!("{}:{}: {}", provider_name, model_id, e),
                        );
                        let outcome =
                            self.handle_attempt_error(error, provider_name, model_id, attempt);
                        last_error = Some(outcome.error);
                        if outcome.decision == AttemptDecision::RetrySameModel {
                            continue;
                        }
                        break;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            ConduitError::new(ErrorKind::Temporary, "LLM call failed after retries")
        }))
    }

    /// Execute a streaming chat call.
    ///
    /// Returns the raw `reqwest::Response` for the caller to consume as an SSE stream.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_chat_stream(
        &mut self,
        messages_payload: Vec<Value>,
        tools_payload: Option<Vec<Value>>,
        model: Option<&str>,
        provider: Option<&str>,
        max_tokens: Option<u32>,
        reasoning_effort: Option<Value>,
        kwargs: serde_json::Map<String, Value>,
    ) -> Result<(reqwest::Response, TransportKind, String, String), ConduitError> {
        let candidates = self.model_candidates(model, provider)?;
        let mut last_error: Option<ConduitError> = None;
        let max_attempts = self.max_attempts();

        for (provider_name, model_id) in &candidates {
            let client = self.get_client(provider_name);
            let api_base = self.resolve_api_base(provider_name);
            let api_key = self.resolve_api_key(provider_name);
            let runtime = ProviderRuntime::new(
                provider_name,
                model_id,
                api_key.as_deref(),
                api_base.as_deref(),
                self.api_format,
            );

            for attempt in 0..max_attempts {
                let transport =
                    match runtime.selected_transport(tools_payload.as_deref(), false, None) {
                        Ok(t) => t,
                        Err(e) => {
                            last_error = Some(e);
                            break;
                        }
                    };

                let resolved_api_base = runtime.resolved_api_base();

                // Normalize messages for protocol compliance
                let normalized_messages =
                    normalize_messages_for_api(messages_payload.clone(), transport);

                let request = TransportCallRequest {
                    client: Arc::clone(&client),
                    provider_name: provider_name.clone(),
                    model_id: model_id.clone(),
                    api_base: Some(resolved_api_base.clone()),
                    messages_payload: normalized_messages,
                    tools_payload: tools_payload.clone(),
                    max_tokens,
                    stream: true,
                    reasoning_effort: reasoning_effort.clone(),
                    kwargs: kwargs.clone(),
                    is_anthropic_oauth: runtime.is_anthropic_oauth(),
                };

                let url = Self::build_request_url(&resolved_api_base, transport);
                let body = match transport {
                    TransportKind::Responses => Self::build_responses_body(&request),
                    TransportKind::Messages => Self::build_messages_body(&request),
                    TransportKind::Completion => {
                        Self::build_completion_body(&request, provider_name)
                    }
                };

                let http_result = client.post(&url).json(&body).send().await;

                match http_result {
                    Ok(resp) => {
                        let status = resp.status();
                        if !status.is_success() {
                            let error_body = resp.text().await.unwrap_or_default();
                            let kind = Self::classify_http_status(status.as_u16())
                                .unwrap_or(ErrorKind::Provider);
                            let error = ConduitError::new(
                                kind,
                                format!(
                                    "{}:{}: HTTP {} - {}",
                                    provider_name, model_id, status, error_body
                                ),
                            );
                            let outcome =
                                self.handle_attempt_error(error, provider_name, model_id, attempt);
                            last_error = Some(outcome.error);
                            if outcome.decision == AttemptDecision::RetrySameModel {
                                continue;
                            }
                            break;
                        }

                        return Ok((resp, transport, provider_name.clone(), model_id.clone()));
                    }
                    Err(e) => {
                        let kind = if e.is_timeout() {
                            ErrorKind::Temporary
                        } else if e.is_connect() {
                            ErrorKind::Provider
                        } else {
                            ErrorKind::Unknown
                        };
                        let error = ConduitError::new(
                            kind,
                            format!("{}:{}: {}", provider_name, model_id, e),
                        );
                        let outcome =
                            self.handle_attempt_error(error, provider_name, model_id, attempt);
                        last_error = Some(outcome.error);
                        if outcome.decision == AttemptDecision::RetrySameModel {
                            continue;
                        }
                        break;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            ConduitError::new(
                ErrorKind::Temporary,
                "LLM streaming call failed after retries",
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::anthropic_messages;
    use serde_json::json;

    #[test]
    fn test_resolve_model_provider() {
        let (p, m) = LLMCore::resolve_model_provider("openai:gpt-4", None).unwrap();
        assert_eq!(p, "openai");
        assert_eq!(m, "gpt-4");
    }

    #[test]
    fn test_resolve_model_provider_with_override() {
        let (p, m) = LLMCore::resolve_model_provider("gpt-4", Some("openai")).unwrap();
        assert_eq!(p, "openai");
        assert_eq!(m, "gpt-4");
    }

    #[test]
    fn test_resolve_model_provider_error() {
        let result = LLMCore::resolve_model_provider("gpt-4", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_classify_http_status() {
        assert_eq!(LLMCore::classify_http_status(401), Some(ErrorKind::Config));
        assert_eq!(
            LLMCore::classify_http_status(429),
            Some(ErrorKind::Temporary)
        );
        assert_eq!(
            LLMCore::classify_http_status(500),
            Some(ErrorKind::Provider)
        );
        assert_eq!(LLMCore::classify_http_status(200), None);
    }

    #[test]
    fn test_split_messages_for_responses() {
        let messages = vec![
            json!({"role": "system", "content": "You are helpful."}),
            json!({"role": "user", "content": "Hello"}),
        ];
        let (instructions, items) = LLMCore::split_messages_for_responses(&messages);
        assert_eq!(instructions.unwrap(), "You are helpful.");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["role"], "user");
    }

    #[test]
    fn test_convert_tools_for_responses() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "greet",
                "description": "Say hello",
                "parameters": {"type": "object"}
            }
        })];
        let converted = LLMCore::convert_tools_for_responses(Some(&tools)).unwrap();
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["name"], "greet");
        assert!(converted[0].get("function").is_none());
    }

    #[test]
    fn test_normalize_anthropic_messages_keeps_multiple_tool_results() {
        let messages = vec![
            json!({
                "role": "user",
                "content": "find latest papers"
            }),
            json!({
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "bash", "input": {"cmd": "pwd"}},
                    {"type": "tool_use", "id": "toolu_2", "name": "skill", "input": {"name": "active-research"}}
                ]
            }),
            json!({
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok-1"}
                ]
            }),
            json!({
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_2", "content": "ok-2"}
                ]
            }),
        ];

        let normalized = anthropic_messages::normalize_messages(messages);
        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[0]["role"], "user");
        assert_eq!(normalized[1]["role"], "assistant");
        assert_eq!(normalized[2]["role"], "user");
        let content = normalized[2]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["tool_use_id"], "toolu_1");
        assert_eq!(content[1]["tool_use_id"], "toolu_2");
    }

    #[test]
    fn test_normalize_anthropic_messages_merges_text_with_tool_results() {
        let messages = vec![
            json!({
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok"}
                ]
            }),
            json!({
                "role": "user",
                "content": "continue with a summary"
            }),
        ];

        let normalized = anthropic_messages::normalize_messages(messages);
        assert_eq!(normalized.len(), 1);
        let content = normalized[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "continue with a summary");
    }

    #[test]
    fn test_build_messages_body_keeps_tool_results_immediately_after_tool_use() {
        let request = TransportCallRequest {
            client: Arc::new(reqwest::Client::new()),
            provider_name: "anthropic".to_owned(),
            model_id: "claude-test".to_owned(),
            api_base: None,
            messages_payload: vec![
                json!({"role": "system", "content": "system rules"}),
                json!({"role": "user", "content": "find latest papers"}),
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
            ],
            tools_payload: None,
            max_tokens: Some(512),
            stream: false,
            reasoning_effort: None,
            kwargs: serde_json::Map::new(),
            is_anthropic_oauth: false,
        };

        let body = LLMCore::build_messages_body(&request);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["role"], "user");

        let user_blocks = messages[2]["content"].as_array().unwrap();
        assert_eq!(user_blocks[0]["type"], "tool_result");
        assert_eq!(user_blocks[0]["tool_use_id"], "toolu_1");
        assert_eq!(user_blocks[1]["type"], "tool_result");
        assert_eq!(user_blocks[1]["tool_use_id"], "toolu_2");
        assert_eq!(user_blocks[2]["type"], "text");
        assert_eq!(user_blocks[2]["text"], "继续");
    }

    #[test]
    fn test_max_attempts() {
        let core = LLMCore::new(
            "openai".into(),
            "gpt-4".into(),
            vec![],
            2,
            ApiKeyConfig::None,
            ApiBaseConfig::None,
            TransportKind::Completion,
            0,
        );
        assert_eq!(core.max_attempts(), 3);
    }

    #[test]
    fn test_build_request_url() {
        assert_eq!(
            LLMCore::build_request_url("https://api.openai.com/v1", TransportKind::Completion),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            LLMCore::build_request_url("https://api.openai.com/v1", TransportKind::Responses),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            LLMCore::build_request_url("https://api.anthropic.com/v1", TransportKind::Messages),
            "https://api.anthropic.com/v1/messages"
        );
    }

    // ----- build_error_payload -----

    #[test]
    fn test_build_error_payload_basic() {
        let core = LLMCore::new(
            "openai".into(),
            "gpt-4".into(),
            vec![],
            2,
            ApiKeyConfig::None,
            ApiBaseConfig::None,
            TransportKind::Completion,
            0,
        );
        let error = ConduitError::new(ErrorKind::Provider, "server error");
        let payload = core.build_error_payload(&error, "openai", "gpt-4", 1, None);

        assert_eq!(payload.kind, ErrorKind::Provider);
        assert_eq!(payload.message, "server error");
        let details = payload.details.unwrap();
        assert_eq!(details["provider"], "openai");
        assert_eq!(details["model"], "gpt-4");
        assert_eq!(details["attempt"], 2); // attempt + 1
        assert_eq!(details["max_attempts"], 3); // 1 + max_retries(2)
        assert!(details.get("http_status").is_none());
    }

    #[test]
    fn test_build_error_payload_with_http_status() {
        let core = LLMCore::new(
            "openai".into(),
            "gpt-4".into(),
            vec![],
            3,
            ApiKeyConfig::None,
            ApiBaseConfig::None,
            TransportKind::Completion,
            0,
        );
        let error = ConduitError::new(ErrorKind::Temporary, "rate limited");
        let payload = core.build_error_payload(&error, "openai", "gpt-4", 0, Some(429));

        assert_eq!(payload.kind, ErrorKind::Temporary);
        let details = payload.details.unwrap();
        assert_eq!(details["http_status"], 429);
        assert_eq!(details["attempt"], 1);
        assert_eq!(details["max_attempts"], 4);
    }

    #[test]
    fn test_build_error_payload_different_provider() {
        let core = LLMCore::new(
            "anthropic".into(),
            "claude-3".into(),
            vec![],
            1,
            ApiKeyConfig::None,
            ApiBaseConfig::None,
            TransportKind::Completion,
            0,
        );
        let error = ConduitError::new(ErrorKind::Config, "auth failed");
        let payload = core.build_error_payload(&error, "anthropic", "claude-3", 0, Some(401));

        assert_eq!(payload.kind, ErrorKind::Config);
        let details = payload.details.unwrap();
        assert_eq!(details["provider"], "anthropic");
        assert_eq!(details["model"], "claude-3");
        assert_eq!(details["http_status"], 401);
    }

    // ----- accessor tests -----

    #[test]
    fn test_api_key_config_accessor() {
        let core = LLMCore::new(
            "openai".into(),
            "gpt-4".into(),
            vec![],
            3,
            ApiKeyConfig::Single("my-key".into()),
            ApiBaseConfig::None,
            TransportKind::Completion,
            0,
        );
        match core.api_key_config() {
            ApiKeyConfig::Single(key) => assert_eq!(key, "my-key"),
            _ => panic!("Expected Single key config"),
        }
    }

    #[test]
    fn test_api_base_config_accessor() {
        let core = LLMCore::new(
            "openai".into(),
            "gpt-4".into(),
            vec![],
            3,
            ApiKeyConfig::None,
            ApiBaseConfig::Single("https://custom.api.com".into()),
            TransportKind::Completion,
            0,
        );
        match core.api_base_config() {
            ApiBaseConfig::Single(base) => assert_eq!(base, "https://custom.api.com"),
            _ => panic!("Expected Single base config"),
        }
    }

    #[test]
    fn test_api_format_accessor() {
        let core = LLMCore::new(
            "openai".into(),
            "gpt-4".into(),
            vec![],
            3,
            ApiKeyConfig::None,
            ApiBaseConfig::None,
            TransportKind::Responses,
            0,
        );
        assert_eq!(core.api_format(), ApiFormat::Responses);
    }

    #[test]
    fn test_verbose_accessor() {
        let core = LLMCore::new(
            "openai".into(),
            "gpt-4".into(),
            vec![],
            3,
            ApiKeyConfig::None,
            ApiBaseConfig::None,
            TransportKind::Completion,
            2,
        );
        assert_eq!(core.verbose(), 2);
    }

    // ----- classify_by_text_signature -----

    #[test]
    fn test_classify_auth_errors() {
        assert_eq!(
            classify_by_text_signature("Unauthorized request"),
            Some(ErrorKind::Config)
        );
        assert_eq!(
            classify_by_text_signature("invalid api key provided"),
            Some(ErrorKind::Config)
        );
        assert_eq!(
            classify_by_text_signature("Invalid key format"),
            Some(ErrorKind::Config)
        );
    }

    #[test]
    fn test_classify_rate_limit_errors() {
        assert_eq!(
            classify_by_text_signature("Rate limit exceeded"),
            Some(ErrorKind::Temporary)
        );
        assert_eq!(
            classify_by_text_signature("HTTP 429 Too Many Requests"),
            Some(ErrorKind::Temporary)
        );
        assert_eq!(
            classify_by_text_signature("Quota exceeded for this month"),
            Some(ErrorKind::Temporary)
        );
    }

    #[test]
    fn test_classify_not_found_errors() {
        assert_eq!(
            classify_by_text_signature("Model not found"),
            Some(ErrorKind::NotFound)
        );
        assert_eq!(
            classify_by_text_signature("HTTP 404"),
            Some(ErrorKind::NotFound)
        );
    }

    #[test]
    fn test_classify_timeout_errors() {
        assert_eq!(
            classify_by_text_signature("Request timeout"),
            Some(ErrorKind::Temporary)
        );
        assert_eq!(
            classify_by_text_signature("Connection timed out"),
            Some(ErrorKind::Temporary)
        );
    }

    #[test]
    fn test_classify_server_errors() {
        assert_eq!(
            classify_by_text_signature("Internal server error"),
            Some(ErrorKind::Temporary)
        );
        assert_eq!(
            classify_by_text_signature("HTTP 502 Bad Gateway"),
            Some(ErrorKind::Temporary)
        );
        assert_eq!(
            classify_by_text_signature("HTTP 503 Service Unavailable"),
            Some(ErrorKind::Temporary)
        );
    }

    #[test]
    fn test_classify_unknown_message() {
        assert_eq!(classify_by_text_signature("Something went wrong"), None);
        assert_eq!(classify_by_text_signature(""), None);
    }

    // ----- normalize_messages_for_api -----

    #[test]
    fn test_prune_orphan_tool_result() {
        // tool_result without matching tool_call should be removed
        let messages = vec![
            json!({"role": "user", "content": "hello"}),
            json!({"role": "tool", "tool_call_id": "call_orphan", "content": "result"}),
            json!({"role": "assistant", "content": "hi"}),
        ];
        let result = prune_orphan_tool_messages(messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["role"], "user");
        assert_eq!(result[1]["role"], "assistant");
    }

    #[test]
    fn test_prune_orphan_tool_call() {
        // assistant with tool_calls but no matching tool_result should be removed
        let messages = vec![
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "foo", "arguments": "{}"}}]}),
        ];
        let result = prune_orphan_tool_messages(messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
    }

    #[test]
    fn test_keep_matched_tool_pair() {
        // matched tool_call + tool_result should both be kept
        let messages = vec![
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "foo", "arguments": "{}"}}]}),
            json!({"role": "tool", "tool_call_id": "call_1", "content": "result"}),
            json!({"role": "assistant", "content": "done"}),
        ];
        let result = prune_orphan_tool_messages(messages);
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_anthropic_merge_consecutive_user() {
        let messages = vec![
            json!({"role": "user", "content": "first"}),
            json!({"role": "user", "content": "second"}),
            json!({"role": "assistant", "content": "reply"}),
        ];
        let result = enforce_anthropic_message_rules(messages);
        // merged_user + assistant + trailing "Continue." user
        assert_eq!(result.len(), 3);
        assert_eq!(result[0]["content"], "first\n\nsecond");
        assert_eq!(result[1]["role"], "assistant");
        assert_eq!(result[2]["role"], "user");
        assert_eq!(result[2]["content"], "Continue.");
    }

    #[test]
    fn test_anthropic_insert_user_at_start() {
        let messages = vec![json!({"role": "assistant", "content": "hi"})];
        let result = enforce_anthropic_message_rules(messages);
        assert_eq!(result.len(), 3); // synthetic user + assistant + trailing user
        assert_eq!(result[0]["role"], "user");
        assert_eq!(result[0]["content"], "Continue.");
        assert_eq!(result[1]["role"], "assistant");
        assert_eq!(result[2]["role"], "user");
    }

    #[test]
    fn test_anthropic_append_user_at_end() {
        let messages = vec![
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "reply"}),
        ];
        let result = enforce_anthropic_message_rules(messages);
        assert_eq!(result.len(), 3);
        assert_eq!(result[2]["role"], "user");
        assert_eq!(result[2]["content"], "Continue.");
    }

    #[test]
    fn test_anthropic_no_append_when_user_last() {
        let messages = vec![
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "reply"}),
            json!({"role": "user", "content": "more"}),
        ];
        let result = enforce_anthropic_message_rules(messages);
        assert_eq!(result.len(), 3);
        assert_eq!(result[2]["content"], "more");
    }

    #[test]
    fn test_anthropic_system_preserved() {
        let messages = vec![
            json!({"role": "system", "content": "system prompt"}),
            json!({"role": "user", "content": "hello"}),
        ];
        let result = enforce_anthropic_message_rules(messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["role"], "system");
        assert_eq!(result[1]["role"], "user");
    }

    #[test]
    fn test_normalize_empty() {
        let result = normalize_messages_for_api(vec![], TransportKind::Messages);
        assert!(result.is_empty());
    }

    #[test]
    fn test_normalize_completion_skips_anthropic_rules() {
        // For completion transport, consecutive same-role should NOT be merged
        let messages = vec![
            json!({"role": "user", "content": "first"}),
            json!({"role": "user", "content": "second"}),
        ];
        let result = normalize_messages_for_api(messages, TransportKind::Completion);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_normalize_messages_keeps_multiple_tool_results_for_messages_transport() {
        let messages = vec![
            json!({"role": "user", "content": "hello"}),
            json!({
                "role": "assistant",
                "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "bash", "arguments": "{}"}},
                    {"id": "call_2", "type": "function", "function": {"name": "skill", "arguments": "{}"}}
                ]
            }),
            json!({"role": "tool", "tool_call_id": "call_1", "content": "result-1"}),
            json!({"role": "tool", "tool_call_id": "call_2", "content": "result-2"}),
        ];

        let result = normalize_messages_for_api(messages, TransportKind::Messages);
        assert_eq!(result.len(), 4);
        assert_eq!(result[2]["tool_call_id"], "call_1");
        assert_eq!(result[3]["tool_call_id"], "call_2");
    }

    #[test]
    fn test_normalize_messages_canonicalizes_responses_style_tool_calls() {
        let messages = vec![
            json!({"role": "user", "content": "hello"}),
            json!({
                "role": "assistant",
                "tool_calls": [{
                    "type": "function_call",
                    "call_id": "call_123",
                    "name": "tape_info",
                    "arguments": "{}"
                }]
            }),
            json!({"role": "tool", "tool_call_id": "call_123", "content": "ok"}),
        ];

        let result = normalize_messages_for_api(messages, TransportKind::Messages);

        assert_eq!(result.len(), 3);
        assert_eq!(result[1]["tool_calls"][0]["id"], "call_123");
        assert_eq!(result[1]["tool_calls"][0]["function"]["name"], "tape_info");
    }
}
