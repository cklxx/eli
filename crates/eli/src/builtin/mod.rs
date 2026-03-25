//! Builtin module — default hook implementations and runtime components.

pub mod agent;
pub mod cli;
pub mod config;
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
use nexil::ConduitError;
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

/// Default hook implementations for basic runtime operations.
pub struct BuiltinImpl {
    agents: std::sync::RwLock<HashMap<String, Arc<Mutex<Agent>>>>,
    home: PathBuf,
    router: SmartRouter,
    middleware_chain: MiddlewareChain,
}

#[allow(clippy::new_without_default)]
impl BuiltinImpl {
    /// Create a new `BuiltinImpl`, registering builtin tools.
    pub fn new() -> Self {
        tools::register_builtin_tools();
        let home = settings::AgentSettings::from_env().home;
        Self {
            agents: std::sync::RwLock::new(HashMap::new()),
            home,
            router: SmartRouter::new(),
            middleware_chain: MiddlewareChain::with_defaults(),
        }
    }

    /// Get or create a per-session Agent, enabling concurrent model execution across sessions.
    fn get_or_create_agent(&self, session_id: &str) -> Arc<Mutex<Agent>> {
        {
            let agents = self.agents.read().unwrap_or_else(|e| e.into_inner());
            if let Some(agent) = agents.get(session_id) {
                return Arc::clone(agent);
            }
        }
        let mut agents = self.agents.write().unwrap_or_else(|e| e.into_inner());
        agents
            .entry(session_id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(Agent::new())))
            .clone()
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
        let agent = self.get_or_create_agent(session_id);
        agent
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
        let output_channel = effective_output_channel(message);
        let clean = crate::builtin::cli::strip_fake_tool_calls(model_output);
        let (content, context) = render_content_and_context(&clean, message, session_id);
        let outbound = ChannelMessage::new(session_id, &message.channel, content)
            .with_chat_id(&message.chat_id)
            .with_output_channel(output_channel)
            .with_kind(message.kind)
            .with_context(context)
            .finalize();
        vec![outbound]
    }
}

fn effective_output_channel(message: &ChannelMessage) -> &str {
    if message.output_channel.is_empty() {
        message.channel.as_str()
    } else {
        message.output_channel.as_str()
    }
}

