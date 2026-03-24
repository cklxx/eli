//! Conduit LLM facade.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use crate::auth::APIKeyResolver;
use crate::clients::parsing::parser_for_transport;
pub use crate::core::api_format::ApiFormat;
use crate::core::errors::{ConduitError, ErrorKind};
use crate::core::execution::{ApiBaseConfig, ApiKeyConfig, LLMCore};
use crate::core::response_parser::TransportResponse;
use crate::core::results::{
    AsyncTextStream, StreamEvent, ToolAutoResult, ToolAutoResultKind, ToolExecution,
};
use crate::core::tool_calls::{normalize_message_tool_calls, normalize_tool_calls};
use crate::tape::entries::TapeEntry;
use crate::tape::{
    AsyncTapeManager, AsyncTapeStore, AsyncTapeStoreAdapter, InMemoryTapeStore, TapeContext,
    TapeManager,
};
use crate::tools::context::ToolContext;
use crate::tools::executor::{ToolCallResponse, ToolExecutor};
use crate::tools::schema::ToolSet;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public constants
// ---------------------------------------------------------------------------

/// Default model when none is specified.
pub const DEFAULT_MODEL: &str = "openai:gpt-4o-mini";

/// Hook to process stream events before they are emitted to the caller.
/// Return `Some(event)` to forward (possibly transformed), or `None` to drop.
pub type StreamEventFilter = Arc<dyn Fn(StreamEvent) -> Option<StreamEvent> + Send + Sync>;

// ---------------------------------------------------------------------------
// ChatRequest
// ---------------------------------------------------------------------------

/// Bundles the parameters shared across chat and tool-calling methods.
///
/// All fields are optional so callers only fill in what they need.
#[derive(Default)]
pub struct ChatRequest<'a> {
    pub prompt: Option<&'a str>,
    pub system_prompt: Option<&'a str>,
    pub model: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub messages: Option<Vec<Value>>,
    pub max_tokens: Option<u32>,
    pub tools: Option<&'a ToolSet>,
    pub tool_context: Option<&'a ToolContext>,
    pub tape: Option<&'a str>,
    pub tape_context: Option<&'a TapeContext>,
}

// ---------------------------------------------------------------------------
// LLMBuilder
// ---------------------------------------------------------------------------

/// Builder for constructing an [`LLM`] instance.
pub struct LLMBuilder {
    model: Option<String>,
    provider: Option<String>,
    fallback_models: Option<Vec<String>>,
    max_retries: Option<u32>,
    api_key: Option<String>,
    api_key_map: Option<HashMap<String, String>>,
    api_key_resolver: Option<APIKeyResolver>,
    api_base: Option<String>,
    api_base_map: Option<HashMap<String, String>>,
    api_format: Option<ApiFormat>,
    verbose: Option<u32>,
    context: Option<TapeContext>,
    tape_store: Option<Box<dyn AsyncTapeStore + Send + Sync>>,
    stream_filter: Option<StreamEventFilter>,
}

impl LLMBuilder {
    /// Create a new builder with all fields unset.
    pub fn new() -> Self {
        Self {
            model: None,
            provider: None,
            fallback_models: None,
            max_retries: None,
            api_key: None,
            api_key_map: None,
            api_key_resolver: None,
            api_base: None,
            api_base_map: None,
            api_format: None,
            verbose: None,
            context: None,
            tape_store: None,
            stream_filter: None,
        }
    }

    /// Set the model (e.g. `"openai:gpt-4o"`).
    pub fn model(mut self, model: &str) -> Self {
        self.model = Some(model.to_owned());
        self
    }

    /// Set the provider explicitly (e.g. `"openai"`).
    pub fn provider(mut self, provider: &str) -> Self {
        self.provider = Some(provider.to_owned());
        self
    }

    /// Set fallback models to try when the primary model fails.
    pub fn fallback_models(mut self, models: Vec<String>) -> Self {
        self.fallback_models = Some(models);
        self
    }

    /// Set the maximum number of retries per model.
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = Some(retries);
        self
    }

    /// Set a single API key used for all providers.
    pub fn api_key(mut self, key: &str) -> Self {
        self.api_key = Some(key.to_owned());
        self
    }

    /// Set per-provider API keys.
    pub fn api_key_map(mut self, map: HashMap<String, String>) -> Self {
        self.api_key_map = Some(map);
        self
    }

    /// Set a resolver function that produces an API key for a provider name.
    /// At build time the resolver is called for the default provider and the
    /// result is stored, avoiding changes to `LLMCore` internals.
    pub fn api_key_resolver(mut self, resolver: APIKeyResolver) -> Self {
        self.api_key_resolver = Some(resolver);
        self
    }

    /// Set the API base URL for all providers.
    pub fn api_base(mut self, base: &str) -> Self {
        self.api_base = Some(base.to_owned());
        self
    }

    /// Set per-provider API base URLs.
    pub fn api_base_map(mut self, map: HashMap<String, String>) -> Self {
        self.api_base_map = Some(map);
        self
    }

    /// Set the API format (completion, responses, or messages).
    pub fn api_format(mut self, format: ApiFormat) -> Self {
        self.api_format = Some(format);
        self
    }

    /// Set verbosity level (0, 1, or 2).
    pub fn verbose(mut self, level: u32) -> Self {
        self.verbose = Some(level);
        self
    }

    /// Set the tape context for conversation history tracking.
    pub fn context(mut self, context: TapeContext) -> Self {
        self.context = Some(context);
        self
    }

    /// Set a custom async tape store instead of the default in-memory store.
    pub fn tape_store(mut self, store: impl AsyncTapeStore + 'static) -> Self {
        self.tape_store = Some(Box::new(store));
        self
    }

    /// Set a stream event filter applied to every event before emission.
    pub fn stream_filter(mut self, filter: StreamEventFilter) -> Self {
        self.stream_filter = Some(filter);
        self
    }

    /// Build the [`LLM`] instance, validating all configuration.
    pub fn build(self) -> Result<LLM, ConduitError> {
        let verbose = self.verbose.unwrap_or(0);
        if verbose > 2 {
            return Err(ConduitError::new(
                ErrorKind::InvalidInput,
                "verbose must be 0, 1, or 2",
            ));
        }

        let max_retries = self.max_retries.unwrap_or(3);
        let model_str = self.model.as_deref().unwrap_or(DEFAULT_MODEL);
        let provider_str = self.provider.as_deref();

        let (resolved_provider, resolved_model) =
            LLMCore::resolve_model_provider(model_str, provider_str)?;

        // Resolve API key: explicit key > resolver > key map > none
        let api_key_config = if let Some(key) = self.api_key {
            ApiKeyConfig::Single(key)
        } else if let Some(resolver) = &self.api_key_resolver {
            // Call the resolver for the default provider at build time.
            if let Some(resolved_key) = resolver(&resolved_provider) {
                ApiKeyConfig::Single(resolved_key)
            } else if let Some(map) = self.api_key_map {
                ApiKeyConfig::PerProvider(map)
            } else {
                ApiKeyConfig::None
            }
        } else if let Some(map) = self.api_key_map {
            ApiKeyConfig::PerProvider(map)
        } else {
            ApiKeyConfig::None
        };

        let api_base_config = match (self.api_base, self.api_base_map) {
            (Some(base), _) => ApiBaseConfig::Single(base),
            (None, Some(map)) => ApiBaseConfig::PerProvider(map),
            (None, None) => ApiBaseConfig::None,
        };

        let api_format = self.api_format.unwrap_or_default();

        let core = LLMCore::new(
            resolved_provider,
            resolved_model,
            self.fallback_models.unwrap_or_default(),
            max_retries,
            api_key_config,
            api_base_config,
            api_format,
            verbose,
        );

        let context = self.context;

        let (tape, async_tape) = if let Some(custom_store) = self.tape_store {
            let tape = TapeManager::new(None, context.clone());
            let async_tape = AsyncTapeManager::new(Some(custom_store), context);
            (tape, async_tape)
        } else {
            let shared_tape_store = InMemoryTapeStore::new();
            let async_store = AsyncTapeStoreAdapter::new(shared_tape_store.clone());
            let tape = TapeManager::new(Some(Box::new(shared_tape_store)), context.clone());
            let async_tape = AsyncTapeManager::new(Some(Box::new(async_store)), context);
            (tape, async_tape)
        };

        Ok(LLM {
            core,
            tool_executor: ToolExecutor::new(),
            tape,
            async_tape,
            stream_filter: self.stream_filter,
        })
    }
}

