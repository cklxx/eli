//! Tape manager helpers for Conduit.

use serde_json::Value;

use crate::core::errors::ConduitError;
use crate::core::results::ErrorPayload;
use crate::tape::context::{TapeContext, build_messages};
use crate::tape::entries::TapeEntry;
use crate::tape::query::TapeQuery;
use crate::tape::store::{AsyncTapeStore, AsyncTapeStoreAdapter, InMemoryTapeStore, TapeStore};

// ---------------------------------------------------------------------------
// TapeManager (sync)
// ---------------------------------------------------------------------------

/// Global tape manager that owns storage and default context.
pub struct TapeManager {
    store: Box<dyn TapeStore>,
    default_context: TapeContext,
}

impl TapeManager {
    /// Create a new TapeManager with optional store and default context.
    pub fn new(store: Option<Box<dyn TapeStore>>, default_context: Option<TapeContext>) -> Self {
        Self {
            store: store.unwrap_or_else(|| Box::new(InMemoryTapeStore::new())),
            default_context: default_context.unwrap_or_default(),
        }
    }

    /// Get a reference to the default context.
    pub fn default_context(&self) -> &TapeContext {
        &self.default_context
    }

    /// Set the default context.
    pub fn set_default_context(&mut self, ctx: TapeContext) {
        self.default_context = ctx;
    }

    /// List all tape names.
    pub fn list_tapes(&self) -> Result<Vec<String>, ConduitError> {
        self.store.list_tapes()
    }

    /// Read messages from a tape, applying the given or default context.
    pub fn read_messages(
        &self,
        tape: &str,
        context: Option<&TapeContext>,
    ) -> Result<Vec<Value>, ConduitError> {
        let active_context = context.unwrap_or(&self.default_context);
        let query = self.query_tape(tape);
        let query = active_context.build_query(query);
        let entries = self.store.fetch_all(&query)?;
        Ok(build_messages(&entries, active_context))
    }

    /// Append an entry to a tape.
    pub fn append_entry(&self, tape: &str, entry: &TapeEntry) -> Result<(), ConduitError> {
        self.store.append(tape, entry)
    }

    /// Create a new TapeQuery for the given tape.
    pub fn query_tape(&self, tape: &str) -> TapeQuery {
        TapeQuery::new(tape)
    }

    /// Reset (delete) a tape.
    pub fn reset_tape(&self, tape: &str) -> Result<(), ConduitError> {
        self.store.reset(tape)
    }

    /// Record a handoff: creates an anchor and handoff event, appends both.
    pub fn handoff(
        &self,
        tape: &str,
        name: &str,
        state: Option<Value>,
        meta: Value,
    ) -> Result<Vec<TapeEntry>, ConduitError> {
        let entry = TapeEntry::anchor(name, state.clone(), meta.clone());
        let handoff_data = serde_json::json!({
            "name": name,
            "state": state.clone().unwrap_or_else(|| serde_json::json!({})),
        });
        let event = TapeEntry::event("handoff", Some(handoff_data), meta);
        self.store.append(tape, &entry)?;
        self.store.append(tape, &event)?;
        Ok(vec![entry, event])
    }

