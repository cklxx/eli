//! Tape service — high-level operations on tapes (info, anchors, search, etc.).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use nexil::{ConduitError, ErrorKind, TapeEntry, TapeQuery};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::builtin::store::ForkTapeStore;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Runtime tape info summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapeInfo {
    pub name: String,
    pub entries: usize,
    pub anchors: usize,
    pub last_anchor: Option<String>,
    pub entries_since_last_anchor: usize,
    pub last_token_usage: Option<i64>,
}

/// Rendered anchor summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnchorSummary {
    pub name: String,
    pub state: HashMap<String, Value>,
}

// ---------------------------------------------------------------------------
// TapeService
// ---------------------------------------------------------------------------

/// High-level operations over tapes, backed by a `ForkTapeStore`.
#[derive(Clone)]
pub struct TapeService {
    archive_path: PathBuf,
    store: ForkTapeStore,
}

impl TapeService {
    pub fn new(archive_path: PathBuf, store: ForkTapeStore) -> Self {
        Self {
            archive_path,
            store,
        }
    }

    /// Return information about a tape.
    pub async fn info(&self, tape_name: &str) -> Result<TapeInfo, ConduitError> {
        let query = TapeQuery::new(tape_name);
        let entries = self.store.fetch_all(&query).await?;

        let anchor_positions: Vec<(usize, String)> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == "anchor")
            .map(|(i, e)| {
                let name = e
                    .payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_owned();
                (i, name)
            })
            .collect();

        let (last_anchor, entries_since_last_anchor) =
            if let Some((idx, name)) = anchor_positions.last() {
                (Some(name.clone()), entries.len() - idx - 1)
            } else {
                (None, entries.len())
            };

        let last_token_usage = find_last_token_usage(&entries);