impl Default for LLMBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// LLM (public facade)
// ---------------------------------------------------------------------------

/// Developer-first LLM client powered by any-llm.
pub struct LLM {
    core: LLMCore,
    tool_executor: ToolExecutor,
    #[allow(dead_code)]
    tape: TapeManager,
    async_tape: AsyncTapeManager,
    stream_filter: Option<StreamEventFilter>,
}

impl LLM {
    /// Create a new LLM client.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: Option<&str>,
        provider: Option<&str>,
        fallback_models: Option<Vec<String>>,
        max_retries: Option<u32>,
        api_key: Option<String>,
        api_key_map: Option<std::collections::HashMap<String, String>>,
        api_base: Option<String>,
        api_base_map: Option<std::collections::HashMap<String, String>>,
        api_format: Option<ApiFormat>,
        verbose: Option<u32>,
        context: Option<TapeContext>,
    ) -> Result<Self, ConduitError> {
        let verbose = verbose.unwrap_or(0);
        if verbose > 2 {
            return Err(ConduitError::new(
                ErrorKind::InvalidInput,
                "verbose must be 0, 1, or 2",
            ));
        }

        let max_retries = max_retries.unwrap_or(3);
        let model_str = model.unwrap_or(DEFAULT_MODEL);

        let (resolved_provider, resolved_model) =
            LLMCore::resolve_model_provider(model_str, provider)?;

        let api_key_config = match (api_key, api_key_map) {
            (Some(key), _) => ApiKeyConfig::Single(key),
            (None, Some(map)) => ApiKeyConfig::PerProvider(map),
            (None, None) => ApiKeyConfig::None,
        };

        let api_base_config = match (api_base, api_base_map) {
            (Some(base), _) => ApiBaseConfig::Single(base),
            (None, Some(map)) => ApiBaseConfig::PerProvider(map),
            (None, None) => ApiBaseConfig::None,
        };

        let api_format = api_format.unwrap_or_default();

        let core = LLMCore::new(
            resolved_provider,
            resolved_model,
            fallback_models.unwrap_or_default(),
            max_retries,
            api_key_config,
            api_base_config,
            api_format,
            verbose,
        );

        let shared_tape_store = InMemoryTapeStore::new();
        let async_store = AsyncTapeStoreAdapter::new(shared_tape_store.clone());

        let tape = TapeManager::new(Some(Box::new(shared_tape_store)), context.clone());
        let async_tape = AsyncTapeManager::new(Some(Box::new(async_store)), context);

        Ok(Self {
            core,
            tool_executor: ToolExecutor::new(),
            tape,
            async_tape,
            stream_filter: None,
        })
    }

    /// Return a new [`LLMBuilder`].
    pub fn builder() -> LLMBuilder {
        LLMBuilder::new()
    }

    // -- Accessors -----------------------------------------------------------

    /// The resolved model name.
    pub fn model(&self) -> &str {
        self.core.model()
    }

    /// The resolved provider name.
    pub fn provider(&self) -> &str {
        self.core.provider()
    }

    /// The fallback models.
    pub fn fallback_models(&self) -> &[String] {
        self.core.fallback_models()
    }

    /// Access the tool executor.
    pub fn tools(&self) -> &ToolExecutor {
        &self.tool_executor
    }

    /// Set a stream event filter. Events returning `None` are dropped.
    pub fn with_stream_filter(&mut self, filter: StreamEventFilter) {
        self.stream_filter = Some(filter);
    }

    /// Remove any previously set stream event filter.
    pub fn clear_stream_filter(&mut self) {
        self.stream_filter = None;
    }

    /// Set the tape context used for conversation history selection.
    pub fn set_context(&mut self, context: TapeContext) {
        self.tape.set_default_context(context.clone());
        self.async_tape.set_default_context(context);
    }

    /// Return a reference to the current tape context, if one is set.
    pub fn context(&self) -> Option<&TapeContext> {
        Some(self.tape.default_context())
    }

    /// Append a raw tape entry to the named tape (async).
    pub async fn append_tape_entry(
        &self,
        tape: &str,
        entry: &crate::tape::TapeEntry,
    ) -> Result<(), ConduitError> {
        self.async_tape.append_entry(tape, entry).await
    }

    /// Record a handoff (anchor + event) to the named tape (async).
    pub async fn handoff_tape(
        &self,
        tape: &str,
        name: &str,
        state: Option<Value>,
        meta: Value,
    ) -> Result<Vec<crate::tape::TapeEntry>, ConduitError> {
        self.async_tape.handoff(tape, name, state, meta).await
    }

    /// Create a [`TapeSession`](crate::tape::TapeSession) bound to a tape name.
    pub fn session(&mut self, tape: impl Into<String>) -> crate::tape::TapeSession<'_> {
        crate::tape::TapeSession::new(self, tape)
    }

    /// Access the current stream filter, if any.
    pub fn stream_filter(&self) -> Option<&StreamEventFilter> {
        self.stream_filter.as_ref()
    }

    // -- Sync wrappers -------------------------------------------------------

    /// Synchronous wrapper for [`chat_async`](Self::chat_async).
    ///
    /// Creates a single-threaded tokio runtime and blocks the current thread.
    pub fn chat_sync(&mut self, req: ChatRequest<'_>) -> Result<String, ConduitError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| {
                ConduitError::new(ErrorKind::Unknown, format!("Failed to create runtime: {e}"))
            })?;
        rt.block_on(self.chat_async(req))
    }

    /// Synchronous wrapper for [`run_tools`](Self::run_tools).
    ///
    /// Creates a single-threaded tokio runtime and blocks the current thread.
    pub fn run_tools_sync(&mut self, req: ChatRequest<'_>) -> Result<ToolAutoResult, ConduitError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| {
                ConduitError::new(ErrorKind::Unknown, format!("Failed to create runtime: {e}"))
            })?;
        rt.block_on(self.run_tools(req))
    }

    // -- Chat ----------------------------------------------------------------

    /// Synchronous chat (blocks the current thread).
    pub fn chat(&mut self, req: ChatRequest<'_>) -> Result<String, ConduitError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| {
                ConduitError::new(ErrorKind::Unknown, format!("Failed to create runtime: {e}"))
            })?;
        rt.block_on(self.chat_async(req))
    }

    /// Async chat completion returning the assistant text.
    pub async fn chat_async(&mut self, req: ChatRequest<'_>) -> Result<String, ConduitError> {
        let ChatRequest {
            prompt,
            system_prompt,
            model,
            provider,
            messages,
            max_tokens,
            tape,
            ..
        } = req;

        // Read existing tape messages if a tape name is provided.
        let tape_messages = if let Some(tape_name) = tape {
            match self.async_tape.read_messages(tape_name, None).await {
                Ok(messages) => messages,
                Err(e) => {
                    tracing::error!(error = %e, tape = %tape_name, "failed to read tape messages");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        let mut msgs = build_messages(prompt, system_prompt, messages.as_deref());
        if !tape_messages.is_empty() {
            // Prepend tape history before the new messages (after system prompt).
            let system_count = msgs
                .iter()
                .take_while(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
                .count();
            let mut combined = msgs[..system_count].to_vec();
            combined.extend(tape_messages);
            combined.extend_from_slice(&msgs[system_count..]);
            msgs = combined;
        }

        let new_messages: Vec<Value> = msgs
            .iter()
            .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
            .cloned()
            .collect();

        let response =
            self.core
                .run_chat(
                    msgs,
                    None, // tools_payload
                    model,
                    provider,
                    max_tokens,
                    false, // stream
                    None,  // reasoning_effort
                    Default::default(),
                    |resp: TransportResponse, _prov: &str, _model: &str, _attempt: u32| {
                        Ok(resp.payload)
                    },
                )
                .await?;

        let content = extract_content(&response)?;

        // Record the exchange to tape if a tape name is provided.
        if let Some(tape_name) = tape {
            let run_id = Uuid::new_v4().to_string();
            if let Err(e) = self
                .async_tape
                .record_chat(
                    tape_name,
                    &run_id,
                    system_prompt,
                    None, // context_error
                    &new_messages,
                    Some(&content),
                    None, // tool_calls
                    None, // tool_results
                    None, // error
                    None, // usage
                    Some(self.core.provider()),
                    Some(self.core.model()),
                )
                .await
            {
                tracing::error!(error = %e, tape = %tape_name, "failed to record chat transcript");
            }
        }

        Ok(content)
    }

    // -- Tool calls ----------------------------------------------------------

    /// Get raw tool calls from the model.
    pub async fn tool_calls(&mut self, req: ChatRequest<'_>) -> Result<Vec<Value>, ConduitError> {
        let ChatRequest {
            prompt,
            system_prompt,
            model,
            provider,
            messages,
            max_tokens,
            tools,
            ..
        } = req;
        let tools = tools.ok_or_else(|| {
            ConduitError::new(ErrorKind::InvalidInput, "tool_calls requires tools")
        })?;
        let msgs = build_messages(prompt, system_prompt, messages.as_deref());
        let schemas = tools.payload().map(|s| s.to_vec());
        let response =
            self.core
                .run_chat(
                    msgs,
                    schemas,
                    model,
                    provider,
                    max_tokens,
                    false,
                    None,
                    Default::default(),
                    |resp: TransportResponse, _prov: &str, _model: &str, _attempt: u32| {
                        Ok(resp.payload)
                    },
                )
                .await?;

        extract_tool_calls(&response)
    }

    /// Get tool calls and execute them against the provided tools.
    pub async fn run_tools(
        &mut self,
        req: ChatRequest<'_>,
    ) -> Result<ToolAutoResult, ConduitError> {
        let ChatRequest {
            prompt,
            system_prompt,
            model,
            provider,
            messages,
            max_tokens,
            tools,
            tool_context: context,
            tape,
            tape_context,
        } = req;
        let tools = tools.ok_or_else(|| {
            ConduitError::new(ErrorKind::InvalidInput, "run_tools requires tools")
        })?;
        let schemas = tools.payload().map(|s| s.to_vec());

        // Accumulate tool calls/results across all rounds for the return value
        let mut all_tool_calls: Vec<Value> = Vec::new();
        let mut all_tool_results: Vec<Value> = Vec::new();

        // Track the latest usage from API responses.
        let mut last_usage: Option<Value> = None;

        let initial_round_msgs = build_messages(prompt, system_prompt, messages.as_deref());
        let mut in_memory_msgs = initial_round_msgs.clone();

        // On the first round with tape, write the initial context (system + user prompt)
        // to tape so subsequent reads include it.
        let mut first_round = true;

        let round_params = RoundParams {
            schemas: &schemas,
            model,
            provider,
            max_tokens,
            tools,
            tool_context: context,
        };

        let max_iterations: usize = 250; // Safety limit for tool-calling rounds
        let mut iteration: usize = 0;

        loop {
            iteration += 1;
            if iteration > max_iterations {
                return Err(ConduitError::new(
                    ErrorKind::Unknown,
                    format!("run_tools exceeded max iterations ({})", max_iterations),
                ));
            }

            // Build msgs for this round
            let msgs = self
                ._prepare_messages(
                    tape,
                    tape_context,
                    first_round,
                    &initial_round_msgs,
                    &in_memory_msgs,
                )
                .await?;

            first_round = false;

            // Execute model call + tool round
            let round = self._execute_tool_round(&msgs, &round_params).await?;

            // Update cumulative usage
            if let Some(usage) = round.usage {
                last_usage = Some(usage);
            }

            match round.outcome {
                ToolRoundOutcome::Text(content) => {
                    // Write final assistant message to tape
                    if let Some(tape_name) = tape {
                        let meta = serde_json::json!({ "run_id": Uuid::new_v4().to_string() });
                        let assistant_msg =
                            serde_json::json!({"role": "assistant", "content": &content});
                        self.async_tape
                            .append_entry(tape_name, &TapeEntry::message(assistant_msg, meta))
                            .await?;
                    }

                    return Ok(ToolAutoResult {
                        kind: ToolAutoResultKind::Text,
                        text: Some(content),
                        tool_calls: all_tool_calls,
                        tool_results: all_tool_results,
                        error: None,
                        usage: last_usage,
                    });
                }
                ToolRoundOutcome::Tools {
                    response,
                    execution,
                } => {
                    // Accumulate for return value
                    all_tool_calls.extend(execution.tool_calls.clone());
                    all_tool_results.extend(execution.tool_results.clone());

                    // Persist round to tape or accumulate in memory
                    self._persist_round(tape, &response, &execution, &mut in_memory_msgs)
                        .await?;
                }
            }
            // Loop continues - next iteration reads from tape (or in_memory_msgs)
        }
    }

    // -- run_tools helpers ---------------------------------------------------

    /// Build the message list for a single tool round.
    ///
    /// When a tape is active, writes initial messages on the first round and
    /// then reads the full context back from tape. Otherwise returns the
    /// in-memory accumulation.
    async fn _prepare_messages(
        &self,
        tape: Option<&str>,
        tape_context: Option<&TapeContext>,
        first_round: bool,
        initial_round_msgs: &[Value],
        in_memory_msgs: &[Value],
    ) -> Result<Vec<Value>, ConduitError> {
        if let Some(tape_name) = tape {
            if first_round && !initial_round_msgs.is_empty() {
                self.append_initial_round_messages(tape_name, initial_round_msgs)
                    .await?;
            }

            // Read full context from tape (includes system, messages, tool_call, tool_result)
            let default_ctx = self.async_tape.default_context().clone();
            let ctx = tape_context.unwrap_or(&default_ctx);
            let query = ctx.build_query(self.async_tape.query_tape(tape_name));
            let entries = match self.async_tape.fetch_entries(&query).await {
                Ok(entries) => entries,
                Err(e) if e.kind == ErrorKind::NotFound && query.after_last => {
                    tracing::warn!(
                        error = %e,
                        tape = %tape_name,
                        "anchored tape context unavailable; falling back to full tape"
                    );
                    match self
                        .async_tape
                        .fetch_entries(&self.async_tape.query_tape(tape_name))
                        .await
                    {
                        Ok(entries) => entries,
                        Err(fallback) => {
                            tracing::error!(
                                error = %fallback,
                                tape = %tape_name,
                                "failed to fetch fallback tape entries"
                            );
                            Vec::new()
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, tape = %tape_name, "failed to fetch tape entries");
                    Vec::new()
                }
            };
            let mut tape_msgs = build_full_context_from_entries(&entries);
            // Apply context budget: truncate large tool results, trim if over budget
            crate::tape::context::apply_context_budget(&mut tape_msgs);
            Ok(tape_msgs)
        } else {
            Ok(in_memory_msgs.to_vec())
        }
    }

    async fn append_initial_round_messages(
        &self,
        tape_name: &str,
        initial_round_msgs: &[Value],
    ) -> Result<(), ConduitError> {
        let run_id = Uuid::new_v4().to_string();
        let meta = serde_json::json!({ "run_id": run_id });

        for message in initial_round_msgs {
            let role = message.get("role").and_then(|v| v.as_str());
            if role == Some("system")
                && let Some(content) = message.get("content").and_then(|v| v.as_str())
            {
                self.async_tape
                    .append_entry(tape_name, &TapeEntry::system(content, meta.clone()))
                    .await?;
            } else {
                self.async_tape
                    .append_entry(
                        tape_name,
                        &TapeEntry::message(message.clone(), meta.clone()),
                    )
                    .await?;
            }
        }

        Ok(())
    }

    /// Execute a single model call and, if tool calls are returned, execute them.
    ///
    /// Returns the round outcome (text response or tool execution results).
    async fn _execute_tool_round(
        &mut self,
        msgs: &[Value],
        params: &RoundParams<'_>,
    ) -> Result<ToolRound, ConduitError> {
        let response =
            self.core
                .run_chat(
                    msgs.to_vec(),
                    params.schemas.clone(),
                    params.model,
                    params.provider,
                    params.max_tokens,
                    false,
                    None,
                    Default::default(),
                    |resp: TransportResponse, _prov: &str, _model: &str, _attempt: u32| {
                        Ok(resp.payload)
                    },
                )
                .await?;

        let usage = response.get("usage").cloned();
        let raw_calls = extract_tool_calls(&response)?;

        if raw_calls.is_empty() {
            let content = extract_content(&response)?;
            return Ok(ToolRound {
                usage,
                outcome: ToolRoundOutcome::Text(content),
            });
        }

        // Execute tools
        let execution = self
            .tool_executor
            .execute_async(
                ToolCallResponse::List(raw_calls),
                &params.tools.runnable,
                params.tool_context,
            )
            .await?;

        // Log tool errors but continue the loop so the LLM can see the
        // error as a tool result and react (retry, try a different
        // approach, or report failure gracefully).
        if let Some(ref err) = execution.error {
            tracing::warn!(
                error = %err,
                "tool execution error — feeding back to LLM for recovery"
            );
        }

        Ok(ToolRound {
            usage,
            outcome: ToolRoundOutcome::Tools {
                response,
                execution,
            },
        })
    }

    /// Write a completed tool round to tape, or accumulate in memory.
    async fn _persist_round(
        &self,
        tape: Option<&str>,
        response: &Value,
        execution: &ToolExecution,
        in_memory_msgs: &mut Vec<Value>,
    ) -> Result<(), ConduitError> {
        if let Some(tape_name) = tape {
            // Write assistant tool_call entry to tape
            let meta = serde_json::json!({ "run_id": Uuid::new_v4().to_string() });
            let assistant_msg = build_assistant_tool_call_message(response);
            let assistant_text = assistant_msg
                .get("content")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            self.async_tape
                .append_entry(
                    tape_name,
                    &TapeEntry::tool_call_with_content(
                        execution.tool_calls.clone(),
                        assistant_text,
                        meta.clone(),
                    ),
                )
                .await?;

            // Write tool_result entries to tape
            let paired: Vec<Value> = execution
                .tool_calls
                .iter()
                .zip(execution.tool_results.iter())
                .map(|(call, result)| {
                    let call_id = call.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                    serde_json::json!({"call_id": call_id, "output": result})
                })
                .collect();
            self.async_tape
                .append_entry(tape_name, &TapeEntry::tool_result(paired, meta))
                .await?;
        } else {
            // tape=None fallback: accumulate in memory
            let assistant_msg = build_assistant_tool_call_message(response);
            in_memory_msgs.push(assistant_msg);

            for (i, result) in execution.tool_results.iter().enumerate() {
                let call_id = execution
                    .tool_calls
                    .get(i)
                    .and_then(|c| c.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let content_str = match result {
                    Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                in_memory_msgs.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": content_str,
                }));
            }
        }
        Ok(())
    }

    // -- Streaming -----------------------------------------------------------

    /// Stream chat completion as an async `TextStream`.
    pub async fn stream(&mut self, req: ChatRequest<'_>) -> Result<AsyncTextStream, ConduitError> {
        let ChatRequest {
            prompt,
            system_prompt,
            model,
            provider,
            messages,
            max_tokens,
            tape,
            ..
        } = req;
        use futures::StreamExt;

        // Read existing tape messages if a tape name is provided.
        let tape_messages = if let Some(tape_name) = tape {
            match self.async_tape.read_messages(tape_name, None).await {
                Ok(messages) => messages,
                Err(e) => {
                    tracing::error!(error = %e, tape = %tape_name, "failed to read tape messages");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        let mut msgs = build_messages(prompt, system_prompt, messages.as_deref());
        if !tape_messages.is_empty() {
            let system_count = msgs
                .iter()
                .take_while(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
                .count();
            let mut combined = msgs[..system_count].to_vec();
            combined.extend(tape_messages);
            combined.extend_from_slice(&msgs[system_count..]);
            msgs = combined;
        }

        // For streaming, record the user messages to tape before streaming starts.
        // The full assistant response should be recorded by the caller after
        // consuming the stream (e.g., via TapeSession or manually).
        if let Some(tape_name) = tape {
            let new_messages: Vec<Value> = msgs
                .iter()
                .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
                .cloned()
                .collect();
            let run_id = Uuid::new_v4().to_string();
            if let Err(e) = self
                .async_tape
                .record_chat(
                    tape_name,
                    &run_id,
                    system_prompt,
                    None,
                    &new_messages,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(self.core.provider()),
                    Some(self.core.model()),
                )
                .await
            {
                tracing::error!(error = %e, tape = %tape_name, "failed to record streaming chat context");
            }
        }

        let (response, transport, _prov, _model) = self
            .core
            .run_chat_stream(
                msgs,
                None,
                model,
                provider,
                max_tokens,
                None,
                Default::default(),
            )
            .await?;

        let byte_stream = response.bytes_stream();
        let parser = parser_for_transport(transport);
        let text_stream = byte_stream.filter_map(move |chunk| async move {
            match chunk {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    let mut output = String::new();
                    for line in text.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                continue;
                            }
                            if let Ok(val) = serde_json::from_str::<Value>(data) {
                                let content = parser.extract_chunk_text(&val);
                                if !content.is_empty() {
                                    output.push_str(&content);
                                }
                            }
                        }
                    }
                    if output.is_empty() {
                        None
                    } else {
                        Some(output)
                    }
                }
                Err(_) => None,
            }
        });

        Ok(AsyncTextStream::new(text_stream, None))
    }

    // -- Responses -----------------------------------------------------------

    /// Send a raw responses-format request.
    pub async fn responses(
        &mut self,
        input: Value,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> Result<Value, ConduitError> {
        let prov = provider
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.core.provider().to_string());
        let base = self
            .core
            .resolve_api_base(&prov)
            .unwrap_or_else(|| default_api_base(&prov));
        let api_key = self.core.resolve_api_key(&prov).ok_or_else(|| {
            ConduitError::new(
                ErrorKind::Config,
                format!("No API key found for provider '{prov}'"),
            )
        })?;

        let mdl = model
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.core.model().to_string());

        let url = format!("{}/responses", base.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": mdl,
            "input": input,
        });

        let client = self.core.get_client(&prov);
        let resp = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Provider, format!("HTTP request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ConduitError::new(
                ErrorKind::Provider,
                format!("HTTP {status}: {text}"),
            ));
        }

        resp.json::<Value>().await.map_err(|e| {
            ConduitError::new(
                ErrorKind::Provider,
                format!("Failed to parse responses response: {e}"),
            )
        })
    }

    // -- Embeddings ----------------------------------------------------------

    /// Embed one or more inputs.
    pub async fn embed(
        &mut self,
        inputs: EmbedInput<'_>,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> Result<Vec<Vec<f64>>, ConduitError> {
        let prov = provider
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.core.provider().to_string());
        let mdl = model
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.core.model().to_string());
        let base = self
            .core
            .resolve_api_base(&prov)
            .unwrap_or_else(|| default_api_base(&prov));
        let url = format!("{base}/embeddings");

        let api_key = self.core.resolve_api_key(&prov).ok_or_else(|| {
            ConduitError::new(
                ErrorKind::Config,
                format!("No API key found for provider '{prov}'"),
            )
        })?;

        let input_val: Value = match inputs {
            EmbedInput::Single(s) => serde_json::json!(s),
            EmbedInput::Multiple(v) => serde_json::json!(v),
        };

        let body = serde_json::json!({
            "model": mdl,
            "input": input_val,
        });

        let client = self.core.get_client(&prov);
        let resp = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Provider, format!("HTTP request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ConduitError::new(
                ErrorKind::Provider,
                format!("HTTP {status}: {text}"),
            ));
        }

        let val: Value = resp.json().await.map_err(|e| {
            ConduitError::new(
                ErrorKind::Provider,
                format!("Failed to parse embedding response: {e}"),
            )
        })?;

        let data = val.get("data").and_then(|d| d.as_array()).ok_or_else(|| {
            ConduitError::new(
                ErrorKind::Provider,
                "Embedding response missing 'data' array",
            )
        })?;

        let mut embeddings = Vec::with_capacity(data.len());
        for item in data {
            let embedding = item
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| {
                    ConduitError::new(ErrorKind::Provider, "Embedding item missing 'embedding'")
                })?
                .iter()
                .filter_map(|v| v.as_f64())
                .collect::<Vec<f64>>();
            embeddings.push(embedding);
        }

        Ok(embeddings)
    }

    // -- Text utilities ------------------------------------------------------

    /// Boolean question: does the input satisfy the question?
    pub async fn if_(&mut self, input_text: &str, question: &str) -> Result<bool, ConduitError> {
        let prompt = format!(
            "Answer ONLY 'yes' or 'no'. Question about the following text:\n\
             Text: {input_text}\n\
             Question: {question}"
        );
        let answer = self
            .chat_async(ChatRequest {
                prompt: Some(&prompt),
                max_tokens: Some(16),
                ..Default::default()
            })
            .await?;
        let normalized = answer.trim().to_lowercase();
        Ok(normalized.starts_with("yes"))
    }

    /// Classify input text into one of the provided choices.
    pub async fn classify(
        &mut self,
        input_text: &str,
        choices: &[String],
    ) -> Result<String, ConduitError> {
        if choices.is_empty() {
            return Err(ConduitError::new(
                ErrorKind::InvalidInput,
                "classify requires at least one choice",
            ));
        }

        let choices_str = choices
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}. {c}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Classify the following text into exactly one of the categories below.\n\
             Reply with ONLY the category label, nothing else.\n\n\
             Categories:\n{choices_str}\n\n\
             Text: {input_text}"
        );

        let answer = self
            .chat_async(ChatRequest {
                prompt: Some(&prompt),
                max_tokens: Some(64),
                ..Default::default()
            })
            .await?;

        let trimmed = answer.trim().to_string();

        // Try exact match first
        for choice in choices {
            if trimmed == *choice {
                return Ok(choice.clone());
            }
        }
        // Try case-insensitive prefix match
        let lower = trimmed.to_lowercase();
        for choice in choices {
            if lower.starts_with(&choice.to_lowercase()) {
                return Ok(choice.clone());
            }
        }
        // Fallback: return raw answer
        Ok(trimmed)
    }
}

