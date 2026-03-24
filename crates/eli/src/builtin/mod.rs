//! Builtin module — default hook implementations and runtime components.

pub mod agent;
pub mod cli;
pub mod config;
pub mod context;
mod model_specs;
pub mod settings;
pub mod shell_manager;
pub mod store;
pub mod tape;
pub mod tape_viewer;
pub mod tools;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use conduit::ConduitError;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::builtin::agent::Agent;
use crate::builtin::store::FileTapeStore;
use crate::channels::message::{ChannelMessage, MessageKind};
use crate::hooks::{EliHookSpec, TapeStoreKind};
use crate::smart_router::{RouteDecision, SmartRouter};
use crate::tool_middleware::MiddlewareChain;
use crate::types::{Envelope, PromptValue, State};

pub(crate) const CLEANUP_ONLY_CONTEXT_KEY: &str = "_eli_cleanup_only";

// ---------------------------------------------------------------------------
// BuiltinImpl — default hook implementations
// ---------------------------------------------------------------------------

/// Default hook implementations for basic runtime operations.
pub struct BuiltinImpl {
    agent: Mutex<Agent>,
    home: PathBuf,
    router: SmartRouter,
    middleware_chain: MiddlewareChain,
}

#[allow(clippy::new_without_default)]
impl BuiltinImpl {
    /// Create a new `BuiltinImpl`, registering builtin tools.
    pub fn new() -> Self {
        tools::register_builtin_tools();
        let agent = Agent::new();
        let home = agent.settings.home.clone();
        Self {
            agent: Mutex::new(agent),
            home,
            router: SmartRouter::new(),
            middleware_chain: MiddlewareChain::with_defaults(),
        }
    }

    /// Resolve a session ID from a channel message.
    pub fn resolve_session(&self, message: &ChannelMessage) -> String {
        if !message.session_id.trim().is_empty() {
            return message.session_id.clone();
        }
        let channel = &message.channel;
        let chat_id = &message.chat_id;
        format!("{channel}:{chat_id}")
    }

    /// Load initial state for a session.
    pub fn load_state(&self, session_id: &str) -> HashMap<String, Value> {
        let mut state: HashMap<String, Value> = HashMap::new();
        state.insert(
            "session_id".to_owned(),
            Value::String(session_id.to_owned()),
        );
        let workspace = std::env::current_dir()
            .unwrap_or_default()
            .display()
            .to_string();
        state.insert("_runtime_workspace".to_owned(), Value::String(workspace));
        state
    }

    /// Build a prompt from an inbound message.
    pub fn build_prompt(&self, message: &ChannelMessage) -> PromptValue {
        let content = extract_message_text(&message.content);
        if content.starts_with('/') {
            return PromptValue::Text(content);
        }
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let context_str = message.context_str();
        let text = if context_str.is_empty() {
            content
        } else {
            format!("{context_str}\n---Date: {now}---\n{content}")
        };
        PromptValue::Text(text)
    }

    /// Run the model on a prompt within a session.
    pub async fn run_model(
        &self,
        prompt: PromptValue,
        session_id: &str,
        state: &HashMap<String, Value>,
    ) -> Result<String, ConduitError> {
        self.agent
            .lock()
            .await
            .run(session_id, prompt, state, None, None, None)
            .await
    }

    /// Provide the tape store (FileTapeStore backed by the agent's home directory).
    pub fn provide_tape_store(&self) -> FileTapeStore {
        FileTapeStore::new(self.home.join("tapes"))
    }

    /// Handle errors by logging them.
    pub async fn on_error(&self, stage: &str, error: &str, message: Option<&ChannelMessage>) {
        tracing::error!(stage = stage, error = error, "pipeline error");
        if let Some(msg) = message {
            tracing::error!(
                session_id = %msg.session_id,
                channel = %msg.channel,
                "error occurred in session"
            );
        }
    }

