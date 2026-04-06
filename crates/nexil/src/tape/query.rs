//! Query helpers for tape entries.

use crate::core::errors::ConduitError;
use crate::tape::entries::{TapeEntry, TapeEntryKind};
use crate::tape::store::{AsyncTapeStore, TapeStore};

/// Builder for querying tape entries. All setters consume and return `Self` for chaining.
#[derive(Debug, Clone)]
pub struct TapeQuery {
    pub tape: String,
    pub query_text: Option<String>,
    pub after_anchor: Option<String>,
    pub after_last: bool,
    pub between_anchors: Option<(String, String)>,
    pub between_dates: Option<(String, String)>,
    pub kinds: Vec<TapeEntryKind>,
    pub limit: Option<usize>,
}

impl TapeQuery {
    /// Create a new query targeting the given tape name.
    pub fn new(tape: impl Into<String>) -> Self {
        Self {
            tape: tape.into(),
            query_text: None,
            after_anchor: None,
            after_last: false,
            between_anchors: None,
            between_dates: None,
            kinds: Vec::new(),
            limit: None,
        }
    }

    /// Set a free-text search filter.
    pub fn query(mut self, value: impl Into<String>) -> Self {
        self.query_text = Some(value.into());
        self
    }

    /// Restrict to entries after the named anchor.
    pub fn after_anchor(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if name.is_empty() {
            self.after_anchor = None;
            self.after_last = false;
        } else {
            self.after_anchor = Some(name);
            self.after_last = false;
        }
        self
    }

    /// Restrict to entries after the most recent anchor.
    pub fn last_anchor(mut self) -> Self {
        self.after_anchor = None;
        self.after_last = true;
        self
    }

    /// Restrict to entries between two named anchors.
    pub fn between_anchors(mut self, start: impl Into<String>, end: impl Into<String>) -> Self {
        self.between_anchors = Some((start.into(), end.into()));
        self
    }

    /// Restrict to entries within a date range (ISO date or datetime strings).
    pub fn between_dates(mut self, start: impl Into<String>, end: impl Into<String>) -> Self {
        self.between_dates = Some((start.into(), end.into()));
        self
    }

    /// Restrict to entries of the given kinds.
    pub fn kinds(mut self, kinds: Vec<TapeEntryKind>) -> Self {
        self.kinds = kinds;
        self
    }

    /// Limit the number of returned entries.
    pub fn limit(mut self, value: usize) -> Self {
        self.limit = Some(value);
        self
    }

    /// Execute the query against a sync TapeStore.
    pub fn all_sync(&self, store: &dyn TapeStore) -> Result<Vec<TapeEntry>, ConduitError> {
        store.fetch_all(self)
    }

    /// Execute the query against an async TapeStore.
    pub async fn all_async(
        &self,
        store: &dyn AsyncTapeStore,
    ) -> Result<Vec<TapeEntry>, ConduitError> {
        store.fetch_all(self).await
    }
}