impl fmt::Display for LLM {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LLM({}:{})", self.core.provider(), self.core.model())
    }
}

impl fmt::Debug for LLM {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LLM")
            .field("provider", &self.core.provider())
            .field("model", &self.core.model())
            .field("fallback_models", &self.core.fallback_models())
            .field("max_retries", &self.core.max_retries())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Internal types for run_tools decomposition
// ---------------------------------------------------------------------------

/// Parameters for a single tool-calling round (avoids too-many-arguments).
struct RoundParams<'a> {
    schemas: &'a Option<Vec<Value>>,
    model: Option<&'a str>,
    provider: Option<&'a str>,
    max_tokens: Option<u32>,
    tools: &'a ToolSet,
    tool_context: Option<&'a ToolContext>,
}

/// Result of a single tool-calling round.
struct ToolRound {
    usage: Option<Value>,
    outcome: ToolRoundOutcome,
}

/// Whether the model returned text (done) or tool calls (continue looping).
enum ToolRoundOutcome {
    /// Model returned a text response — no more tool calls.
    Text(String),
    /// Model returned tool calls that were executed.
    Tools {
        response: Value,
        execution: ToolExecution,
    },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Input for embedding operations.
pub enum EmbedInput<'a> {
    Single(&'a str),
    Multiple(&'a [String]),
}

impl<'a> From<&'a str> for EmbedInput<'a> {
    fn from(s: &'a str) -> Self {
        EmbedInput::Single(s)
    }
}

impl<'a> From<&'a [String]> for EmbedInput<'a> {
    fn from(v: &'a [String]) -> Self {
        EmbedInput::Multiple(v)
    }
}