    /// Render outbound messages from model output.
    pub fn render_outbound(
        &self,
        message: &ChannelMessage,
        session_id: &str,
        model_output: &str,
    ) -> Vec<ChannelMessage> {
        let output_channel = if message.output_channel.is_empty() {
            message.channel.as_str()
        } else {
            message.output_channel.as_str()
        };
        let clean = crate::builtin::cli::strip_fake_tool_calls(model_output);
        if clean.trim().is_empty() {
            tracing::info!(
                target: "eli_trace",
                session_id = %session_id,
                raw_model_output = ?model_output,
                "builtin.render_outbound.empty_after_cleanup"
            );
            let mut extra = message.context.clone();
            extra.insert(CLEANUP_ONLY_CONTEXT_KEY.to_owned(), Value::Bool(true));
            let outbound = ChannelMessage::new(session_id, &message.channel, "")
                .with_chat_id(&message.chat_id)
                .with_output_channel(output_channel)
                .with_kind(message.kind)
                .with_context(extra)
                .finalize();
            return vec![outbound];
        }

        let outbound = ChannelMessage::new(session_id, &message.channel, clean)
            .with_chat_id(&message.chat_id)
            .with_output_channel(output_channel)
            .with_kind(message.kind)
            .with_context(message.context.clone())
            .finalize();
        vec![outbound]
    }
}

fn extract_message_text(content: &str) -> String {
    match serde_json::from_str::<Value>(content) {
        Ok(val) => val
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or(content)
            .to_owned(),
        Err(_) => content.to_owned(),
    }
}

fn envelope_to_channel_message(message: &Envelope) -> ChannelMessage {
    let channel = message
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("cli")
        .to_owned();

    let kind = match message
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("normal")
    {
        "error" => MessageKind::Error,
        "command" => MessageKind::Command,
        _ => MessageKind::Normal,
    };

    ChannelMessage {
        session_id: message
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        channel: channel.clone(),
        content: match message.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        },
        chat_id: message
            .get("chat_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_owned(),
        is_active: message
            .get("is_active")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        kind,
        context: message
            .get("context")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default(),
        media: Vec::new(),
        output_channel: message
            .get("output_channel")
            .and_then(|v| v.as_str())
            .unwrap_or(&channel)
            .to_owned(),
    }
}

#[async_trait]
impl EliHookSpec for BuiltinImpl {
    fn plugin_name(&self) -> &str {
        "builtin"
    }

    fn classify_inbound(&self, message: &Envelope) -> Option<RouteDecision> {
        let content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        self.router.classify(content)
    }

    async fn resolve_session(
        &self,
        message: &Envelope,
    ) -> Result<Option<String>, crate::hooks::HookError> {
        Ok(Some(
            self.resolve_session(&envelope_to_channel_message(message)),
        ))
    }

    async fn load_state(
        &self,
        message: &Envelope,
        session_id: &str,
    ) -> Result<Option<State>, crate::hooks::HookError> {
        let mut state = self.load_state(session_id);
        for field in ["sender_id", "chat_id", "channel", "output_channel"] {
            if let Some(value) = message.get(field).cloned() {
                state.insert(field.to_owned(), value);
            }
        }
        Ok(Some(state))
    }

    async fn build_user_prompt(
        &self,
        message: &Envelope,
        _session_id: &str,
        _state: &State,
    ) -> Option<PromptValue> {
        Some(self.build_prompt(&envelope_to_channel_message(message)))
    }

    async fn run_model(
        &self,
        prompt: &PromptValue,
        session_id: &str,
        state: &State,
    ) -> Result<Option<String>, crate::hooks::HookError> {
        match self.run_model(prompt.clone(), session_id, state).await {
            Ok(output) => Ok(Some(output)),
            Err(e) => {
                tracing::error!(error = %e, session_id = %session_id, "run_model failed");
                Err(crate::hooks::HookError::Plugin {
                    plugin: self.plugin_name().to_owned(),
                    hook_point: "run_model",
                    source: e.into(),
                })
            }
        }
    }

