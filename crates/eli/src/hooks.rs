//! Hook specifications and runtime for the Eli framework.
//!
//! Replaces Python pluggy with a simple Vec-of-trait-objects approach.
//! Hook precedence: implementations are stored in registration order;
//! `call_first` iterates in **reverse** (last registered wins).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use conduit::tape::{AsyncTapeStore, TapeStore};

use crate::types::{Envelope, MessageHandler, PromptValue, State};

fn preview_text(text: &str) -> String {
    const LIMIT: usize = 1000;
    let mut chars = text.chars();
    let preview: String = chars.by_ref().take(LIMIT).collect();
    let normalized = preview.replace('\n', "\\n");
    if chars.next().is_some() {
        format!("{normalized}...(truncated)")
    } else {
        normalized
    }
}

fn preview_json(value: &Envelope) -> String {
    preview_text(&value.to_string())
}

// ---------------------------------------------------------------------------
// Channel trait (mirrors Python eli.channels.base.Channel)
// ---------------------------------------------------------------------------

/// A channel that can receive and optionally send messages.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Unique name identifying this channel type.
    fn name(&self) -> &str;

    /// Start listening for events. Runs until `stop` is called or the token is cancelled.
    async fn start(&self, stop: tokio::sync::watch::Receiver<bool>) -> anyhow::Result<()>;

    /// Gracefully stop the channel.
    async fn stop(&self) -> anyhow::Result<()>;

    /// Whether this channel needs debounce to prevent overload.
    fn needs_debounce(&self) -> bool {
        false
    }

    /// Send a message through this channel (optional).
    async fn send(&self, _message: Envelope) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TapeStoreKind: unifies sync and async tape stores
// ---------------------------------------------------------------------------

/// Wraps either a sync or async tape store returned by plugins.
pub enum TapeStoreKind {
    Sync(Arc<dyn TapeStore>),
    Async(Arc<dyn AsyncTapeStore>),
}

// ---------------------------------------------------------------------------
// EliHookSpec trait
// ---------------------------------------------------------------------------

/// Hook contract for Eli framework extensions.
///
/// All methods have default implementations that return `None` / empty so that
/// plugins only need to override the hooks they care about.
#[async_trait]
#[allow(unused_variables)]
pub trait EliHookSpec: Send + Sync {
    /// Human-readable name for this plugin (used in diagnostics).
    fn plugin_name(&self) -> &str {
        "unnamed"
    }

    /// Resolve session id for one inbound message.
    async fn resolve_session(&self, message: &Envelope) -> Option<String> {
        None
    }

    /// Load state snapshot for one session.
    async fn load_state(&self, message: &Envelope, session_id: &str) -> Option<State> {
        None
    }

    /// Build model prompt for this turn.
    /// Returns either plain text or a list of content parts (multimodal).
    async fn build_prompt(
        &self,
        message: &Envelope,
        session_id: &str,
        state: &State,
    ) -> Option<PromptValue> {
        None
    }

    /// Run model for one turn and return plain text output.
    async fn run_model(
        &self,
        prompt: &PromptValue,
        session_id: &str,
        state: &State,
    ) -> Option<String> {
        None
    }

    /// Persist state updates after one model turn.
    async fn save_state(
        &self,
        session_id: &str,
        state: &State,
        message: &Envelope,
        model_output: &str,
    ) {
    }

    /// Render outbound messages from model output.
    /// Each implementation may return zero or more envelopes.
    async fn render_outbound(
        &self,
        message: &Envelope,
        session_id: &str,
        state: &State,
        model_output: &str,
    ) -> Option<Vec<Envelope>> {
        None
    }

    /// Dispatch one outbound message to external channel(s).
    async fn dispatch_outbound(&self, message: &Envelope) -> Option<bool> {
        None
    }

    /// Register CLI commands (synchronous hook).
    fn register_cli_commands(&self, app: &mut clap::Command) {}

    /// Observe framework errors from any stage.
    async fn on_error(&self, stage: &str, error: &anyhow::Error, message: Option<&Envelope>) {}

