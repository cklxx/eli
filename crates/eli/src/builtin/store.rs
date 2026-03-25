//! Tape store implementations: ForkTapeStore with context-var fork/merge and
//! FileTapeStore with JSONL persistence.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nexil::tape::store::fetch_all_in_memory;
use nexil::tape::{AsyncTapeStore, AsyncTapeStoreAdapter, InMemoryTapeStore, TapeStore};
use nexil::{ConduitError, ErrorKind, TapeEntry, TapeQuery};
use serde_json::Value;

tokio::task_local! {
    static CURRENT_STORE: InMemoryTapeStore;
    static CURRENT_FORK_TAPE: String;
    static CURRENT_TAPE_WAS_RESET: std::cell::Cell<bool>;
}

// ---------------------------------------------------------------------------
// ForkTapeStore
// ---------------------------------------------------------------------------

/// A tape store that forks writes into an in-memory store and merges them back
/// into the parent on scope exit. Mirrors the Python `ForkTapeStore`.
#[derive(Clone)]
pub struct ForkTapeStore {
    parent: Arc<dyn AsyncTapeStore>,
}

impl ForkTapeStore {
    /// Wrap a sync `TapeStore` (adapted to async) as the parent.
    pub fn from_sync<S: TapeStore + 'static>(store: S) -> Self {
        Self {
            parent: Arc::new(AsyncTapeStoreAdapter::new(store)),
        }
    }

    /// Wrap an existing `AsyncTapeStore` as the parent.
    pub fn from_async(store: Arc<dyn AsyncTapeStore>) -> Self {
        Self { parent: store }
    }

    /// List tapes from the parent store.
    pub async fn list_tapes(&self) -> Result<Vec<String>, ConduitError> {
        self.parent.list_tapes().await
    }

    /// Reset a tape. If inside a fork scope for the same tape, marks reset
    /// instead of propagating immediately.
    pub async fn reset(&self, tape: &str) -> Result<(), ConduitError> {
        let is_fork_tape = CURRENT_FORK_TAPE.try_with(|t| t == tape).unwrap_or(false);
        if is_fork_tape
            && let Ok(()) = CURRENT_STORE.try_with(|store| {
                let _ = store.reset(tape);
            })
        {
            let _ = CURRENT_TAPE_WAS_RESET.try_with(|c| c.set(true));
            return Ok(());
        }
        self.parent.reset(tape).await
    }

    /// Fetch entries, combining parent and forked stores.
    pub async fn fetch_all(&self, query: &TapeQuery) -> Result<Vec<TapeEntry>, ConduitError> {
        let fork_tape = CURRENT_FORK_TAPE.try_with(|t| t.clone()).ok();
        let was_reset = CURRENT_TAPE_WAS_RESET
            .try_with(|c| c.get())
            .unwrap_or(false);

        let is_fork_query = fork_tape.as_deref() == Some(&query.tape);
        let mut parent_entries = if is_fork_query && was_reset {
            Vec::new()
        } else {
            self.parent.fetch_all(query).await.unwrap_or_else(|e| {
                tracing::error!(error = %e, tape = %query.tape, "failed to read parent tape");
                Vec::new()
            })
        };

        let mut fork_entries: Vec<TapeEntry> = Vec::new();
        let _ = CURRENT_STORE.try_with(|store| {
            if let Some(entries) = store.read(&query.tape) {
                for entry in entries {
                    if !query.kinds.is_empty() && !query.kinds.contains(&entry.kind) {
                        continue;
                    }
                    if is_anchor_boundary(&entry, query) {
                        fork_entries.clear();
                        parent_entries.clear();
                        continue;
                    }
                    fork_entries.push(entry);
                }
            }
        });

        parent_entries.extend(fork_entries);
        Ok(parent_entries)
    }

    /// Append an entry to the in-memory fork store (or parent if not in fork scope).
    pub async fn append(&self, tape: &str, entry: &TapeEntry) -> Result<(), ConduitError> {
        let appended = CURRENT_STORE
            .try_with(|store| store.append(tape, entry))
            .ok();
        match appended {
            Some(result) => result,
            None => self.parent.append(tape, entry).await,
        }
    }

    /// Fork writes for `tape` into an in-memory store. On scope end,
    /// merge forked entries back into the parent if `merge_back` is true.
    pub async fn fork<F, T>(&self, tape: &str, merge_back: bool, f: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        let store = InMemoryTapeStore::new();
        let store_clone = store.clone();
        let tape_name = tape.to_owned();

        let result = CURRENT_STORE
            .scope(
                store,
                CURRENT_FORK_TAPE.scope(
                    tape_name.clone(),
                    CURRENT_TAPE_WAS_RESET.scope(std::cell::Cell::new(false), f),
                ),
            )
            .await;

        if merge_back && let Some(entries) = store_clone.read(&tape_name) {
            for entry in &entries {
                if let Err(e) = self.parent.append(&tape_name, entry).await {
                    tracing::error!(error = %e, tape = %tape_name, "failed to merge tape entry");
                }
            }
            if !entries.is_empty() {
                tracing::info!(
                    count = entries.len(),
                    tape = %tape_name,
                    "Merged entries into tape"
                );
            }
        }

        result
    }
}