    async fn render_outbound(
        &self,
        message: &Envelope,
        session_id: &str,
        _state: &State,
        model_output: &str,
    ) -> Option<Vec<Envelope>> {
        let outbounds = self.render_outbound(
            &envelope_to_channel_message(message),
            session_id,
            model_output,
        );
        Some(
            outbounds
                .into_iter()
                .filter_map(|message| serde_json::to_value(message).ok())
                .collect(),
        )
    }

    async fn on_error(&self, stage: &str, error: &anyhow::Error, message: Option<&Envelope>) {
        let channel_message = message.map(envelope_to_channel_message);
        self.on_error(stage, &error.to_string(), channel_message.as_ref())
            .await;
    }

    fn build_system_prompt(&self, prompt_text: &str, state: &State) -> Option<String> {
        Some(Agent::new().system_prompt(prompt_text, state, None))
    }

    fn wrap_tool(&self, tool: &conduit::Tool) -> conduit::ToolAction {
        let wrapped = self.middleware_chain.wrap_tools(std::slice::from_ref(tool));
        match wrapped.into_iter().next() {
            Some(t) => conduit::ToolAction::Replace(t),
            None => conduit::ToolAction::Remove,
        }
    }

    fn provide_tape_store(&self) -> Option<TapeStoreKind> {
        Some(TapeStoreKind::Sync(Arc::new(self.provide_tape_store())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_outbound_empty_emits_cleanup_only_message() {
        let builtin = BuiltinImpl::new();
        let mut extra = serde_json::Map::new();
        extra.insert(
            "source_channel".to_owned(),
            Value::String("feishu".to_owned()),
        );
        let message = ChannelMessage::new("feishu:default:user_1", "webhook", "hello")
            .with_chat_id("user_1")
            .with_context(extra)
            .finalize();

        let outbounds = builtin.render_outbound(&message, "feishu:default:user_1", "");

        assert_eq!(outbounds.len(), 1);
        assert!(outbounds[0].content.is_empty());
        assert_eq!(
            outbounds[0]
                .context
                .get(CLEANUP_ONLY_CONTEXT_KEY)
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            outbounds[0]
                .context
                .get("source_channel")
                .and_then(|v| v.as_str()),
            Some("feishu")
        );
    }

    #[test]
    fn test_render_outbound_normal_propagates_inbound_context() {
        let builtin = BuiltinImpl::new();
        let mut extra = serde_json::Map::new();
        extra.insert(
            "source_channel".to_owned(),
            Value::String("feishu".to_owned()),
        );
        extra.insert("account_id".to_owned(), Value::String("default".to_owned()));
        extra.insert(
            "channel_target".to_owned(),
            Value::String("user:ou_abc".to_owned()),
        );
        let message = ChannelMessage::new("feishu:default:ou_abc", "webhook", "hello")
            .with_chat_id("ou_abc")
            .with_context(extra)
            .finalize();

        let outbounds = builtin.render_outbound(&message, "feishu:default:ou_abc", "reply text");

        assert_eq!(outbounds.len(), 1);
        assert_eq!(outbounds[0].content, "reply text");
        // Inbound context must be propagated so the sidecar can route
        // the outbound correctly and clean up typing indicators.
        assert_eq!(
            outbounds[0]
                .context
                .get("source_channel")
                .and_then(|v| v.as_str()),
            Some("feishu"),
            "normal outbound must carry source_channel from inbound context"
        );
        assert_eq!(
            outbounds[0]
                .context
                .get("account_id")
                .and_then(|v| v.as_str()),
            Some("default"),
        );
        assert_eq!(
            outbounds[0]
                .context
                .get("channel_target")
                .and_then(|v| v.as_str()),
            Some("user:ou_abc"),
        );
    }
}