pub(crate) fn default_api_base(provider: &str) -> String {
    match provider {
        "openai" => "https://api.openai.com/v1".to_string(),
        "anthropic" => "https://api.anthropic.com/v1".to_string(),
        other => format!("https://api.{other}.com/v1"),
    }
}

fn build_messages(
    prompt: Option<&str>,
    system_prompt: Option<&str>,
    messages: Option<&[Value]>,
) -> Vec<Value> {
    let mut msgs = Vec::new();
    if let Some(sys) = system_prompt {
        msgs.push(serde_json::json!({"role": "system", "content": sys}));
    }
    if let Some(existing) = messages {
        msgs.extend_from_slice(existing);
    }
    if let Some(p) = prompt {
        msgs.push(serde_json::json!({"role": "user", "content": p}));
    }
    msgs
}

/// Build full conversation context from tape entries, including tool calls and results.
/// This is needed because the default `build_messages` only extracts "message" entries.
fn build_full_context_from_entries(entries: &[TapeEntry]) -> Vec<Value> {
    let mut messages = Vec::new();
    for entry in entries {
        match entry.kind.as_str() {
            "message" => {
                if entry.payload.is_object() {
                    messages.push(normalize_message_tool_calls(&entry.payload));
                }
            }
            "system" => {
                if let Some(content) = entry.payload.get("content").and_then(|c| c.as_str()) {
                    messages.push(serde_json::json!({"role": "system", "content": content}));
                }
            }
            "tool_call" => {
                if let Some(calls) = entry.payload.get("calls").and_then(|c| c.as_array()) {
                    let normalized_calls = normalize_tool_calls(calls);
                    if !normalized_calls.is_empty() {
                        let content = entry.payload.get("content").cloned().unwrap_or(Value::Null);
                        messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": content,
                            "tool_calls": normalized_calls
                        }));
                    }
                }
            }
            "tool_result" => {
                if let Some(results) = entry.payload.get("results").and_then(|r| r.as_array()) {
                    for result in results {
                        let tool_call_id = result
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let content = result
                            .get("output")
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                other => serde_json::to_string(other).unwrap_or_default(),
                            })
                            .unwrap_or_default();
                        messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tool_call_id,
                            "content": content
                        }));
                    }
                }
            }
            _ => {} // Skip anchors, events, errors
        }
    }

    // Deduplicate system messages: keep only the last one
    let mut last_system_idx = None;
    for (i, msg) in messages.iter().enumerate() {
        if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
            last_system_idx = Some(i);
        }
    }
    if let Some(last_idx) = last_system_idx {
        let mut deduped = Vec::new();
        for (i, msg) in messages.into_iter().enumerate() {
            let is_system = msg.get("role").and_then(|r| r.as_str()) == Some("system");
            if !is_system || i == last_idx {
                deduped.push(msg);
            }
        }
        return deduped;
    }

    messages
}