        Ok(TapeInfo {
            name: tape_name.to_owned(),
            entries: entries.len(),
            anchors: anchor_positions.len(),
            last_anchor,
            entries_since_last_anchor,
            last_token_usage,
        })
    }

    /// Ensure the tape has a bootstrap anchor. Creates one if none exist.
    pub async fn ensure_bootstrap_anchor(&self, tape_name: &str) -> Result<(), ConduitError> {
        let query = TapeQuery::new(tape_name).kinds(vec!["anchor".to_owned()]);
        let anchors = self.store.fetch_all(&query).await?;
        if anchors.is_empty() {
            let anchor = TapeEntry::anchor(
                "session/start",
                Some(serde_json::json!({"owner": "human"})),
                Value::Object(Default::default()),
            );
            self.store.append(tape_name, &anchor).await?;
        }
        Ok(())
    }

    /// List anchors in a tape.
    pub async fn anchors(
        &self,
        tape_name: &str,
        limit: usize,
    ) -> Result<Vec<AnchorSummary>, ConduitError> {
        let query = TapeQuery::new(tape_name).kinds(vec!["anchor".to_owned()]);
        let entries = self.store.fetch_all(&query).await?;

        let start = if entries.len() > limit {
            entries.len() - limit
        } else {
            0
        };

        let results: Vec<AnchorSummary> = entries[start..]
            .iter()
            .map(|entry| {
                let name = entry
                    .payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_owned();
                let state = entry
                    .payload
                    .get("state")
                    .and_then(|v| v.as_object())
                    .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                AnchorSummary { name, state }
            })
            .collect();

        Ok(results)
    }

    /// Archive a tape to a timestamped backup file.
    async fn archive(&self, tape_name: &str) -> Result<PathBuf, ConduitError> {
        let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        fs::create_dir_all(&self.archive_path).ok();
        let archive_path = self
            .archive_path
            .join(format!("{tape_name}.jsonl.{stamp}.bak"));

        let query = TapeQuery::new(tape_name);
        let entries = self.store.fetch_all(&query).await?;

        let mut file = fs::File::create(&archive_path).map_err(|e| {
            ConduitError::new(
                ErrorKind::Unknown,
                format!("Failed to create archive file: {e}"),
            )
        })?;

        use std::io::Write;
        for entry in &entries {
            if let Ok(json) = serde_json::to_string(entry) {
                let _ = writeln!(file, "{json}");
            }
        }

        Ok(archive_path)
    }

    /// Reset a tape, optionally archiving it first.
    pub async fn reset(&self, tape_name: &str, do_archive: bool) -> Result<String, ConduitError> {
        let archive_path = if do_archive {
            Some(self.archive(tape_name).await?)
        } else {
            None
        };

        self.store.reset(tape_name).await?;

        let mut state = serde_json::json!({"owner": "human"});
        if let Some(ref path) = archive_path {
            state["archived"] = Value::String(path.display().to_string());
        }

        let anchor = TapeEntry::anchor(
            "session/start",
            Some(state),
            Value::Object(Default::default()),
        );
        self.store.append(tape_name, &anchor).await?;

        match archive_path {
            Some(p) => Ok(format!("Archived: {}", p.display())),
            None => Ok("ok".to_owned()),
        }
    }

    /// Get the name of the last anchor in the tape (if any).
    pub async fn last_anchor_name(&self, tape_name: &str) -> Result<Option<String>, ConduitError> {
        let query = TapeQuery::new(tape_name).kinds(vec!["anchor".to_owned()]);
        let entries = self.store.fetch_all(&query).await?;
        Ok(entries.last().and_then(|e| {
            e.payload
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned())
        }))
    }

    /// Find the most recent `auto-handoff.grace` event and return
    /// `(remaining_rounds, prev_anchor_name)`.
    pub async fn auto_handoff_grace(
        &self,
        tape_name: &str,
    ) -> Result<Option<(u32, String)>, ConduitError> {
        let query = TapeQuery::new(tape_name).kinds(vec!["event".to_owned()]);
        let entries = self.store.fetch_all(&query).await?;
        // Walk backwards to find the latest grace event.
        for entry in entries.iter().rev() {
            if entry.payload.get("name").and_then(|v| v.as_str()) == Some("auto-handoff.grace") {
                let data = entry.payload.get("data");
                let remaining = data
                    .and_then(|d| d.get("remaining"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                // remaining == 0 means grace has expired; treat as no active grace
                // so the caller falls through to the new-handoff trigger branch.
                if remaining == 0 {
                    return Ok(None);
                }
                let prev_anchor = data
                    .and_then(|d| d.get("prev_anchor"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                return Ok(Some((remaining, prev_anchor)));
            }
        }
        Ok(None)
    }

    /// Add a handoff anchor to the tape.
    pub async fn handoff(
        &self,
        tape_name: &str,
        name: &str,
        state: Option<Value>,
    ) -> Result<(), ConduitError> {
        let anchor = TapeEntry::anchor(name, state, Value::Object(Default::default()));
        self.store.append(tape_name, &anchor).await
    }

    /// Search for entries matching a query.
    pub async fn search(&self, query: &TapeQuery) -> Result<Vec<TapeEntry>, ConduitError> {
        self.store.fetch_all(query).await
    }

    /// Append an event entry to the tape.
    pub async fn append_event(
        &self,
        tape_name: &str,
        name: &str,
        data: Value,
    ) -> Result<(), ConduitError> {
        let entry = TapeEntry::event(name, Some(data), Value::Object(Default::default()));
        self.store.append(tape_name, &entry).await
    }

    /// Derive a tape name from a session ID and workspace path.
    pub fn session_tape_name(session_id: &str, workspace: &Path) -> String {
        let workspace_str = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf())
            .display()
            .to_string();
        let workspace_hash = format!("{:x}", md5::compute(workspace_str.as_bytes()));
        let session_hash = format!("{:x}", md5::compute(session_id.as_bytes()));
        format!(
            "{}__{}",
            &workspace_hash[..16.min(workspace_hash.len())],
            &session_hash[..16.min(session_hash.len())]
        )
    }

    /// Fork writes for a tape into a temporary in-memory store.
    pub async fn fork_tape<F, T>(&self, tape_name: &str, merge_back: bool, f: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        self.store.fork(tape_name, merge_back, f).await
    }

    /// Get a reference to the underlying store for advanced queries.
    pub fn store(&self) -> &ForkTapeStore {
        &self.store
    }
}

/// Find the most recent agent run event's `total_tokens` usage.
fn find_last_token_usage(entries: &[TapeEntry]) -> Option<i64> {
    entries.iter().rev().find_map(|entry| {
        let name = entry
            .payload
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        (entry.kind == "event" && (name == "run" || name == "agent.run"))
            .then(|| {
                entry
                    .payload
                    .get("data")
                    .and_then(|d| d.get("usage"))
                    .and_then(|u| u.get("total_tokens"))
                    .and_then(|t| t.as_i64())
            })
            .flatten()
    })
}
