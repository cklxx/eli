//! Tape stores for Conduit.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};

use crate::core::errors::{ConduitError, ErrorKind};
use crate::tape::entries::TapeEntry;
use crate::tape::query::TapeQuery;

// ---------------------------------------------------------------------------
// TapeStore (sync)
// ---------------------------------------------------------------------------

/// Append-only tape storage interface (synchronous).
pub trait TapeStore: Send + Sync {
    fn list_tapes(&self) -> Result<Vec<String>, ConduitError>;
    fn reset(&self, tape: &str) -> Result<(), ConduitError>;
    fn fetch_all(&self, query: &TapeQuery) -> Result<Vec<TapeEntry>, ConduitError>;
    fn append(&self, tape: &str, entry: &TapeEntry) -> Result<(), ConduitError>;
}

// ---------------------------------------------------------------------------
// AsyncTapeStore
// ---------------------------------------------------------------------------

/// Append-only tape storage interface (asynchronous).
#[async_trait]
pub trait AsyncTapeStore: Send + Sync {
    async fn list_tapes(&self) -> Result<Vec<String>, ConduitError>;
    async fn reset(&self, tape: &str) -> Result<(), ConduitError>;
    async fn fetch_all(&self, query: &TapeQuery) -> Result<Vec<TapeEntry>, ConduitError>;
    async fn append(&self, tape: &str, entry: &TapeEntry) -> Result<(), ConduitError>;
}

// ---------------------------------------------------------------------------
// Anchor helpers
// ---------------------------------------------------------------------------

fn is_matching_anchor(entry: &TapeEntry, name: Option<&str>) -> bool {
    entry.kind == "anchor"
        && name
            .map(|n| entry.payload.get("name").and_then(|v| v.as_str()) == Some(n))
            .unwrap_or(true)
}

fn anchor_index(
    entries: &[TapeEntry],
    name: Option<&str>,
    default: i64,
    forward: bool,
    start: usize,
) -> i64 {
    let mut range = start..entries.len();
    if forward {
        range
            .find(|&i| is_matching_anchor(&entries[i], name))
            .map(|i| i as i64)
            .unwrap_or(default)
    } else {
        range
            .rev()
            .find(|&i| is_matching_anchor(&entries[i], name))
            .map(|i| i as i64)
            .unwrap_or(default)
    }
}

fn boundary_time(is_end: bool) -> NaiveTime {
    if is_end {
        NaiveTime::from_hms_nano_opt(23, 59, 59, 999_999_999)
            .expect("SAFETY: static time components are always valid")
    } else {
        NaiveTime::MIN
    }
}

fn try_parse_date_only(value: &str, is_end: bool) -> Option<DateTime<Utc>> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .map(|d| d.and_time(boundary_time(is_end)).and_utc())
}

fn parse_datetime_boundary(value: &str, is_end: bool) -> Result<DateTime<Utc>, ConduitError> {
    if !value.contains('T')
        && !value.contains(' ')
        && let Some(dt) = try_parse_date_only(value, is_end)
    {
        return Ok(dt);
    }

    DateTime::parse_from_rfc3339(value)
        .map(|p| p.with_timezone(&Utc))
        .or_else(|_| value.parse::<DateTime<Utc>>())
        .or_else(|_| {
            try_parse_date_only(value, is_end).ok_or_else(|| {
                ConduitError::new(
                    ErrorKind::InvalidInput,
                    format!("Invalid ISO date or datetime: '{value}'."),
                )
            })
        })
}

fn entry_in_datetime_range(
    entry: &TapeEntry,
    start_dt: &DateTime<Utc>,
    end_dt: &DateTime<Utc>,
) -> Result<bool, ConduitError> {
    let entry_dt = parse_datetime_boundary(&entry.date, false)?;
    Ok(&entry_dt >= start_dt && &entry_dt <= end_dt)
}

fn entry_matches_query(entry: &TapeEntry, query: &str) -> bool {
    let needle = query.to_lowercase();
    let haystack_obj = serde_json::json!({
        "kind": entry.kind,
        "date": entry.date,
        "payload": entry.payload,
        "meta": entry.meta,
    });
    let haystack = serde_json::to_string(&haystack_obj)
        .unwrap_or_default()
        .to_lowercase();
    haystack.contains(&needle)
}