fn extract_content(response: &Value) -> Result<String, ConduitError> {
    // Try completion format first
    if let Some(content) = response
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
    {
        return Ok(content.to_string());
    }
    // Try Anthropic Messages format: { "content": [{"type": "text", "text": "..."}], "role": "assistant" }
    if response.get("role").and_then(|r| r.as_str()) == Some("assistant")
        && let Some(content_arr) = response.get("content").and_then(|c| c.as_array())
    {
        let mut text_parts: Vec<String> = Vec::new();
        for block in content_arr {
            if block.get("type").and_then(|t| t.as_str()) == Some("text")
                && let Some(text) = block.get("text").and_then(|t| t.as_str())
            {
                text_parts.push(text.to_string());
            }
        }
        if !text_parts.is_empty() {
            return Ok(text_parts.join(""));
        }
    }
    // Try responses format
    if let Some(output) = response.get("output").and_then(|o| o.as_array()) {
        for item in output {
            if item.get("type").and_then(|t| t.as_str()) == Some("message")
                && let Some(content) = item
                    .get("content")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
            {
                return Ok(content.to_string());
            }
        }
    }
    Err(ConduitError::new(
        ErrorKind::Provider,
        "Response missing content",
    ))
}

fn extract_tool_calls(response: &Value) -> Result<Vec<Value>, ConduitError> {
    // Completion format
    if let Some(calls) = response
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("tool_calls"))
        .and_then(|tc| tc.as_array())
    {
        return Ok(normalize_tool_calls(calls));
    }
    // Anthropic Messages format: { "content": [{"type": "tool_use", "id": "...", "name": "...", "input": {...}}] }
    if let Some(content_arr) = response.get("content").and_then(|c| c.as_array()) {
        let mut calls = Vec::new();
        for block in content_arr {
            if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                calls.push(block.clone());
            }
        }
        if !calls.is_empty() {
            return Ok(normalize_tool_calls(&calls));
        }
    }
    // Responses format
    if let Some(output) = response.get("output").and_then(|o| o.as_array()) {
        let mut calls = Vec::new();
        for item in output {
            if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                calls.push(item.clone());
            }
        }
        if !calls.is_empty() {
            return Ok(normalize_tool_calls(&calls));
        }
    }
    Ok(Vec::new())
}

