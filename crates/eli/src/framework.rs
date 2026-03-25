//! Hook-first Eli framework runtime.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use nexil::CancellationToken;

use crate::control_plane::{BudgetLedger, TurnContext, with_turn_context};
use crate::envelope::{ValueExt, unpack_batch_vec};
use crate::hooks::{ChannelHook, EliHookSpec, HookRuntime, TapeStoreKind};
use crate::types::{
    Envelope, MessageHandler, OutboundChannelRouter, PromptValue, State, TurnResult,
};

// ---------------------------------------------------------------------------
// PluginStatus
// ---------------------------------------------------------------------------

/// Records whether a plugin loaded successfully.
#[derive(Debug, Clone)]
pub struct PluginStatus {
    pub is_success: bool,
    pub detail: Option<String>,
}

// ---------------------------------------------------------------------------
// EliFramework
// ---------------------------------------------------------------------------

/// Minimal framework core. Everything grows from hook plugins.
pub struct EliFramework {
    /// Working directory / project root.
    pub workspace: RwLock<PathBuf>,
    /// The hook runtime that dispatches to registered plugins.
    hook_runtime: RwLock<HookRuntime>,
    /// Status of each loaded plugin.
    plugin_status: RwLock<HashMap<String, PluginStatus>>,
    /// Optional router for outbound message dispatch.
    outbound_router: RwLock<Option<Arc<dyn OutboundChannelRouter>>>,
    /// Token budget ledger (control plane infrastructure).
    pub budget: BudgetLedger,
}

impl EliFramework {
    /// Create a new framework instance rooted at the current working directory.
    pub fn new() -> Self {
        Self {
            workspace: RwLock::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
            hook_runtime: RwLock::new(HookRuntime::new(Vec::new())),
            plugin_status: RwLock::new(HashMap::new()),
            outbound_router: RwLock::new(None),
            budget: BudgetLedger::new(),
        }
    }

    /// Create a framework with a given workspace path.
    pub fn with_workspace(workspace: PathBuf) -> Self {
        Self {
            workspace: RwLock::new(workspace),
            hook_runtime: RwLock::new(HookRuntime::new(Vec::new())),
            plugin_status: RwLock::new(HashMap::new()),
            outbound_router: RwLock::new(None),
            budget: BudgetLedger::new(),
        }
    }

    // -- Plugin registration ------------------------------------------------

    /// Register a plugin with the framework.
    pub async fn register_plugin(&self, name: &str, plugin: Arc<dyn EliHookSpec>) {
        let mut rt = self.hook_runtime.write().await;
        rt.register(plugin);
        let mut status = self.plugin_status.write().await;
        status.insert(
            name.to_string(),
            PluginStatus {
                is_success: true,
                detail: None,
            },
        );
    }

    /// Record a plugin load failure.
    pub async fn record_plugin_failure(&self, name: &str, detail: String) {
        let mut status = self.plugin_status.write().await;
        status.insert(
            name.to_string(),
            PluginStatus {
                is_success: false,
                detail: Some(detail),
            },
        );
    }

    /// Load hooks from a list of plugins. This is the Rust equivalent of
    /// the Python `load_hooks` which discovers plugins via entry points.
    /// In Rust, callers explicitly provide the plugin list.
    pub async fn load_hooks(&self, plugins: Vec<(String, Arc<dyn EliHookSpec>)>) {
        for (name, plugin) in plugins {
            self.register_plugin(&name, plugin).await;
        }
    }

    // -- Main orchestration loop --------------------------------------------

    /// Run one inbound message through all hooks and return the turn result.
    pub async fn process_inbound(&self, mut inbound: Envelope) -> anyhow::Result<TurnResult> {
        Self::strip_internal_context(&mut inbound);

        let rt = self.hook_runtime.read().await;
        let workspace = self.workspace.read().await.clone();

        if let Some(result) = Self::try_greet_shortcircuit(&rt, &inbound) {
            return Ok(result);
        }

        // Build turn context: cancellation token + tool wrapping from hooks.
        let turn_ctx = TurnContext {
            cancellation: CancellationToken::new(),
            wrap_tools: Some(rt.wrap_tools_fn()),
        };

        with_turn_context(turn_ctx, async {
            let session_id = self.resolve_session_id(&rt, &inbound).await;
            Self::inject_session_id(&mut inbound, &session_id);

            let state = self
                .build_state(&rt, &inbound, &session_id, &workspace)
                .await;
            let prompt = Self::build_prompt(&rt, &inbound, &session_id, &state).await;
            let model_output = Self::run_model(&rt, &prompt, &session_id, &state, &inbound).await;

            rt.call_save_state(&session_id, &state, &inbound, &model_output)
                .await;

            let outbounds = self
                .collect_outbounds(&rt, &inbound, &session_id, &state, &model_output)
                .await;
            for outbound in &outbounds {
                rt.call_dispatch_outbound(outbound).await;
            }

            Ok(TurnResult {
                session_id,
                prompt,
                model_output,
                outbounds,
            })
        })
        .await
    }