// ---------------------------------------------------------------------------
// In-memory query logic
// ---------------------------------------------------------------------------

/// Execute a query against an in-memory list of entries.
pub fn fetch_all_in_memory(
    entries: &[TapeEntry],
    query: &TapeQuery,
) -> Result<Vec<TapeEntry>, ConduitError> {
    let mut start_index: usize = 0;
    let mut end_index: Option<usize> = None;

    if let Some((ref start_name, ref end_name)) = query.between_anchors {
        let start_idx = anchor_index(entries, Some(start_name), -1, false, 0);
        if start_idx < 0 {
            return Err(ConduitError::new(
                ErrorKind::NotFound,
                format!("Anchor '{}' was not found.", start_name),
            ));
        }
        let end_idx = anchor_index(entries, Some(end_name), -1, true, (start_idx + 1) as usize);
        if end_idx < 0 {
            return Err(ConduitError::new(
                ErrorKind::NotFound,
                format!("Anchor '{}' was not found.", end_name),
            ));
        }
        start_index = ((start_idx + 1) as usize).min(entries.len());
        end_index = Some(start_index.max(end_idx as usize).min(entries.len()));
    } else if query.after_last {
        let idx = anchor_index(entries, None, -1, false, 0);
        if idx < 0 {
            return Err(ConduitError::new(
                ErrorKind::NotFound,
                "No anchors found in tape.",
            ));
        }
        start_index = ((idx + 1) as usize).min(entries.len());
    } else if let Some(ref after_name) = query.after_anchor {
        let idx = anchor_index(entries, Some(after_name), -1, false, 0);
        if idx < 0 {
            return Err(ConduitError::new(
                ErrorKind::NotFound,
                format!("Anchor '{}' was not found.", after_name),
            ));
        }
        start_index = ((idx + 1) as usize).min(entries.len());
    }

    let sliced: Vec<TapeEntry> = match end_index {
        Some(end) => entries[start_index..end].to_vec(),
        None => entries[start_index..].to_vec(),
    };

    let mut result = sliced;

    if let Some((ref start_date, ref end_date)) = query.between_dates {
        let start_dt = parse_datetime_boundary(start_date, false)?;
        let end_dt = parse_datetime_boundary(end_date, true)?;
        if start_dt > end_dt {
            return Err(ConduitError::new(
                ErrorKind::InvalidInput,
                "Start date must be earlier than or equal to end date.",
            ));
        }
        result.retain(|e| entry_in_datetime_range(e, &start_dt, &end_dt).unwrap_or(false));
    }

    if let Some(ref q) = query.query_text {
        result.retain(|e| entry_matches_query(e, q));
    }

    if !query.kinds.is_empty() {
        result.retain(|e| query.kinds.contains(&e.kind));
    }

    if let Some(limit) = query.limit {
        result.truncate(limit);
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// InMemoryTapeStore
// ---------------------------------------------------------------------------

/// In-memory tape storage (thread-safe via RwLock).
#[derive(Debug, Clone)]
pub struct InMemoryTapeStore {
    tapes: Arc<RwLock<HashMap<String, Vec<TapeEntry>>>>,
    next_ids: Arc<RwLock<HashMap<String, i64>>>,
}

impl InMemoryTapeStore {
    pub fn new() -> Self {
        Self {
            tapes: Arc::new(RwLock::new(HashMap::new())),
            next_ids: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn read(&self, tape: &str) -> Option<Vec<TapeEntry>> {
        let tapes = self.tapes.read().unwrap_or_else(|e| e.into_inner());
        tapes
            .get(tape)
            .map(|entries| entries.iter().map(|e| e.copy()).collect())
    }
}

impl Default for InMemoryTapeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TapeStore for InMemoryTapeStore {
    fn list_tapes(&self) -> Result<Vec<String>, ConduitError> {
        let tapes = self.tapes.read().unwrap_or_else(|e| e.into_inner());
        let mut keys: Vec<String> = tapes.keys().cloned().collect();
        keys.sort_unstable();
        Ok(keys)
    }

    fn reset(&self, tape: &str) -> Result<(), ConduitError> {
        // Lock next_ids before tapes — same order as append() to prevent deadlock.
        let mut ids = self.next_ids.write().unwrap_or_else(|e| e.into_inner());
        ids.remove(tape);
        let mut tapes = self.tapes.write().unwrap_or_else(|e| e.into_inner());
        tapes.remove(tape);
        Ok(())
    }

    fn fetch_all(&self, query: &TapeQuery) -> Result<Vec<TapeEntry>, ConduitError> {
        let entries = self.read(&query.tape).unwrap_or_default();
        fetch_all_in_memory(&entries, query)
    }

    fn append(&self, tape: &str, entry: &TapeEntry) -> Result<(), ConduitError> {
        let mut ids = self.next_ids.write().unwrap_or_else(|e| e.into_inner());
        let next_id = ids.get(tape).copied().unwrap_or(1);
        ids.insert(tape.into(), next_id + 1);

        let stored = TapeEntry::new(
            next_id,
            entry.kind.clone(),
            entry.payload.clone(),
            entry.meta.clone(),
            entry.date.clone(),
        );

        let mut tapes = self.tapes.write().unwrap_or_else(|e| e.into_inner());
        let entries = tapes.entry(tape.into()).or_default();
        entries.push(stored);

        const MAX_ENTRIES_PER_TAPE: usize = 10_000;
        if entries.len() > MAX_ENTRIES_PER_TAPE {
            let excess = entries.len() - MAX_ENTRIES_PER_TAPE;
            entries.drain(..excess);
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AsyncTapeStoreAdapter
// ---------------------------------------------------------------------------

/// Adapt a sync TapeStore to AsyncTapeStore by running operations in a blocking task.
pub struct AsyncTapeStoreAdapter<S: TapeStore + 'static> {
    store: Arc<S>,
}

impl<S: TapeStore + 'static> AsyncTapeStoreAdapter<S> {
    pub fn new(store: S) -> Self {
        Self {
            store: Arc::new(store),
        }
    }

    pub fn from_arc(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S: TapeStore + 'static> AsyncTapeStore for AsyncTapeStoreAdapter<S> {
    async fn list_tapes(&self) -> Result<Vec<String>, ConduitError> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.list_tapes())
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Unknown, format!("blocking task failed: {e}"))
            })?
    }

    async fn reset(&self, tape: &str) -> Result<(), ConduitError> {
        let store = self.store.clone();
        let tape = tape.to_string();
        tokio::task::spawn_blocking(move || store.reset(&tape))
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Unknown, format!("blocking task failed: {e}"))
            })?
    }

    async fn fetch_all(&self, query: &TapeQuery) -> Result<Vec<TapeEntry>, ConduitError> {
        let store = self.store.clone();
        let query = query.clone();
        tokio::task::spawn_blocking(move || store.fetch_all(&query))
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Unknown, format!("blocking task failed: {e}"))
            })?
    }

    async fn append(&self, tape: &str, entry: &TapeEntry) -> Result<(), ConduitError> {
        let store = self.store.clone();
        let tape = tape.to_string();
        let entry = entry.clone();
        tokio::task::spawn_blocking(move || store.append(&tape, &entry))
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Unknown, format!("blocking task failed: {e}"))
            })?
    }
}

// ---------------------------------------------------------------------------
// UnavailableTapeStore
// ---------------------------------------------------------------------------

/// Sync TapeStore sentinel that always fails with a clear message.
pub struct UnavailableTapeStore {
    message: String,
}

impl UnavailableTapeStore {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn raise(&self) -> ConduitError {
        ConduitError::new(ErrorKind::InvalidInput, self.message.clone())
    }
}

impl TapeStore for UnavailableTapeStore {
    fn list_tapes(&self) -> Result<Vec<String>, ConduitError> {
        Err(self.raise())
    }

    fn reset(&self, _tape: &str) -> Result<(), ConduitError> {
        Err(self.raise())
    }

    fn fetch_all(&self, _query: &TapeQuery) -> Result<Vec<TapeEntry>, ConduitError> {
        Err(self.raise())
    }

    fn append(&self, _tape: &str, _entry: &TapeEntry) -> Result<(), ConduitError> {
        Err(self.raise())
    }
}