#[async_trait]
impl AsyncTapeStore for ForkTapeStore {
    async fn list_tapes(&self) -> Result<Vec<String>, ConduitError> {
        ForkTapeStore::list_tapes(self).await
    }

    async fn reset(&self, tape: &str) -> Result<(), ConduitError> {
        ForkTapeStore::reset(self, tape).await
    }

    async fn fetch_all(&self, query: &TapeQuery) -> Result<Vec<TapeEntry>, ConduitError> {
        ForkTapeStore::fetch_all(self, query).await
    }

    async fn append(&self, tape: &str, entry: &TapeEntry) -> Result<(), ConduitError> {
        ForkTapeStore::append(self, tape, entry).await
    }
}

// ---------------------------------------------------------------------------
// FileTapeStore
// ---------------------------------------------------------------------------

/// A `TapeStore` that persists tapes as JSONL files under a directory.
pub struct FileTapeStore {
    directory: PathBuf,
    tape_files: Mutex<HashMap<String, TapeFile>>,
}

impl FileTapeStore {
    pub fn new(directory: PathBuf) -> Self {
        fs::create_dir_all(&directory).ok();
        Self {
            directory,
            tape_files: Mutex::new(HashMap::new()),
        }
    }

    fn tape_file_path(&self, tape: &str) -> PathBuf {
        self.directory.join(format!("{tape}.jsonl"))
    }

    fn with_tape_file<F, R>(&self, tape: &str, f: F) -> R
    where
        F: FnOnce(&mut TapeFile) -> R,
    {
        let mut files = self.tape_files.lock().unwrap_or_else(|e| e.into_inner());
        let tf = files
            .entry(tape.to_owned())
            .or_insert_with(|| TapeFile::new(self.tape_file_path(tape)));
        f(tf)
    }
}

impl TapeStore for FileTapeStore {
    fn list_tapes(&self) -> Result<Vec<String>, ConduitError> {
        let mut result: Vec<String> = fs::read_dir(&self.directory)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let is_jsonl = path.extension().and_then(|e| e.to_str()) == Some("jsonl");
                let stem = path.file_stem().and_then(|s| s.to_str())?;
                (is_jsonl && stem.contains("__")).then(|| stem.to_owned())
            })
            .collect();
        result.sort();
        Ok(result)
    }

    fn reset(&self, tape: &str) -> Result<(), ConduitError> {
        self.with_tape_file(tape, |tf| tf.reset())
    }

    fn fetch_all(&self, query: &TapeQuery) -> Result<Vec<TapeEntry>, ConduitError> {
        let entries = self.with_tape_file(&query.tape, |tf| tf.read());
        if let Some(ref q) = query.query_text {
            let limit = query.limit.unwrap_or(20);
            return Ok(filter_entries(&entries, q, limit));
        }
        fetch_all_in_memory(&entries, query)
    }

    fn append(&self, tape: &str, entry: &TapeEntry) -> Result<(), ConduitError> {
        self.with_tape_file(tape, |tf| tf.append(entry))
    }
}

/// Substring filter for search queries, deduplicating by payload text.
fn filter_entries(entries: &[TapeEntry], query: &str, limit: usize) -> Vec<TapeEntry> {
    let normalized = query.trim().to_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    entries
        .iter()
        .rev()
        .filter(|entry| {
            let text = entry_text(entry).to_lowercase();
            seen.insert(text.clone()) && text.contains(&normalized)
        })
        .take(limit)
        .cloned()
        .collect()
}