    fn strip_internal_context(inbound: &mut Envelope) {
        if let Some(ctx) = inbound.get_mut("context").and_then(|v| v.as_object_mut()) {
            ctx.remove("_eli_cleanup_only");
        }
    }

    fn try_greet_shortcircuit(rt: &HookRuntime, inbound: &Envelope) -> Option<TurnResult> {
        let crate::smart_router::RouteDecision::Greet(reply) = rt.call_classify_inbound(inbound)?;

        let session_id = Self::default_session_id(inbound);
        let channel = inbound.field_str("channel", "");
        let chat_id = inbound.field_str("chat_id", "");
        let output_channel = inbound
            .get("output_channel")
            .and_then(|v| v.as_str())
            .unwrap_or(&channel)
            .to_owned();

        let outbound = serde_json::json!({
            "content": reply,
            "session_id": session_id,
            "channel": channel,
            "chat_id": chat_id,
            "output_channel": output_channel,
        });
        Some(TurnResult {
            session_id,
            prompt: PromptValue::Text(String::new()),
            model_output: reply.clone(),
            outbounds: vec![outbound],
        })
    }

    async fn resolve_session_id(&self, rt: &HookRuntime, inbound: &Envelope) -> String {
        match rt.call_resolve_session(inbound).await {
            Ok(Some(id)) => id,
            Ok(None) => Self::default_session_id(inbound),
            Err(e) => {
                tracing::warn!(error = %e, "resolve_session failed, using default session id");
                Self::default_session_id(inbound)
            }
        }
    }

    fn inject_session_id(inbound: &mut Envelope, session_id: &str) {
        if let Some(obj) = inbound.as_object_mut() {
            obj.entry("session_id")
                .or_insert_with(|| Value::String(session_id.to_owned()));
        }
    }

    async fn build_state(
        &self,
        rt: &HookRuntime,
        inbound: &Envelope,
        session_id: &str,
        workspace: &std::path::Path,
    ) -> State {
        let mut state: State = HashMap::new();
        state.insert(
            "_runtime_workspace".to_string(),
            Value::String(workspace.to_string_lossy().to_string()),
        );

        let hook_states = match rt.call_load_state(inbound, session_id).await {
            Ok(states) => states,
            Err(e) => {
                tracing::warn!(error = %e, session_id = %session_id, "load_state failed, using empty state");
                Vec::new()
            }
        };
        for hs in hook_states.into_iter().rev().flatten() {
            for (k, v) in hs {
                state.entry(k).or_insert(v);
            }
        }
        state
    }

    async fn build_prompt(
        rt: &HookRuntime,
        inbound: &Envelope,
        session_id: &str,
        state: &State,
    ) -> PromptValue {
        rt.call_build_user_prompt(inbound, session_id, state)
            .await
            .unwrap_or_else(|| PromptValue::Text(inbound.content_text()))
    }

    async fn run_model(
        rt: &HookRuntime,
        prompt: &PromptValue,
        session_id: &str,
        state: &State,
        inbound: &Envelope,
    ) -> String {
        match rt.call_run_model(prompt, session_id, state).await {
            Ok(Some(output)) => output,
            Ok(None) => {
                let err = anyhow::anyhow!("no model skill returned output");
                rt.notify_error("run_model:fallback", &err, Some(inbound))
                    .await;
                "[Error: no model plugin available]".to_owned()
            }
            Err(e) => {
                let err_msg = format!("{e}");
                let err = anyhow::anyhow!("run_model hook failed: {}", err_msg);
                rt.notify_error("run_model", &err, Some(inbound)).await;
                format!("[Error: {err_msg}]")
            }
        }
    }

    // -- Diagnostics --------------------------------------------------------

    /// Return hook implementation summary for diagnostics.
    pub async fn hook_report(&self) -> HashMap<String, Vec<String>> {
        let rt = self.hook_runtime.read().await;
        rt.hook_report()
    }