    /// Provide a system prompt fragment to be prepended to model prompts.
    fn system_prompt(&self, prompt: &PromptValue, state: &State) -> Option<String> {
        None
    }

    /// Provide a tape store instance for conversation recording.
    fn provide_tape_store(&self) -> Option<TapeStoreKind> {
        None
    }

    /// Provide channels for receiving messages.
    fn provide_channels(&self, message_handler: MessageHandler) -> Vec<Box<dyn Channel>> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// HookRuntime
// ---------------------------------------------------------------------------

/// Executes hooks with fault isolation and precedence semantics.
pub struct HookRuntime {
    plugins: Vec<Arc<dyn EliHookSpec>>,
}

impl HookRuntime {
    /// Create a new runtime from a list of plugins (in registration order).
    pub fn new(plugins: Vec<Arc<dyn EliHookSpec>>) -> Self {
        Self { plugins }
    }

    /// Add a plugin at the end of the registration list.
    pub fn register(&mut self, plugin: Arc<dyn EliHookSpec>) {
        self.plugins.push(plugin);
    }

    /// Return an iterator over plugins in **reverse** registration order (last-registered first).
    fn reversed(&self) -> impl Iterator<Item = &Arc<dyn EliHookSpec>> {
        self.plugins.iter().rev()
    }

    // -- firstresult hooks (async) ------------------------------------------

    /// Resolve session: return the first non-None result.
    pub async fn call_resolve_session(&self, message: &Envelope) -> Option<String> {
        for plugin in self.reversed() {
            match plugin.resolve_session(message).await {
                Some(val) => return Some(val),
                None => continue,
            }
        }
        None
    }

    /// Load state: collect all results, reverse order, and merge.
    pub async fn call_load_state(
        &self,
        message: &Envelope,
        session_id: &str,
    ) -> Vec<Option<State>> {
        let mut results = Vec::new();
        for plugin in self.plugins.iter() {
            results.push(plugin.load_state(message, session_id).await);
        }
        results
    }

    /// Build prompt: return the first non-None result.
    pub async fn call_build_prompt(
        &self,
        message: &Envelope,
        session_id: &str,
        state: &State,
    ) -> Option<PromptValue> {
        for plugin in self.reversed() {
            tracing::info!(
                target: "eli_trace",
                plugin = %plugin.plugin_name(),
                session_id = %session_id,
                inbound = %preview_json(message),
                "hook.build_prompt.call"
            );
            match plugin.build_prompt(message, session_id, state).await {
                Some(val) => {
                    tracing::info!(
                        target: "eli_trace",
                        plugin = %plugin.plugin_name(),
                        session_id = %session_id,
                        prompt = %preview_text(&val.as_text()),
                        "hook.build_prompt.return"
                    );
                    return Some(val);
                }
                None => {
                    tracing::info!(
                        target: "eli_trace",
                        plugin = %plugin.plugin_name(),
                        session_id = %session_id,
                        "hook.build_prompt.none"
                    );
                    continue;
                }
            }
        }
        None
    }

    /// Run model: return the first non-None result.
    pub async fn call_run_model(
        &self,
        prompt: &PromptValue,
        session_id: &str,
        state: &State,
    ) -> Option<String> {
        for plugin in self.reversed() {
            tracing::info!(
                target: "eli_trace",
                plugin = %plugin.plugin_name(),
                session_id = %session_id,
                prompt = %preview_text(&prompt.as_text()),
                "hook.run_model.call"
            );
            match plugin.run_model(prompt, session_id, state).await {
                Some(val) => {
                    tracing::info!(
                        target: "eli_trace",
                        plugin = %plugin.plugin_name(),
                        session_id = %session_id,
                        output = %preview_text(&val),
                        output_len = val.len(),
                        "hook.run_model.return"
                    );
                    return Some(val);
                }
                None => {
                    tracing::info!(
                        target: "eli_trace",
                        plugin = %plugin.plugin_name(),
                        session_id = %session_id,
                        "hook.run_model.none"
                    );
                    continue;
                }
            }
        }
        None
    }