/// Extract a text representation of a tape entry for search purposes.
fn entry_text(entry: &TapeEntry) -> String {
    if let Some(text) = entry.payload.get("content").and_then(|v| v.as_str()) {
        return text.to_owned();
    }
    if let Some(text) = entry.payload.get("text").and_then(|v| v.as_str()) {
        return text.to_owned();
    }
    serde_json::to_string(&entry.payload).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// TapeFile
// ---------------------------------------------------------------------------

/// Helper for a single JSONL tape file with caching.
pub struct TapeFile {
    path: PathBuf,
    read_entries: Vec<TapeEntry>,
    read_offset: u64,
}

impl TapeFile {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            read_entries: Vec::new(),
            read_offset: 0,
        }
    }

    fn next_id(&self) -> i64 {
        self.read_entries.last().map(|e| e.id + 1).unwrap_or(1)
    }

    fn reset_cache(&mut self) {
        self.read_entries.clear();
        self.read_offset = 0;
    }

    pub fn reset(&mut self) -> Result<(), ConduitError> {
        if self.path.exists()
            && let Err(e) = fs::remove_file(&self.path)
        {
            tracing::warn!(
                error = %e,
                path = %self.path.display(),
                "failed to remove tape file"
            );
            return Err(ConduitError::new(
                ErrorKind::Unknown,
                format!("failed to remove tape file {}: {e}", self.path.display()),
            ));
        }
        // Clean up the associated spill directory ({tape}.d/) if it exists.
        let spill_dir = self.path.with_extension("d");
        if spill_dir.is_dir()
            && let Err(e) = fs::remove_dir_all(&spill_dir)
        {
            tracing::warn!(
                error = %e,
                path = %spill_dir.display(),
                "failed to remove spill directory"
            );
        }
        self.reset_cache();
        Ok(())
    }

    pub fn read(&mut self) -> Vec<TapeEntry> {
        if !self.path.exists() {
            self.reset_cache();
            return Vec::new();
        }

        let file_size = fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if file_size < self.read_offset {
            self.reset_cache();
        }

        self.read_new_entries(file_size);
        self.read_entries.clone()
    }

    fn read_new_entries(&mut self, file_size: u64) {
        let Ok(file) = fs::File::open(&self.path) else {
            return;
        };
        let reader = BufReader::new(file);
        let mut current_offset: u64 = 0;
        for line_result in reader.lines() {
            let Ok(raw_line) = line_result else {
                continue;
            };
            current_offset += raw_line.len() as u64 + 1;
            if current_offset <= self.read_offset {
                continue;
            }
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(payload) = serde_json::from_str::<Value>(line)
                && let Some(entry) = entry_from_payload(&payload)
            {
                self.read_entries.push(entry);
            }
        }
        self.read_offset = file_size;
    }

    pub fn append(&mut self, entry: &TapeEntry) -> Result<(), ConduitError> {
        let _ = self.read();
        self.ensure_parent_dir()?;

        let stored = TapeEntry::new(
            self.next_id(),
            entry.kind.clone(),
            entry.payload.clone(),
            entry.meta.clone(),
            entry.date.clone(),
        );

        let line = self.write_entry_to_file(&stored)?;
        self.read_entries.push(stored);
        self.read_offset += line.len() as u64;
        Ok(())
    }

    fn ensure_parent_dir(&self) -> Result<(), ConduitError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                ConduitError::new(
                    ErrorKind::Unknown,
                    format!("failed to create tape directory {}: {e}", parent.display()),
                )
            })?;
        }
        Ok(())
    }

    fn write_entry_to_file(&self, entry: &TapeEntry) -> Result<String, ConduitError> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| {
                ConduitError::new(
                    ErrorKind::Unknown,
                    format!(
                        "failed to open tape file {} for append: {e}",
                        self.path.display()
                    ),
                )
            })?;

        let json = serde_json::to_string(entry).map_err(|e| {
            ConduitError::new(
                ErrorKind::Unknown,
                format!(
                    "failed to serialize tape entry for {}: {e}",
                    self.path.display()
                ),
            )
        })?;

        let line = format!("{json}\n");
        file.write_all(line.as_bytes()).map_err(|e| {
            ConduitError::new(
                ErrorKind::Unknown,
                format!(
                    "failed to append tape entry to {}: {e}",
                    self.path.display()
                ),
            )
        })?;
        Ok(line)
    }
}

fn is_anchor_boundary(entry: &TapeEntry, query: &TapeQuery) -> bool {
    if entry.kind != "anchor" {
        return false;
    }
    let anchor_name = entry
        .payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    query.after_last || query.after_anchor.as_deref() == Some(anchor_name)
}