    /// Return the plugin status map.
    pub async fn plugin_status(&self) -> HashMap<String, PluginStatus> {
        self.plugin_status.read().await.clone()
    }

    // -- Outbound router ----------------------------------------------------

    /// Bind an outbound channel router.
    pub async fn bind_outbound_router(&self, router: Arc<dyn OutboundChannelRouter>) {
        let mut r = self.outbound_router.write().await;
        *r = Some(router);
    }

    /// Dispatch a message through the bound outbound router.
    pub async fn dispatch_via_router(&self, message: &Envelope) -> bool {
        let router = self.outbound_router.read().await;
        match router.as_ref() {
            Some(r) => r.dispatch(message.clone()).await,
            None => false,
        }
    }

    /// Signal a session to quit through the bound outbound router.
    pub async fn quit_via_router(&self, session_id: &str) {
        let router = self.outbound_router.read().await;
        if let Some(r) = router.as_ref() {
            r.quit(session_id).await;
        }
    }

    // -- Channel and tape store accessors -----------------------------------

    /// Collect channels from all plugins, deduplicating by name.
    pub async fn get_channels(
        &self,
        message_handler: MessageHandler,
    ) -> HashMap<String, Box<dyn ChannelHook>> {
        let rt = self.hook_runtime.read().await;
        let all_channels = rt.call_provide_channels(message_handler);
        let mut map: HashMap<String, Box<dyn ChannelHook>> = HashMap::new();
        for ch in all_channels {
            let name = ch.name().to_string();
            map.entry(name).or_insert(ch);
        }
        map
    }

    /// Get the first provided tape store.
    pub async fn get_tape_store(&self) -> Option<TapeStoreKind> {
        let rt = self.hook_runtime.read().await;
        rt.call_provide_tape_store()
    }

    /// Build the system prompt via the hook chain.
    pub async fn get_system_prompt(&self, prompt: &PromptValue, state: &State) -> String {
        let rt = self.hook_runtime.read().await;
        rt.call_build_system_prompt(&prompt.as_text(), state)
            .unwrap_or_default()
    }

    // -- Internal helpers ---------------------------------------------------

    /// Compute a default session ID from the envelope fields.
    fn default_session_id(message: &Envelope) -> String {
        // Try session_id field first
        if let Some(obj) = message.as_object()
            && let Some(Value::String(sid)) = obj.get("session_id")
        {
            return sid.clone();
        }
        let channel = message.field_str("channel", "default");
        let chat_id = message.field_str("chat_id", "default");
        format!("{channel}:{chat_id}")
    }

    /// Collect outbound envelopes from render hooks, falling back to a default
    /// envelope containing the model output.
    async fn collect_outbounds(
        &self,
        rt: &HookRuntime,
        message: &Envelope,
        session_id: &str,
        state: &State,
        model_output: &str,
    ) -> Vec<Envelope> {
        let batches = rt
            .call_render_outbound(message, session_id, state, model_output)
            .await;
        if !batches.is_empty() {
            return unpack_batch_vec(batches);
        }

        // Fallback: build a default outbound envelope
        let mut fallback = serde_json::Map::new();
        fallback.insert(
            "content".to_string(),
            Value::String(model_output.to_string()),
        );
        fallback.insert(
            "session_id".to_string(),
            Value::String(session_id.to_string()),
        );

        if let Some(channel) = message.field("channel", None) {
            fallback.insert("channel".to_string(), channel);
        }
        if let Some(chat_id) = message.field("chat_id", None) {
            fallback.insert("chat_id".to_string(), chat_id);
        }

        vec![Value::Object(fallback)]
    }
}

impl Default for EliFramework {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{EliHookSpec, HookError};
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    struct SessionPlugin;

    #[async_trait]
    impl EliHookSpec for SessionPlugin {
        fn plugin_name(&self) -> &str {
            "session-plugin"
        }