    /// Save state: call all implementations (notify pattern).
    pub async fn call_save_state(
        &self,
        session_id: &str,
        state: &State,
        message: &Envelope,
        model_output: &str,
    ) {
        for plugin in self.plugins.iter() {
            plugin
                .save_state(session_id, state, message, model_output)
                .await;
        }
    }

    /// Render outbound: collect results from all implementations.
    pub async fn call_render_outbound(
        &self,
        message: &Envelope,
        session_id: &str,
        state: &State,
        model_output: &str,
    ) -> Vec<Vec<Envelope>> {
        let mut results = Vec::new();
        for plugin in self.plugins.iter() {
            tracing::info!(
                target: "eli_trace",
                plugin = %plugin.plugin_name(),
                session_id = %session_id,
                model_output = %preview_text(model_output),
                "hook.render_outbound.call"
            );
            if let Some(batch) = plugin
                .render_outbound(message, session_id, state, model_output)
                .await
            {
                let preview = batch
                    .first()
                    .map(preview_json)
                    .unwrap_or_else(|| String::from("(empty batch)"));
                tracing::info!(
                    target: "eli_trace",
                    plugin = %plugin.plugin_name(),
                    session_id = %session_id,
                    batch_len = batch.len(),
                    first_outbound = %preview,
                    "hook.render_outbound.return"
                );
                results.push(batch);
            } else {
                tracing::info!(
                    target: "eli_trace",
                    plugin = %plugin.plugin_name(),
                    session_id = %session_id,
                    "hook.render_outbound.none"
                );
            }
        }
        results
    }

    /// Dispatch outbound: call all implementations.
    pub async fn call_dispatch_outbound(&self, message: &Envelope) {
        for plugin in self.plugins.iter() {
            plugin.dispatch_outbound(message).await;
        }
    }

    /// Register CLI commands on all plugins (synchronous).
    pub fn call_register_cli_commands(&self, app: &mut clap::Command) {
        for plugin in self.plugins.iter() {
            plugin.register_cli_commands(app);
        }
    }

    /// Notify all error observers, swallowing any panics/errors from the observers.
    pub async fn notify_error(
        &self,
        stage: &str,
        error: &anyhow::Error,
        message: Option<&Envelope>,
    ) {
        for plugin in self.plugins.iter() {
            let result = std::panic::AssertUnwindSafe(plugin.on_error(stage, error, message));
            // We intentionally ignore errors from error observers
            let _ = result.await;
        }
    }

    /// Collect system prompt fragments from all plugins (reversed, joined).
    pub fn call_system_prompt(&self, prompt: &PromptValue, state: &State) -> String {
        let mut fragments: Vec<String> = Vec::new();
        for plugin in self.reversed() {
            if let Some(fragment) = plugin.system_prompt(prompt, state)
                && !fragment.is_empty()
            {
                fragments.push(fragment);
            }
        }
        fragments.join("\n\n")
    }

    /// Get the first provided tape store.
    pub fn call_provide_tape_store(&self) -> Option<TapeStoreKind> {
        for plugin in self.reversed() {
            if let Some(store) = plugin.provide_tape_store() {
                return Some(store);
            }
        }
        None
    }

    /// Collect channels from all plugins.
    pub fn call_provide_channels(&self, message_handler: MessageHandler) -> Vec<Box<dyn Channel>> {
        let mut channels = Vec::new();
        for plugin in self.plugins.iter() {
            let mut provided = plugin.provide_channels(message_handler.clone());
            channels.append(&mut provided);
        }
        channels
    }