/// Build an assistant message containing tool_calls from the raw API response.
/// Supports OpenAI completion, Anthropic Messages, and Responses formats.
fn build_assistant_tool_call_message(response: &Value) -> Value {
    // OpenAI completion format: response.choices[0].message
    if let Some(msg) = response
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
    {
        return normalize_message_tool_calls(msg);
    }

    // Anthropic Messages format: build from response.content
    if let Some(content) = response.get("content").and_then(|c| c.as_array()) {
        let mut tool_calls = Vec::new();
        let mut text_parts = Vec::new();
        for block in content {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("tool_use") => {
                    tool_calls.push(serde_json::json!({
                        "id": block.get("id").cloned().unwrap_or(Value::Null),
                        "type": "function",
                        "function": {
                            "name": block.get("name").cloned().unwrap_or(Value::Null),
                            "arguments": serde_json::to_string(
                                block.get("input").unwrap_or(&Value::Null)
                            ).unwrap_or_default(),
                        }
                    }));
                }
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                        text_parts.push(t.to_owned());
                    }
                }
                _ => {}
            }
        }
        let content_val = if text_parts.is_empty() {
            Value::Null
        } else {
            Value::String(text_parts.join(""))
        };
        return normalize_message_tool_calls(&serde_json::json!({
            "role": "assistant",
            "content": content_val,
            "tool_calls": tool_calls,
        }));
    }

    // Responses format: build from response.output
    if let Some(output) = response.get("output").and_then(|o| o.as_array()) {
        let mut tool_calls = Vec::new();
        for item in output {
            if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                tool_calls.push(serde_json::json!({
                    "id": item.get("call_id").cloned().unwrap_or(Value::Null),
                    "type": "function",
                    "function": {
                        "name": item.get("name").cloned().unwrap_or(Value::Null),
                        "arguments": item.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}"),
                    }
                }));
            }
        }
        return normalize_message_tool_calls(&serde_json::json!({
            "role": "assistant",
            "content": null,
            "tool_calls": tool_calls,
        }));
    }

    // Fallback
    serde_json::json!({"role": "assistant", "content": null})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::results::StreamEventKind;
    use serde_json::json;

    // ----- LLM::new -----

    #[test]
    fn test_llm_new_default_config() {
        let llm = LLM::new(
            Some("openai:gpt-4o"),
            None,
            None,
            None,
            Some("test-key".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(llm.model(), "gpt-4o");
        assert_eq!(llm.provider(), "openai");
        assert!(llm.fallback_models().is_empty());
    }

    #[test]
    fn test_llm_new_with_provider_prefix() {
        let llm = LLM::new(
            Some("anthropic:claude-3-5-sonnet"),
            None,
            None,
            None,
            Some("key".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(llm.model(), "claude-3-5-sonnet");
        assert_eq!(llm.provider(), "anthropic");
    }

    #[test]
    fn test_llm_new_with_explicit_provider() {
        let llm = LLM::new(
            Some("gpt-4o"),
            Some("openai"),
            None,
            None,
            Some("key".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(llm.model(), "gpt-4o");
        assert_eq!(llm.provider(), "openai");
    }

    #[test]
    fn test_llm_new_defaults_to_gpt4o_mini() {
        let llm = LLM::new(
            None,
            None,
            None,
            None,
            Some("key".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        // Default model is "openai:gpt-4o-mini"
        assert_eq!(llm.provider(), "openai");
        assert_eq!(llm.model(), "gpt-4o-mini");
    }

    #[test]
    fn test_llm_new_rejects_invalid_verbose() {
        let result = LLM::new(
            Some("openai:gpt-4o"),
            None,
            None,
            None,
            Some("key".to_string()),
            None,
            None,
            None,
            None,
            Some(5),
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("verbose"));
    }

    #[test]
    fn test_llm_new_rejects_provider_prefix_with_explicit_provider() {
        let result = LLM::new(
            Some("openai:gpt-4o"),
            Some("anthropic"),
            None,
            None,
            Some("key".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_llm_new_with_fallback_models() {
        let llm = LLM::new(
            Some("openai:gpt-4o"),
            None,
            Some(vec!["openai:gpt-4o-mini".to_string()]),
            None,
            Some("key".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(llm.fallback_models(), &["openai:gpt-4o-mini"]);
    }

    // ----- Display / Debug -----

    #[test]
    fn test_llm_display() {
        let llm = LLM::new(
            Some("openai:gpt-4o"),
            None,
            None,
            None,
            Some("key".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let display = format!("{}", llm);
        assert_eq!(display, "LLM(openai:gpt-4o)");
    }

    #[test]
    fn test_llm_debug() {
        let llm = LLM::new(
            Some("openai:gpt-4o"),
            None,
            None,
            None,
            Some("key".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let debug = format!("{:?}", llm);
        assert!(debug.contains("LLM"));
        assert!(debug.contains("openai"));
    }

    // ----- ApiFormat -----

    #[test]
    fn test_api_format_as_str() {
        assert_eq!(ApiFormat::Auto.as_str(), "auto");
        assert_eq!(ApiFormat::Completion.as_str(), "completion");
        assert_eq!(ApiFormat::Responses.as_str(), "responses");
        assert_eq!(ApiFormat::Messages.as_str(), "messages");
    }

    #[test]
    fn test_api_format_equality() {
        assert_eq!(ApiFormat::Auto, ApiFormat::Auto);
        assert_eq!(ApiFormat::Completion, ApiFormat::Completion);
        assert_ne!(ApiFormat::Completion, ApiFormat::Responses);
    }

    // ----- EmbedInput -----

    #[test]
    fn test_embed_input_from_str() {
        let input: EmbedInput = "hello".into();
        matches!(input, EmbedInput::Single("hello"));
    }

    #[test]
    fn test_embed_input_from_slice() {
        let data = vec!["a".to_string(), "b".to_string()];
        let input: EmbedInput = data.as_slice().into();
        matches!(input, EmbedInput::Multiple(_));
    }

    // ----- build_messages -----

    #[test]
    fn test_build_messages_with_prompt_only() {
        let msgs = build_messages(Some("hello"), None, None);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "hello");
    }

    #[test]
    fn test_build_messages_with_system_and_prompt() {
        let msgs = build_messages(Some("hello"), Some("you are helpful"), None);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "you are helpful");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn test_build_messages_with_existing_messages() {
        let existing = vec![json!({"role": "assistant", "content": "hi"})];
        let msgs = build_messages(Some("follow up"), None, Some(&existing));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "assistant");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn test_build_messages_empty() {
        let msgs = build_messages(None, None, None);
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_build_full_context_from_entries_normalizes_responses_tool_calls() {
        let entries = vec![
            TapeEntry::tool_call(
                vec![json!({
                    "type": "function_call",
                    "call_id": "call_123",
                    "name": "tape_info",
                    "arguments": "{}"
                })],
                json!({}),
            ),
            TapeEntry::tool_result(
                vec![json!({
                    "call_id": "call_123",
                    "output": {"count": 1}
                })],
                json!({}),
            ),
        ];

        let messages = build_full_context_from_entries(&entries);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["tool_calls"][0]["id"], "call_123");
        assert_eq!(
            messages[0]["tool_calls"][0]["function"]["name"],
            "tape_info"
        );
        assert_eq!(messages[1]["tool_call_id"], "call_123");
    }

    #[test]
    fn test_build_full_context_from_entries_preserves_tool_call_content() {
        let entries = vec![TapeEntry::tool_call_with_content(
            vec![json!({
                "type": "function",
                "id": "call_123",
                "function": {
                    "name": "tape_info",
                    "arguments": "{}"
                }
            })],
            Some("Checking tape state".to_owned()),
            json!({}),
        )];

        let messages = build_full_context_from_entries(&entries);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"], "Checking tape state");
        assert_eq!(messages[0]["tool_calls"][0]["id"], "call_123");
    }

    #[tokio::test]
    async fn test_prepare_messages_with_tape_persists_initial_prompt_and_system_prompt() {
        let llm = LLM::builder()
            .tape_store(AsyncTapeStoreAdapter::new(InMemoryTapeStore::new()))
            .build()
            .unwrap();
        let initial_round_msgs = build_messages(Some("hello"), Some("system"), None);

        let messages = llm
            ._prepare_messages(
                Some("test-tape"),
                None,
                true,
                &initial_round_msgs,
                &initial_round_msgs,
            )
            .await
            .unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "hello");

        let entries = llm
            .async_tape
            .fetch_entries(&llm.async_tape.query_tape("test-tape"))
            .await
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, "system");
        assert_eq!(entries[1].kind, "message");
    }

    // ----- extract_content -----

    #[test]
    fn test_extract_content_completion_format() {
        let response = json!({
            "choices": [{
                "message": {
                    "content": "Hello there!"
                }
            }]
        });
        assert_eq!(extract_content(&response).unwrap(), "Hello there!");
    }

    #[test]
    fn test_extract_content_responses_format() {
        let response = json!({
            "output": [{
                "type": "message",
                "content": [{"text": "Response text"}]
            }]
        });
        assert_eq!(extract_content(&response).unwrap(), "Response text");
    }

    #[test]
    fn test_extract_content_missing() {
        let response = json!({});
        assert!(extract_content(&response).is_err());
    }

    #[test]
    fn test_extract_content_anthropic_empty_content_errors() {
        let response = json!({
            "role": "assistant",
            "content": []
        });
        assert!(extract_content(&response).is_err());
    }

    // ----- extract_tool_calls -----

    #[test]
    fn test_extract_tool_calls_completion_format() {
        let response = json!({
            "choices": [{
                "message": {
                    "tool_calls": [
                        {"type": "function", "function": {"name": "tool1", "arguments": "{}"}}
                    ]
                }
            }]
        });
        let calls = extract_tool_calls(&response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "tool1");
    }

    #[test]
    fn test_extract_tool_calls_responses_format() {
        let response = json!({
            "output": [
                {"type": "function_call", "name": "tool1", "arguments": "{}"},
                {"type": "message", "content": [{"text": "hello"}]},
            ]
        });
        let calls = extract_tool_calls(&response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "tool1");
    }

    #[test]
    fn test_extract_tool_calls_empty() {
        let response = json!({"choices": [{"message": {"content": "no tools"}}]});
        let calls = extract_tool_calls(&response).unwrap();
        assert!(calls.is_empty());
    }

    // ----- default_api_base -----

    #[test]
    fn test_default_api_base_openai() {
        assert_eq!(default_api_base("openai"), "https://api.openai.com/v1");
    }

    #[test]
    fn test_default_api_base_anthropic() {
        assert_eq!(
            default_api_base("anthropic"),
            "https://api.anthropic.com/v1"
        );
    }

    #[test]
    fn test_default_api_base_other() {
        assert_eq!(default_api_base("cohere"), "https://api.cohere.com/v1");
    }

    // ----- LLMBuilder -----

    #[test]
    fn test_builder_basic() {
        let llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key("test-key")
            .build()
            .unwrap();

        assert_eq!(llm.model(), "gpt-4o");
        assert_eq!(llm.provider(), "openai");
        assert!(llm.fallback_models().is_empty());
        assert!(llm.stream_filter().is_none());
    }

    #[test]
    fn test_builder_with_provider() {
        let llm = LLM::builder()
            .model("gpt-4o")
            .provider("openai")
            .api_key("test-key")
            .build()
            .unwrap();

        assert_eq!(llm.model(), "gpt-4o");
        assert_eq!(llm.provider(), "openai");
    }

    #[test]
    fn test_builder_with_fallback_models() {
        let llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key("test-key")
            .fallback_models(vec!["openai:gpt-4o-mini".to_string()])
            .build()
            .unwrap();

        assert_eq!(llm.fallback_models(), &["openai:gpt-4o-mini"]);
    }

    #[test]
    fn test_builder_with_api_format() {
        let llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key("test-key")
            .api_format(ApiFormat::Responses)
            .build()
            .unwrap();

        assert_eq!(llm.provider(), "openai");
    }

    #[test]
    fn test_builder_with_verbose() {
        let llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key("test-key")
            .verbose(2)
            .build()
            .unwrap();

        assert_eq!(llm.model(), "gpt-4o");
    }

    #[test]
    fn test_builder_rejects_invalid_verbose() {
        let result = LLM::builder()
            .model("openai:gpt-4o")
            .api_key("test-key")
            .verbose(5)
            .build();

        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("verbose"));
    }

    #[test]
    fn test_builder_defaults_to_gpt4o_mini() {
        // Default model is "openai:gpt-4o-mini" which includes provider prefix
        let llm = LLM::builder().api_key("test-key").build().unwrap();

        assert_eq!(llm.provider(), "openai");
        assert_eq!(llm.model(), "gpt-4o-mini");
    }

    #[test]
    fn test_builder_with_api_key_resolver() {
        let resolver: APIKeyResolver = Box::new(|provider: &str| {
            if provider == "openai" {
                Some("resolved-key".to_string())
            } else {
                None
            }
        });

        let llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key_resolver(resolver)
            .build()
            .unwrap();

        assert_eq!(llm.model(), "gpt-4o");
        assert_eq!(llm.provider(), "openai");
    }

    #[test]
    fn test_builder_api_key_resolver_fallback_to_map() {
        // Resolver returns None for "openai", should fall back to map
        let resolver: APIKeyResolver = Box::new(|_provider: &str| None);

        let mut map = HashMap::new();
        map.insert("openai".to_string(), "map-key".to_string());

        let llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key_resolver(resolver)
            .api_key_map(map)
            .build()
            .unwrap();

        assert_eq!(llm.model(), "gpt-4o");
    }

    #[test]
    fn test_builder_explicit_key_overrides_resolver() {
        let resolver: APIKeyResolver = Box::new(|_provider: &str| Some("resolver-key".to_string()));

        let llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key("explicit-key")
            .api_key_resolver(resolver)
            .build()
            .unwrap();

        // Explicit key takes priority
        assert_eq!(llm.model(), "gpt-4o");
    }

    #[test]
    fn test_builder_with_stream_filter() {
        let filter: StreamEventFilter = Arc::new(|event| Some(event));

        let llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key("test-key")
            .stream_filter(filter)
            .build()
            .unwrap();

        assert!(llm.stream_filter().is_some());
    }

    #[test]
    fn test_builder_with_max_retries() {
        let llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key("test-key")
            .max_retries(5)
            .build()
            .unwrap();

        assert_eq!(llm.model(), "gpt-4o");
    }

    #[test]
    fn test_builder_with_api_base() {
        let llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key("test-key")
            .api_base("https://custom.api.com/v1")
            .build()
            .unwrap();

        assert_eq!(llm.model(), "gpt-4o");
    }

    #[test]
    fn test_builder_default() {
        // LLMBuilder::default() should be equivalent to LLMBuilder::new()
        let _builder = LLMBuilder::default();
    }

    // ----- StreamEventFilter -----

    #[test]
    fn test_stream_filter_drops_events() {
        // Filter that drops all Text events
        let filter: StreamEventFilter = Arc::new(|event| {
            if event.kind == StreamEventKind::Text {
                None
            } else {
                Some(event)
            }
        });

        let text_event = StreamEvent::new(StreamEventKind::Text, json!({"delta": "hello"}));
        let usage_event = StreamEvent::new(StreamEventKind::Usage, json!({"tokens": 42}));

        assert!(filter(text_event).is_none());
        assert!(filter(usage_event).is_some());
    }

    #[test]
    fn test_stream_filter_transforms_events() {
        // Filter that uppercases text deltas
        let filter: StreamEventFilter = Arc::new(|mut event| {
            if event.kind == StreamEventKind::Text {
                if let Some(delta) = event.data.get("delta").and_then(|d| d.as_str()) {
                    event.data = json!({"delta": delta.to_uppercase()});
                }
            }
            Some(event)
        });

        let event = StreamEvent::new(StreamEventKind::Text, json!({"delta": "hello"}));
        let result = filter(event).unwrap();
        assert_eq!(result.data["delta"], "HELLO");
    }

    #[test]
    fn test_stream_filter_passthrough() {
        let filter: StreamEventFilter = Arc::new(|event| Some(event));

        let event = StreamEvent::new(StreamEventKind::Final, json!({"ok": true}));
        let result = filter(event);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.kind, StreamEventKind::Final);
    }

    #[test]
    fn test_with_stream_filter_set_and_clear() {
        let mut llm = LLM::builder()
            .model("openai:gpt-4o")
            .api_key("test-key")
            .build()
            .unwrap();

        assert!(llm.stream_filter().is_none());

        let filter: StreamEventFilter = Arc::new(|event| Some(event));
        llm.with_stream_filter(filter);
        assert!(llm.stream_filter().is_some());

        llm.clear_stream_filter();
        assert!(llm.stream_filter().is_none());
    }

    // ----- LLM::responses URL/body building -----

    #[test]
    fn test_responses_url_default_provider() {
        // Verify the URL is built correctly from the default provider base
        let base = default_api_base("openai");
        let url = format!("{}/responses", base.trim_end_matches('/'));
        assert_eq!(url, "https://api.openai.com/v1/responses");
    }

    #[test]
    fn test_responses_url_custom_base() {
        let base = "https://custom.api.com/v2/";
        let url = format!("{}/responses", base.trim_end_matches('/'));
        assert_eq!(url, "https://custom.api.com/v2/responses");
    }

    #[test]
    fn test_responses_url_anthropic() {
        let base = default_api_base("anthropic");
        let url = format!("{}/responses", base.trim_end_matches('/'));
        assert_eq!(url, "https://api.anthropic.com/v1/responses");
    }

    #[test]
    fn test_responses_body_structure() {
        // Verify the body JSON structure matches what responses() would build
        let input = json!("Tell me a joke");
        let model = "gpt-4o";
        let body = json!({
            "model": model,
            "input": input,
        });
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["input"], "Tell me a joke");
    }

    #[test]
    fn test_responses_body_with_array_input() {
        let input = json!([
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi there"},
        ]);
        let body = json!({
            "model": "gpt-4o",
            "input": input,
        });
        assert!(body["input"].is_array());
        assert_eq!(body["input"].as_array().unwrap().len(), 2);
    }
}