        async fn resolve_session(&self, message: &Envelope) -> Result<Option<String>, HookError> {
            Ok(message
                .as_object()
                .and_then(|o| o.get("session_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned()))
        }
    }

    struct PromptPlugin {
        fragment: String,
    }

    #[async_trait]
    impl EliHookSpec for PromptPlugin {
        fn plugin_name(&self) -> &str {
            "prompt-plugin"
        }

        fn build_system_prompt(&self, _prompt_text: &str, _state: &State) -> Option<String> {
            Some(self.fragment.clone())
        }
    }

    struct ModelPlugin;

    #[async_trait]
    impl EliHookSpec for ModelPlugin {
        fn plugin_name(&self) -> &str {
            "model-plugin"
        }

        async fn run_model(
            &self,
            prompt: &PromptValue,
            _session_id: &str,
            _state: &State,
        ) -> Result<Option<String>, HookError> {
            Ok(Some(format!("echo: {}", prompt.as_text())))
        }
    }

    // -- Creation tests -------------------------------------------------------

    #[tokio::test]
    async fn test_framework_creation_default() {
        let fw = EliFramework::new();
        let ws = fw.workspace.read().await;
        assert!(ws.is_absolute() || *ws == std::path::PathBuf::from("."));
    }

    #[tokio::test]
    async fn test_framework_with_workspace() {
        let fw = EliFramework::with_workspace("/tmp/test".into());
        let ws = fw.workspace.read().await;
        assert_eq!(*ws, std::path::PathBuf::from("/tmp/test"));
    }

    // -- Plugin registration --------------------------------------------------

    #[tokio::test]
    async fn test_register_plugin_records_status() {
        let fw = EliFramework::new();
        fw.register_plugin("test", Arc::new(SessionPlugin)).await;
        let status = fw.plugin_status().await;
        assert!(status.contains_key("test"));
        assert!(status["test"].is_success);
    }

    #[tokio::test]
    async fn test_record_plugin_failure() {
        let fw = EliFramework::new();
        fw.record_plugin_failure("bad", "failed to load".into())
            .await;
        let status = fw.plugin_status().await;
        assert!(!status["bad"].is_success);
        assert_eq!(status["bad"].detail.as_deref(), Some("failed to load"));
    }

    #[tokio::test]
    async fn test_load_hooks_registers_multiple_plugins() {
        let fw = EliFramework::new();
        fw.load_hooks(vec![
            (
                "session".into(),
                Arc::new(SessionPlugin) as Arc<dyn EliHookSpec>,
            ),
            (
                "model".into(),
                Arc::new(ModelPlugin) as Arc<dyn EliHookSpec>,
            ),
        ])
        .await;
        let status = fw.plugin_status().await;
        assert_eq!(status.len(), 2);
        assert!(status["session"].is_success);
        assert!(status["model"].is_success);
    }

    // -- get_system_prompt ----------------------------------------------------

    #[tokio::test]
    async fn test_get_system_prompt_returns_first_result() {
        let fw = EliFramework::new();
        fw.register_plugin(
            "low",
            Arc::new(PromptPlugin {
                fragment: "low".into(),
            }),
        )
        .await;
        fw.register_plugin(
            "high",
            Arc::new(PromptPlugin {
                fragment: "high".into(),
            }),
        )
        .await;
        let prompt = PromptValue::Text("hello".into());
        let state = State::new();
        let result = fw.get_system_prompt(&prompt, &state).await;
        // Last-registered (high) wins
        assert_eq!(result, "high");
    }

    // -- process_inbound (full pipeline) --------------------------------------

    #[tokio::test]
    async fn test_process_inbound_full_pipeline() {
        let fw = EliFramework::new();
        fw.register_plugin("session", Arc::new(SessionPlugin) as Arc<dyn EliHookSpec>)
            .await;
        fw.register_plugin("model", Arc::new(ModelPlugin) as Arc<dyn EliHookSpec>)
            .await;

        let msg = json!({"content": "ping", "session_id": "test-session"});
        let result = fw.process_inbound(msg).await.unwrap();
        assert_eq!(result.session_id, "test-session");
        assert!(result.model_output.starts_with("echo: "));
        assert!(!result.outbounds.is_empty());
    }

    #[tokio::test]
    async fn test_process_inbound_default_session_id() {
        let fw = EliFramework::new();
        fw.register_plugin("model", Arc::new(ModelPlugin) as Arc<dyn EliHookSpec>)
            .await;

        let msg = json!({"content": "hello", "channel": "telegram", "chat_id": "42"});
        let result = fw.process_inbound(msg).await.unwrap();
        assert_eq!(result.session_id, "telegram:42");
    }

    // -- hook_report ----------------------------------------------------------

    #[tokio::test]
    async fn test_hook_report_reflects_registered_plugins() {
        let fw = EliFramework::new();
        fw.register_plugin("session", Arc::new(SessionPlugin) as Arc<dyn EliHookSpec>)
            .await;
        let report = fw.hook_report().await;
        assert!(report.contains_key("resolve_session"));
        assert!(report["resolve_session"].contains(&"session-plugin".to_owned()));
    }
}