fn render_content_and_context(
    clean: &str,
    message: &ChannelMessage,
    session_id: &str,
) -> (String, serde_json::Map<String, Value>) {
    if clean.trim().is_empty() {
        tracing::info!(
            target: "eli_trace",
            session_id = %session_id,
            "builtin.render_outbound.empty_after_cleanup"
        );
        let mut extra = message.context.clone();
        extra.insert(CLEANUP_ONLY_CONTEXT_KEY.to_owned(), Value::Bool(true));
        (String::new(), extra)
    } else {
        (clean.to_owned(), message.context.clone())
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

fn envelope_str<'a>(message: &'a Envelope, key: &str, default: &'a str) -> &'a str {
    message.get(key).and_then(|v| v.as_str()).unwrap_or(default)
}

fn envelope_content(message: &Envelope) -> String {
    match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn parse_message_kind(message: &Envelope) -> MessageKind {
    match envelope_str(message, "kind", "normal") {
        "error" => MessageKind::Error,
        "command" => MessageKind::Command,
        _ => MessageKind::Normal,
    }
}

fn envelope_to_channel_message(message: &Envelope) -> ChannelMessage {
    let channel = envelope_str(message, "channel", "cli");
    ChannelMessage {
        session_id: envelope_str(message, "session_id", "").to_owned(),
        channel: channel.to_owned(),
        content: envelope_content(message),
        chat_id: envelope_str(message, "chat_id", "default").to_owned(),
        is_active: message
            .get("is_active")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        kind: parse_message_kind(message),
        context: message
            .get("context")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default(),
        media: Vec::new(),
        output_channel: envelope_str(message, "output_channel", channel).to_owned(),
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
        let text_prompt = self.build_prompt(&envelope_to_channel_message(message));

        // If the envelope carries resolved image content blocks, return a
        // multimodal Parts prompt so the LLM receives them as vision input.
        if let Some(parts) = message.get("media_parts").and_then(|v| v.as_array())
            && !parts.is_empty()
        {
            let mut content =
                vec![serde_json::json!({"type": "text", "text": text_prompt.as_text()})];
            content.extend(parts.iter().cloned());
            return Some(PromptValue::Parts(content));
        }

        Some(text_prompt)
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

    fn wrap_tool(&self, tool: &nexil::Tool) -> nexil::ToolAction {
        let wrapped = self.middleware_chain.wrap_tools(std::slice::from_ref(tool));
        match wrapped.into_iter().next() {
            Some(t) => nexil::ToolAction::Replace(t),
            None => nexil::ToolAction::Remove,
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
    fn test_get_or_create_agent_returns_same_instance_for_same_session() {
        let builtin = BuiltinImpl::new();
        let a1 = builtin.get_or_create_agent("session:1");
        let a2 = builtin.get_or_create_agent("session:1");
        assert!(Arc::ptr_eq(&a1, &a2), "same session must return same Arc");
    }

    #[test]
    fn test_get_or_create_agent_returns_different_instances_for_different_sessions() {
        let builtin = BuiltinImpl::new();
        let a1 = builtin.get_or_create_agent("session:1");
        let a2 = builtin.get_or_create_agent("session:2");
        assert!(
            !Arc::ptr_eq(&a1, &a2),
            "different sessions must return different Arcs"
        );
    }

    #[tokio::test]
    async fn test_concurrent_sessions_do_not_block_each_other() {
        let builtin = Arc::new(BuiltinImpl::new());

        // Lock session:1's agent
        let agent1 = builtin.get_or_create_agent("session:1");
        let guard = agent1.lock().await;

        // session:2 should still be lockable (not blocked by session:1)
        let agent2 = builtin.get_or_create_agent("session:2");
        let try_lock = agent2.try_lock();
        assert!(
            try_lock.is_ok(),
            "session:2 must not be blocked by session:1"
        );

        drop(guard);
    }

    #[tokio::test]
    async fn test_same_session_serializes() {
        let builtin = BuiltinImpl::new();

        let agent = builtin.get_or_create_agent("session:1");
        let _guard = agent.lock().await;

        // Same session should be locked
        let agent_again = builtin.get_or_create_agent("session:1");
        let try_lock = agent_again.try_lock();
        assert!(
            try_lock.is_err(),
            "same session must serialize (lock should be held)"
        );
    }

    #[test]
    fn test_get_or_create_agent_concurrent_creation() {
        use std::thread;

        let builtin = Arc::new(BuiltinImpl::new());
        let mut handles = vec![];

        // Spawn 10 threads all requesting the same session simultaneously
        for _ in 0..10 {
            let b = Arc::clone(&builtin);
            handles.push(thread::spawn(move || b.get_or_create_agent("shared")));
        }

        let agents: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // All must point to the same Arc
        for agent in &agents[1..] {
            assert!(
                Arc::ptr_eq(&agents[0], agent),
                "concurrent creation must converge to single instance"
            );
        }
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

    #[tokio::test]
    async fn test_build_user_prompt_with_media_parts() {
        let builtin = BuiltinImpl::new();
        let envelope = serde_json::json!({
            "session_id": "test",
            "channel": "telegram",
            "chat_id": "123",
            "content": "What is this image?",
            "context": {},
            "kind": "normal",
            "output_channel": "telegram",
            "media_parts": [
                {"type": "image_base64", "mime_type": "image/jpeg", "data": "AQID"}
            ]
        });

        let prompt = builtin
            .build_user_prompt(&envelope, "test", &HashMap::new())
            .await
            .unwrap();

        match prompt {
            PromptValue::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                assert_eq!(parts[0]["type"], "text");
                assert!(
                    parts[0]["text"]
                        .as_str()
                        .unwrap()
                        .contains("What is this image?")
                );
                assert_eq!(parts[1]["type"], "image_base64");
                assert_eq!(parts[1]["data"], "AQID");
            }
            PromptValue::Text(_) => panic!("expected Parts, got Text"),
        }
    }

    #[tokio::test]
    async fn test_build_user_prompt_without_media_returns_text() {
        let builtin = BuiltinImpl::new();
        let envelope = serde_json::json!({
            "session_id": "test",
            "channel": "telegram",
            "chat_id": "123",
            "content": "hello",
            "context": {},
            "kind": "normal",
            "output_channel": "telegram",
        });

        let prompt = builtin
            .build_user_prompt(&envelope, "test", &HashMap::new())
            .await
            .unwrap();

        match prompt {
            PromptValue::Text(t) => assert!(t.contains("hello")),
            PromptValue::Parts(_) => panic!("expected Text, got Parts"),
        }
    }

    #[tokio::test]
    async fn test_build_user_prompt_empty_media_parts_returns_text() {
        let builtin = BuiltinImpl::new();
        let envelope = serde_json::json!({
            "session_id": "test",
            "channel": "telegram",
            "chat_id": "123",
            "content": "no images",
            "context": {},
            "kind": "normal",
            "output_channel": "telegram",
            "media_parts": []
        });

        let prompt = builtin
            .build_user_prompt(&envelope, "test", &HashMap::new())
            .await
            .unwrap();

        match prompt {
            PromptValue::Text(t) => assert!(t.contains("no images")),
            PromptValue::Parts(_) => panic!("expected Text when media_parts is empty"),
        }
    }

    #[tokio::test]
    async fn test_build_user_prompt_multiple_images() {
        let builtin = BuiltinImpl::new();
        let envelope = serde_json::json!({
            "session_id": "test",
            "channel": "telegram",
            "chat_id": "123",
            "content": "compare",
            "context": {},
            "kind": "normal",
            "output_channel": "telegram",
            "media_parts": [
                {"type": "image_base64", "mime_type": "image/png", "data": "A"},
                {"type": "image_base64", "mime_type": "image/jpeg", "data": "B"}
            ]
        });

        let prompt = builtin
            .build_user_prompt(&envelope, "test", &HashMap::new())
            .await
            .unwrap();

        match prompt {
            PromptValue::Parts(parts) => {
                assert_eq!(parts.len(), 3); // text + 2 images
                assert_eq!(parts[0]["type"], "text");
                assert_eq!(parts[1]["mime_type"], "image/png");
                assert_eq!(parts[2]["mime_type"], "image/jpeg");
            }
            PromptValue::Text(_) => panic!("expected Parts with multiple images"),
        }
    }

    #[tokio::test]
    async fn test_build_user_prompt_media_parts_null_returns_text() {
        let builtin = BuiltinImpl::new();
        let envelope = serde_json::json!({
            "session_id": "test",
            "channel": "telegram",
            "chat_id": "123",
            "content": "null media",
            "context": {},
            "kind": "normal",
            "output_channel": "telegram",
            "media_parts": null
        });

        let prompt = builtin
            .build_user_prompt(&envelope, "test", &HashMap::new())
            .await
            .unwrap();

        match prompt {
            PromptValue::Text(t) => assert!(t.contains("null media")),
            PromptValue::Parts(_) => panic!("expected Text when media_parts is null"),
        }
    }

    #[tokio::test]
    async fn test_build_user_prompt_parts_text_extraction() {
        let builtin = BuiltinImpl::new();
        let envelope = serde_json::json!({
            "session_id": "test",
            "channel": "telegram",
            "chat_id": "123",
            "content": "describe this",
            "context": {},
            "kind": "normal",
            "output_channel": "telegram",
            "media_parts": [
                {"type": "image_base64", "mime_type": "image/png", "data": "X"}
            ]
        });

        let prompt = builtin
            .build_user_prompt(&envelope, "test", &HashMap::new())
            .await
            .unwrap();

        // strict_text() should extract only the text part, ignoring image blocks.
        let text = prompt.strict_text();
        assert!(text.contains("describe this"));
        assert!(!text.contains("image_base64"));
        assert!(!text.contains("image/png"));
    }
}