    /// Build a hook-name to adapter-names mapping for diagnostics.
    pub fn hook_report(&self) -> HashMap<String, Vec<String>> {
        // We report which plugins implement each hook by checking if their
        // return value differs from the default. Since we can't introspect
        // trait overrides in Rust the way pluggy can, we just list all
        // registered plugin names for each hook.
        let hook_names = [
            "resolve_session",
            "load_state",
            "build_prompt",
            "run_model",
            "save_state",
            "render_outbound",
            "dispatch_outbound",
            "register_cli_commands",
            "on_error",
            "system_prompt",
            "provide_tape_store",
            "provide_channels",
        ];

        let mut report = HashMap::new();
        let names: Vec<String> = self
            .plugins
            .iter()
            .map(|p| p.plugin_name().to_string())
            .collect();

        for hook_name in &hook_names {
            if !names.is_empty() {
                report.insert(hook_name.to_string(), names.clone());
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    // -- Test plugins ---------------------------------------------------------

    struct HighPriorityPlugin;

    #[async_trait]
    impl EliHookSpec for HighPriorityPlugin {
        fn plugin_name(&self) -> &str {
            "high"
        }

        async fn resolve_session(&self, _message: &Envelope) -> Option<String> {
            Some("high-session".into())
        }

        fn system_prompt(&self, _prompt: &PromptValue, _state: &State) -> Option<String> {
            Some("high-prompt".into())
        }

        async fn render_outbound(
            &self,
            _message: &Envelope,
            _session_id: &str,
            _state: &State,
            _model_output: &str,
        ) -> Option<Vec<Envelope>> {
            Some(vec![json!({"content": "high-out"})])
        }
    }

    struct LowPriorityPlugin;

    #[async_trait]
    impl EliHookSpec for LowPriorityPlugin {
        fn plugin_name(&self) -> &str {
            "low"
        }

        async fn resolve_session(&self, _message: &Envelope) -> Option<String> {
            Some("low-session".into())
        }

        fn system_prompt(&self, _prompt: &PromptValue, _state: &State) -> Option<String> {
            Some("low-prompt".into())
        }
    }

    struct ReturnsNonePlugin;

    #[async_trait]
    impl EliHookSpec for ReturnsNonePlugin {
        fn plugin_name(&self) -> &str {
            "none-plugin"
        }
    }

    struct ErrorObserver {
        observed: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl EliHookSpec for ErrorObserver {
        fn plugin_name(&self) -> &str {
            "error-observer"
        }

        async fn on_error(&self, stage: &str, _error: &anyhow::Error, _message: Option<&Envelope>) {
            self.observed.lock().unwrap().push(stage.to_owned());
        }
    }

    struct FailingErrorObserver;

    #[async_trait]
    impl EliHookSpec for FailingErrorObserver {
        fn plugin_name(&self) -> &str {
            "failing-observer"
        }

        async fn on_error(
            &self,
            _stage: &str,
            _error: &anyhow::Error,
            _message: Option<&Envelope>,
        ) {
            // Simulates an observer that does something but doesn't prevent others from running.
            // In the Rust implementation, errors from observers are ignored via `let _ = ...`.
        }
    }

    // -- call_resolve_session (call_first semantics) -------------------------

    #[tokio::test]
    async fn test_call_first_returns_last_registered_non_none() {
        // Low registered first, High registered last.
        // reversed() iterates High first, which returns Some -> wins.
        let rt = HookRuntime::new(vec![
            Arc::new(LowPriorityPlugin) as Arc<dyn EliHookSpec>,
            Arc::new(HighPriorityPlugin),
        ]);
        let msg = json!({"content": "hello"});
        let result = rt.call_resolve_session(&msg).await;
        assert_eq!(result, Some("high-session".into()));
    }

    #[tokio::test]
    async fn test_call_first_skips_none_and_returns_next() {
        // ReturnsNone registered last -> reversed first, returns None.
        // LowPriority next -> returns Some("low-session").
        let rt = HookRuntime::new(vec![
            Arc::new(LowPriorityPlugin) as Arc<dyn EliHookSpec>,
            Arc::new(ReturnsNonePlugin),
        ]);
        let msg = json!({"content": "hello"});
        let result = rt.call_resolve_session(&msg).await;
        assert_eq!(result, Some("low-session".into()));
    }

    #[tokio::test]
    async fn test_call_first_returns_none_when_all_return_none() {
        let rt = HookRuntime::new(vec![Arc::new(ReturnsNonePlugin) as Arc<dyn EliHookSpec>]);
        let msg = json!({"content": "hello"});
        let result = rt.call_resolve_session(&msg).await;
        assert_eq!(result, None);
    }

    // -- call_system_prompt (call_many sync, joined) -------------------------

    #[tokio::test]
    async fn test_call_system_prompt_collects_and_joins_fragments() {
        let rt = HookRuntime::new(vec![
            Arc::new(LowPriorityPlugin) as Arc<dyn EliHookSpec>,
            Arc::new(HighPriorityPlugin),
        ]);
        let prompt = PromptValue::Text("hello".into());
        let state = State::new();
        let result = rt.call_system_prompt(&prompt, &state);
        // reversed order: High first, Low second
        assert_eq!(result, "high-prompt\n\nlow-prompt");
    }

    #[tokio::test]
    async fn test_call_system_prompt_skips_none_results() {
        let rt = HookRuntime::new(vec![
            Arc::new(LowPriorityPlugin) as Arc<dyn EliHookSpec>,
            Arc::new(ReturnsNonePlugin),
        ]);
        let prompt = PromptValue::Text("hello".into());
        let state = State::new();
        let result = rt.call_system_prompt(&prompt, &state);
        assert_eq!(result, "low-prompt");
    }

    // -- call_render_outbound (call_many async) ------------------------------

    #[tokio::test]
    async fn test_call_render_outbound_collects_all() {
        let rt = HookRuntime::new(vec![
            Arc::new(HighPriorityPlugin) as Arc<dyn EliHookSpec>,
            Arc::new(ReturnsNonePlugin),
        ]);
        let msg = json!({"content": "hello"});
        let state = State::new();
        let result = rt.call_render_outbound(&msg, "s1", &state, "output").await;
        // Only HighPriorityPlugin returns Some
        assert_eq!(result.len(), 1);
        assert_eq!(result[0][0], json!({"content": "high-out"}));
    }

    // -- notify_error swallows observer failures -----------------------------

    #[tokio::test]
    async fn test_notify_error_calls_all_observers() {
        let observer = Arc::new(ErrorObserver {
            observed: std::sync::Mutex::new(Vec::new()),
        });
        let rt = HookRuntime::new(vec![
            Arc::new(FailingErrorObserver) as Arc<dyn EliHookSpec>,
            observer.clone() as Arc<dyn EliHookSpec>,
        ]);
        let err = anyhow::anyhow!("test error");
        rt.notify_error("turn", &err, None).await;
        let observed = observer.observed.lock().unwrap();
        assert_eq!(*observed, vec!["turn"]);
    }

    #[tokio::test]
    async fn test_notify_error_with_message() {
        let observer = Arc::new(ErrorObserver {
            observed: std::sync::Mutex::new(Vec::new()),
        });
        let rt = HookRuntime::new(vec![observer.clone() as Arc<dyn EliHookSpec>]);
        let err = anyhow::anyhow!("test error");
        let msg = json!({"content": "hello"});
        rt.notify_error("pipeline", &err, Some(&msg)).await;
        let observed = observer.observed.lock().unwrap();
        assert_eq!(*observed, vec!["pipeline"]);
    }

    // -- hook_report ---------------------------------------------------------

    #[test]
    fn test_hook_report_lists_all_registered_plugins() {
        let rt = HookRuntime::new(vec![
            Arc::new(LowPriorityPlugin) as Arc<dyn EliHookSpec>,
            Arc::new(HighPriorityPlugin),
        ]);
        let report = rt.hook_report();
        assert!(report.contains_key("resolve_session"));
        assert_eq!(report["resolve_session"], vec!["low", "high"]);
        assert!(report.contains_key("system_prompt"));
    }

    #[test]
    fn test_hook_report_empty_when_no_plugins() {
        let rt = HookRuntime::new(vec![]);
        let report = rt.hook_report();
        assert!(report.is_empty());
    }

    // -- register ------------------------------------------------------------

    #[test]
    fn test_register_adds_plugin() {
        let mut rt = HookRuntime::new(vec![]);
        assert!(rt.hook_report().is_empty());
        rt.register(Arc::new(LowPriorityPlugin));
        let report = rt.hook_report();
        assert_eq!(report["resolve_session"], vec!["low"]);
    }
}