/// Parse a single JSON object into a `TapeEntry`.
fn entry_from_payload(payload: &Value) -> Option<TapeEntry> {
    let obj = payload.as_object()?;
    let id = obj.get("id")?.as_i64()?;
    let kind = obj.get("kind")?.as_str()?.to_owned();
    let entry_payload = obj.get("payload")?.clone();
    let meta = obj
        .get("meta")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    let date = if let Some(d) = obj.get("date").and_then(|v| v.as_str()) {
        d.to_owned()
    } else {
        let ts = obj.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
        chrono::DateTime::from_timestamp(ts as i64, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    };

    Some(TapeEntry::new(id, kind, entry_payload, meta, date))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexil::tape::{InMemoryTapeStore, TapeStore};
    use serde_json::json;

    // -- FileTapeStore JSONL round-trip tests ----------------------------------

    #[test]
    fn test_file_tape_store_append_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileTapeStore::new(tmp.path().to_path_buf());

        let entry = TapeEntry::new(
            0,
            "message".into(),
            json!({"content": "hello"}),
            json!({}),
            "2024-01-01T00:00:00Z".into(),
        );
        store.append("test-tape", &entry).unwrap();

        let query = TapeQuery::new("test-tape");
        let entries = store.fetch_all(&query).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, 1);
        assert_eq!(entries[0].payload["content"], "hello");
    }

    #[test]
    fn test_file_tape_store_monotonic_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileTapeStore::new(tmp.path().to_path_buf());

        for i in 0..3 {
            let entry = TapeEntry::new(
                0,
                "event".into(),
                json!({"name": "step", "data": {"n": i}}),
                json!({}),
                "2024-01-01T00:00:00Z".into(),
            );
            store.append("tape", &entry).unwrap();
        }

        let query = TapeQuery::new("tape");
        let entries = store.fetch_all(&query).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id, 1);
        assert_eq!(entries[1].id, 2);
        assert_eq!(entries[2].id, 3);
    }

    #[test]
    fn test_file_tape_store_reset() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileTapeStore::new(tmp.path().to_path_buf());

        let entry = TapeEntry::new(
            0,
            "message".into(),
            json!({"content": "hi"}),
            json!({}),
            "2024-01-01T00:00:00Z".into(),
        );
        store.append("tape", &entry).unwrap();
        store.reset("tape").unwrap();

        let query = TapeQuery::new("tape");
        let entries = store.fetch_all(&query).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_file_tape_store_list_tapes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileTapeStore::new(tmp.path().to_path_buf());

        // list_tapes only returns tapes with __ in the name
        let entry = TapeEntry::new(
            0,
            "message".into(),
            json!({}),
            json!({}),
            "2024-01-01T00:00:00Z".into(),
        );
        store.append("ns__tape1", &entry).unwrap();
        store.append("ns__tape2", &entry).unwrap();
        store.append("simple", &entry).unwrap();

        let tapes = store.list_tapes().unwrap();
        assert!(tapes.contains(&"ns__tape1".to_owned()));
        assert!(tapes.contains(&"ns__tape2".to_owned()));
        assert!(!tapes.contains(&"simple".to_owned()));
    }

    #[test]
    fn test_file_tape_store_jsonl_persistence() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();

        // Write with one store instance
        {
            let store = FileTapeStore::new(dir.clone());
            let entry = TapeEntry::new(
                0,
                "message".into(),
                json!({"content": "persisted"}),
                json!({}),
                "2024-01-01T00:00:00Z".into(),
            );
            store.append("tape", &entry).unwrap();
        }

        // Read with a new instance
        {
            let store = FileTapeStore::new(dir);
            let query = TapeQuery::new("tape");
            let entries = store.fetch_all(&query).unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].payload["content"], "persisted");
        }
    }

    #[test]
    fn test_file_tape_store_append_propagates_io_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let blocked_root = tmp.path().join("blocked-root");
        std::fs::write(&blocked_root, "not a directory").unwrap();
        let store = FileTapeStore::new(blocked_root);

        let entry = TapeEntry::new(
            0,
            "message".into(),
            json!({"content": "will fail"}),
            json!({}),
            "2024-01-01T00:00:00Z".into(),
        );

        let err = store.append("bad__tape", &entry).unwrap_err();
        assert!(err.message.contains("failed to create tape directory"));
    }

    #[test]
    fn test_file_tape_store_reset_propagates_remove_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileTapeStore::new(tmp.path().to_path_buf());
        let bad_path = store.tape_file_path("bad__tape");
        std::fs::create_dir_all(&bad_path).unwrap();

        let err = store.reset("bad__tape").unwrap_err();
        assert!(err.message.contains("failed to remove tape file"));
    }

    // -- ForkTapeStore tests --------------------------------------------------

    #[tokio::test]
    async fn test_fork_merge_back_true_merges_entries() {
        let parent = InMemoryTapeStore::new();
        let store = ForkTapeStore::from_sync(parent.clone());

        store
            .fork("test-tape", true, async {
                store
                    .append(
                        "test-tape",
                        &TapeEntry::new(
                            0,
                            "event".into(),
                            json!({"name": "step"}),
                            json!({}),
                            "2024-01-01T00:00:00Z".into(),
                        ),
                    )
                    .await
                    .unwrap();
                store
                    .append(
                        "test-tape",
                        &TapeEntry::new(
                            0,
                            "event".into(),
                            json!({"name": "step2"}),
                            json!({}),
                            "2024-01-01T00:00:00Z".into(),
                        ),
                    )
                    .await
                    .unwrap();
            })
            .await;

        let entries = parent.read("test-tape");
        assert!(entries.is_some());
        assert_eq!(entries.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_fork_merge_back_false_discards_entries() {
        let parent = InMemoryTapeStore::new();
        let store = ForkTapeStore::from_sync(parent.clone());

        store
            .fork("test-tape", false, async {
                store
                    .append(
                        "test-tape",
                        &TapeEntry::new(
                            0,
                            "event".into(),
                            json!({"name": "step"}),
                            json!({}),
                            "2024-01-01T00:00:00Z".into(),
                        ),
                    )
                    .await
                    .unwrap();
            })
            .await;

        let entries = parent.read("test-tape");
        assert!(entries.is_none());
    }

    #[tokio::test]
    async fn test_fork_reset_with_merge_back_false_preserves_parent() {
        let parent = InMemoryTapeStore::new();
        parent
            .append(
                "test-tape",
                &TapeEntry::new(
                    0,
                    "event".into(),
                    json!({"name": "before"}),
                    json!({}),
                    "2024-01-01T00:00:00Z".into(),
                ),
            )
            .unwrap();

        let store = ForkTapeStore::from_sync(parent.clone());

        store
            .fork("test-tape", false, async {
                store.reset("test-tape").await.unwrap();
                store
                    .append(
                        "test-tape",
                        &TapeEntry::new(
                            0,
                            "event".into(),
                            json!({"name": "inside"}),
                            json!({}),
                            "2024-01-01T00:00:00Z".into(),
                        ),
                    )
                    .await
                    .unwrap();
            })
            .await;

        let entries = parent.read("test-tape").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].payload["name"], "before");
    }

    #[tokio::test]
    async fn test_fork_reset_hides_parent_entries_during_fetch() {
        let parent = InMemoryTapeStore::new();
        parent
            .append(
                "test-tape",
                &TapeEntry::new(
                    0,
                    "event".into(),
                    json!({"name": "before"}),
                    json!({}),
                    "2024-01-01T00:00:00Z".into(),
                ),
            )
            .unwrap();

        let store = ForkTapeStore::from_sync(parent.clone());

        store
            .fork("test-tape", false, async {
                store.reset("test-tape").await.unwrap();
                store
                    .append(
                        "test-tape",
                        &TapeEntry::new(
                            0,
                            "event".into(),
                            json!({"name": "inside"}),
                            json!({}),
                            "2024-01-01T00:00:00Z".into(),
                        ),
                    )
                    .await
                    .unwrap();

                let query = TapeQuery::new("test-tape");
                let entries = store.fetch_all(&query).await.unwrap();
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].payload["name"], "inside");
            })
            .await;
    }

    #[tokio::test]
    async fn test_reset_outside_fork_resets_parent() {
        let parent = InMemoryTapeStore::new();
        parent
            .append(
                "test-tape",
                &TapeEntry::new(
                    0,
                    "event".into(),
                    json!({"name": "before"}),
                    json!({}),
                    "2024-01-01T00:00:00Z".into(),
                ),
            )
            .unwrap();

        let store = ForkTapeStore::from_sync(parent.clone());
        store.reset("test-tape").await.unwrap();

        let entries = parent.read("test-tape");
        assert!(entries.is_none());
    }

    #[tokio::test]
    async fn test_file_tape_store_monotonic_ids_across_forks() {
        let tmp = tempfile::tempdir().unwrap();
        let parent = FileTapeStore::new(tmp.path().to_path_buf());
        let store = ForkTapeStore::from_sync(parent);

        // Wrap parent reference for reading
        let parent_read = FileTapeStore::new(tmp.path().to_path_buf());

        store
            .fork("tape", true, async {
                store
                    .append(
                        "tape",
                        &TapeEntry::new(
                            0,
                            "event".into(),
                            json!({"name": "first"}),
                            json!({}),
                            "2024-01-01T00:00:00Z".into(),
                        ),
                    )
                    .await
                    .unwrap();
            })
            .await;

        store
            .fork("tape", true, async {
                store
                    .append(
                        "tape",
                        &TapeEntry::new(
                            0,
                            "event".into(),
                            json!({"name": "second"}),
                            json!({}),
                            "2024-01-01T00:00:00Z".into(),
                        ),
                    )
                    .await
                    .unwrap();
            })
            .await;

        let query = TapeQuery::new("tape");
        let entries = parent_read.fetch_all(&query).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, 1);
        assert_eq!(entries[1].id, 2);
        assert_eq!(entries[0].payload["name"], "first");
        assert_eq!(entries[1].payload["name"], "second");
    }

    // -- entry_from_payload tests ---------------------------------------------

    #[test]
    fn test_entry_from_payload_valid() {
        let payload = json!({
            "id": 1,
            "kind": "message",
            "payload": {"content": "hello"},
            "meta": {},
            "date": "2024-01-01T00:00:00Z"
        });
        let entry = entry_from_payload(&payload).unwrap();
        assert_eq!(entry.id, 1);
        assert_eq!(entry.kind, "message");
    }

    #[test]
    fn test_entry_from_payload_missing_id_returns_none() {
        let payload = json!({"kind": "message", "payload": {}});
        assert!(entry_from_payload(&payload).is_none());
    }

    #[test]
    fn test_entry_from_payload_not_object_returns_none() {
        assert!(entry_from_payload(&json!("string")).is_none());
        assert!(entry_from_payload(&json!(42)).is_none());
    }

    // -- TapeFile tests -------------------------------------------------------

    #[test]
    fn test_tape_file_append_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.jsonl");
        let mut tf = TapeFile::new(path);

        let entry = TapeEntry::new(
            0,
            "message".into(),
            json!({"content": "hi"}),
            json!({}),
            "2024-01-01T00:00:00Z".into(),
        );
        tf.append(&entry).unwrap();

        let entries = tf.read();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, 1);
    }

    #[test]
    fn test_tape_file_reset_clears_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.jsonl");
        let mut tf = TapeFile::new(path);

        let entry = TapeEntry::new(
            0,
            "message".into(),
            json!({}),
            json!({}),
            "2024-01-01T00:00:00Z".into(),
        );
        tf.append(&entry).unwrap();
        tf.reset().unwrap();

        let entries = tf.read();
        assert!(entries.is_empty());
    }

    // -- filter_entries tests -------------------------------------------------

    #[test]
    fn test_filter_entries_matches_substring() {
        let entries = vec![
            TapeEntry::new(
                1,
                "message".into(),
                json!({"content": "hello world"}),
                json!({}),
                "2024-01-01T00:00:00Z".into(),
            ),
            TapeEntry::new(
                2,
                "message".into(),
                json!({"content": "goodbye world"}),
                json!({}),
                "2024-01-01T00:00:00Z".into(),
            ),
            TapeEntry::new(
                3,
                "message".into(),
                json!({"content": "unrelated"}),
                json!({}),
                "2024-01-01T00:00:00Z".into(),
            ),
        ];
        let results = filter_entries(&entries, "world", 10);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_filter_entries_empty_query_returns_empty() {
        let entries = vec![TapeEntry::new(
            1,
            "message".into(),
            json!({"content": "hello"}),
            json!({}),
            "2024-01-01T00:00:00Z".into(),
        )];
        let results = filter_entries(&entries, "  ", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_filter_entries_respects_limit() {
        let entries = vec![
            TapeEntry::new(
                1,
                "message".into(),
                json!({"content": "match"}),
                json!({}),
                "2024-01-01T00:00:00Z".into(),
            ),
            TapeEntry::new(
                2,
                "message".into(),
                json!({"content": "match too"}),
                json!({}),
                "2024-01-01T00:00:00Z".into(),
            ),
            TapeEntry::new(
                3,
                "message".into(),
                json!({"content": "match three"}),
                json!({}),
                "2024-01-01T00:00:00Z".into(),
            ),
        ];
        let results = filter_entries(&entries, "match", 2);
        assert_eq!(results.len(), 2);
    }
}