    /// Record a complete chat turn to the tape.
    #[allow(clippy::too_many_arguments)]
    pub fn record_chat(
        &self,
        tape: &str,
        run_id: &str,
        system_prompt: Option<&str>,
        context_error: Option<&ErrorPayload>,
        new_messages: &[Value],
        response_text: Option<&str>,
        tool_calls: Option<&[Value]>,
        tool_results: Option<&[Value]>,
        error: Option<&ErrorPayload>,
        usage: Option<Value>,
        provider: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), ConduitError> {
        let meta = serde_json::json!({ "run_id": run_id });

        if let Some(sp) = system_prompt
            && !sp.is_empty()
        {
            self.store
                .append(tape, &TapeEntry::system(sp, meta.clone()))?;
        }

        if let Some(ce) = context_error {
            self.store
                .append(tape, &TapeEntry::error(ce, meta.clone()))?;
        }

        for message in new_messages {
            self.store
                .append(tape, &TapeEntry::message(message.clone(), meta.clone()))?;
        }

        if let Some(tc) = tool_calls
            && !tc.is_empty()
        {
            self.store
                .append(tape, &TapeEntry::tool_call(tc.to_vec(), meta.clone()))?;
        }

        if let Some(tr) = tool_results {
            self.store
                .append(tape, &TapeEntry::tool_result(tr.to_vec(), meta.clone()))?;
        }

        // Only append error if it is different from context_error
        if let Some(err) = error {
            let is_duplicate = context_error.is_some_and(|ce| ce == err);
            if !is_duplicate {
                self.store
                    .append(tape, &TapeEntry::error(err, meta.clone()))?;
            }
        }

        if let Some(rt) = response_text {
            let msg = serde_json::json!({ "role": "assistant", "content": rt });
            self.store
                .append(tape, &TapeEntry::message(msg, meta.clone()))?;
        }

        let mut data = serde_json::Map::new();
        data.insert(
            "status".into(),
            Value::String(if error.is_some() { "error" } else { "ok" }.into()),
        );
        if let Some(u) = usage {
            data.insert("usage".into(), u);
        }
        if let Some(p) = provider
            && !p.is_empty()
        {
            data.insert("provider".into(), Value::String(p.into()));
        }
        if let Some(m) = model
            && !m.is_empty()
        {
            data.insert("model".into(), Value::String(m.into()));
        }
        self.store.append(
            tape,
            &TapeEntry::event("run", Some(Value::Object(data)), meta),
        )?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AsyncTapeManager
// ---------------------------------------------------------------------------

/// Async tape manager for async chat and tool-call paths.
pub struct AsyncTapeManager {
    store: Box<dyn AsyncTapeStore>,
    default_context: TapeContext,
}

impl AsyncTapeManager {
    /// Create a new AsyncTapeManager. Accepts either an AsyncTapeStore or a sync TapeStore
    /// (which will be wrapped in AsyncTapeStoreAdapter).
    pub fn new(
        store: Option<Box<dyn AsyncTapeStore>>,
        default_context: Option<TapeContext>,
    ) -> Self {
        Self {
            store: store
                .unwrap_or_else(|| Box::new(AsyncTapeStoreAdapter::new(InMemoryTapeStore::new()))),
            default_context: default_context.unwrap_or_default(),
        }
    }

    /// Create from a sync TapeStore, wrapping it automatically.
    pub fn from_sync_store<S: TapeStore + 'static>(
        store: S,
        default_context: Option<TapeContext>,
    ) -> Self {
        Self {
            store: Box::new(AsyncTapeStoreAdapter::new(store)),
            default_context: default_context.unwrap_or_default(),
        }
    }

    /// Get a reference to the default context.
    pub fn default_context(&self) -> &TapeContext {
        &self.default_context
    }

    /// Set the default context.
    pub fn set_default_context(&mut self, ctx: TapeContext) {
        self.default_context = ctx;
    }

    /// Create a TapeQuery for the given tape.
    pub fn query_tape(&self, tape: &str) -> TapeQuery {
        TapeQuery::new(tape)
    }

    /// List all tape names.
    pub async fn list_tapes(&self) -> Result<Vec<String>, ConduitError> {
        self.store.list_tapes().await
    }

    /// Read messages from a tape, applying the given or default context.
    pub async fn read_messages(
        &self,
        tape: &str,
        context: Option<&TapeContext>,
    ) -> Result<Vec<Value>, ConduitError> {
        let active_context = context.unwrap_or(&self.default_context);
        let query = self.query_tape(tape);
        let query = active_context.build_query(query);
        let entries = self.store.fetch_all(&query).await?;
        Ok(build_messages(&entries, active_context))
    }

    /// Fetch raw tape entries matching a query.
    pub async fn fetch_entries(&self, query: &TapeQuery) -> Result<Vec<TapeEntry>, ConduitError> {
        self.store.fetch_all(query).await
    }

    /// Append an entry to a tape.
    pub async fn append_entry(&self, tape: &str, entry: &TapeEntry) -> Result<(), ConduitError> {
        self.store.append(tape, entry).await
    }

    /// Reset (delete) a tape.
    pub async fn reset_tape(&self, tape: &str) -> Result<(), ConduitError> {
        self.store.reset(tape).await
    }

    /// Record a handoff: creates an anchor and handoff event, appends both.
    pub async fn handoff(
        &self,
        tape: &str,
        name: &str,
        state: Option<Value>,
        meta: Value,
    ) -> Result<Vec<TapeEntry>, ConduitError> {
        let entry = TapeEntry::anchor(name, state.clone(), meta.clone());
        let handoff_data = serde_json::json!({
            "name": name,
            "state": state.clone().unwrap_or_else(|| serde_json::json!({})),
        });
        let event = TapeEntry::event("handoff", Some(handoff_data), meta);
        self.store.append(tape, &entry).await?;
        self.store.append(tape, &event).await?;
        Ok(vec![entry, event])
    }

    /// Record a complete chat turn to the tape (async).
    #[allow(clippy::too_many_arguments)]
    pub async fn record_chat(
        &self,
        tape: &str,
        run_id: &str,
        system_prompt: Option<&str>,
        context_error: Option<&ErrorPayload>,
        new_messages: &[Value],
        response_text: Option<&str>,
        tool_calls: Option<&[Value]>,
        tool_results: Option<&[Value]>,
        error: Option<&ErrorPayload>,
        usage: Option<Value>,
        provider: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), ConduitError> {
        let meta = serde_json::json!({ "run_id": run_id });

        if let Some(sp) = system_prompt
            && !sp.is_empty()
        {
            self.store
                .append(tape, &TapeEntry::system(sp, meta.clone()))
                .await?;
        }

        if let Some(ce) = context_error {
            self.store
                .append(tape, &TapeEntry::error(ce, meta.clone()))
                .await?;
        }

        for message in new_messages {
            self.store
                .append(tape, &TapeEntry::message(message.clone(), meta.clone()))
                .await?;
        }

        if let Some(tc) = tool_calls
            && !tc.is_empty()
        {
            self.store
                .append(tape, &TapeEntry::tool_call(tc.to_vec(), meta.clone()))
                .await?;
        }

        if let Some(tr) = tool_results {
            self.store
                .append(tape, &TapeEntry::tool_result(tr.to_vec(), meta.clone()))
                .await?;
        }

        if let Some(err) = error {
            let is_duplicate = context_error.is_some_and(|ce| ce == err);
            if !is_duplicate {
                self.store
                    .append(tape, &TapeEntry::error(err, meta.clone()))
                    .await?;
            }
        }

        if let Some(rt) = response_text {
            let msg = serde_json::json!({ "role": "assistant", "content": rt });
            self.store
                .append(tape, &TapeEntry::message(msg, meta.clone()))
                .await?;
        }

        let mut data = serde_json::Map::new();
        data.insert(
            "status".into(),
            Value::String(if error.is_some() { "error" } else { "ok" }.into()),
        );
        if let Some(u) = usage {
            data.insert("usage".into(), u);
        }
        if let Some(p) = provider
            && !p.is_empty()
        {
            data.insert("provider".into(), Value::String(p.into()));
        }
        if let Some(m) = model
            && !m.is_empty()
        {
            data.insert("model".into(), Value::String(m.into()));
        }
        self.store
            .append(
                tape,
                &TapeEntry::event("run", Some(Value::Object(data)), meta),
            )
            .await?;

        Ok(())
    }
}
