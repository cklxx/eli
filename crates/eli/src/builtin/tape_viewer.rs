//! Tape viewer — lightweight web UI for inspecting tape JSONL files.
//!
//! Starts an axum HTTP server that serves:
//! - `GET /`                         → embedded single-page app
//! - `GET /api/tapes`                → list available tapes
//! - `GET /api/tapes/:name`          → entries (paginated: ?offset=0&limit=200&kind=&q=)
//! - `GET /api/tapes/:name/info`     → summary (entry count, anchors, context window)
//! - `GET /api/tapes/:name/context`  → entries the agent would see (after last anchor, messages only)

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct AppState {
    tapes_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Start the tape viewer web server.
pub async fn serve(tapes_dir: PathBuf, port: u16) -> anyhow::Result<()> {
    let tapes_dir = tapes_dir.canonicalize().unwrap_or(tapes_dir);
    if !tapes_dir.exists() {
        anyhow::bail!("Tapes directory does not exist: {}", tapes_dir.display());
    }

    let state = Arc::new(AppState { tapes_dir });

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/tapes", get(list_tapes))
        .route("/api/tapes/{name}", get(get_tape_entries))
        .route("/api/tapes/{name}/info", get(get_tape_info))
        .route("/api/tapes/{name}/context", get(get_tape_context))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    eprintln!("Tape viewer → http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn index_handler() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        Html(INDEX_HTML),
    )
}

/// Validate a tape name from the URL: must be non-empty, no path separators or
/// parent-directory references.  Returns the resolved `.jsonl` path only if it
/// lives inside `tapes_dir`.
fn resolve_tape_path(tapes_dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.starts_with('.')
    {
        return None;
    }
    let path = tapes_dir.join(format!("{name}.jsonl"));
    // Reject symlink escapes: the canonical parent must still be tapes_dir.
    let canonical = path.canonicalize().ok()?;
    if !canonical.starts_with(tapes_dir) {
        return None;
    }
    Some(path)
}

async fn list_tapes(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut tapes: Vec<TapeListItem> = fs::read_dir(&state.tapes_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_jsonl = path.extension().and_then(|e| e.to_str()) == Some("jsonl");
            let stem = path.file_stem().and_then(|s| s.to_str())?;
            is_jsonl.then(|| TapeListItem {
                name: stem.to_owned(),
                size_bytes: fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            })
        })
        .collect();
    tapes.sort_by(|a, b| a.name.cmp(&b.name));
    Json(tapes)
}

#[derive(Deserialize)]
struct EntriesQuery {
    offset: Option<usize>,
    limit: Option<usize>,
    kind: Option<String>,
    q: Option<String>,
}

async fn get_tape_entries(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(params): Query<EntriesQuery>,
) -> impl IntoResponse {
    let Some(path) = resolve_tape_path(&state.tapes_dir, &name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(Value::String("tape not found".into())),
        )
            .into_response();
    };

    let all_entries = read_jsonl(&path);
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(200);

    // Apply filters
    let filtered: Vec<&RawEntry> = all_entries
        .iter()
        .filter(|e| {
            if let Some(ref kind) = params.kind
                && !kind.is_empty()
                && e.kind != *kind
            {
                return false;
            }
            if let Some(ref q) = params.q
                && !q.is_empty()
            {
                let needle = q.to_lowercase();
                let haystack = e.raw_json.to_lowercase();
                if !haystack.contains(&needle) {
                    return false;
                }
            }
            true
        })
        .collect();

    let total = filtered.len();
    let page: Vec<Value> = filtered
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|e| e.parsed.clone())
        .collect();

    let resp = serde_json::json!({
        "total": total,
        "offset": offset,
        "limit": limit,
        "entries": page,
    });
    Json(resp).into_response()
}

async fn get_tape_info(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let Some(path) = resolve_tape_path(&state.tapes_dir, &name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(Value::String("tape not found".into())),
        )
            .into_response();
    };

    let entries = read_jsonl(&path);
    let stats = compute_tape_stats(&entries);

    let info = serde_json::json!({
        "name": name,
        "total_entries": entries.len(),
        "anchors": stats.anchors,
        "entries_since_last_anchor": stats.entries_since_last_anchor,
        "runs": stats.run_count,
        "kinds": stats.kinds,
        "last_token_usage": stats.last_token_usage,
        "last_model": stats.last_model,
        "size_bytes": fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
    });
    Json(info).into_response()
}

struct TapeStats {
    anchors: Vec<AnchorInfo>,
    entries_since_last_anchor: usize,
    run_count: usize,
    kinds: HashMap<String, usize>,
    last_token_usage: Option<Value>,
    last_model: Option<String>,
}

fn compute_tape_stats(entries: &[RawEntry]) -> TapeStats {
    let mut kinds: HashMap<String, usize> = HashMap::new();
    let mut run_ids: HashMap<String, usize> = HashMap::new();

    let anchors: Vec<AnchorInfo> = entries
        .iter()
        .enumerate()
        .inspect(|(_, e)| {
            *kinds.entry(e.kind.clone()).or_default() += 1;
            if let Some(rid) = e
                .parsed
                .get("meta")
                .and_then(|m| m.get("run_id"))
                .and_then(|r| r.as_str())
            {
                *run_ids.entry(rid.to_owned()).or_default() += 1;
            }
        })
        .filter(|(_, e)| e.kind == "anchor")
        .map(|(i, e)| AnchorInfo {
            index: i,
            name: raw_entry_payload_str(e, "name").to_owned(),
            date: e.date.clone(),
        })
        .collect();

    let entries_since_last_anchor = anchors
        .last()
        .map(|a| entries.len().saturating_sub(a.index + 1))
        .unwrap_or(entries.len());

    let (last_token_usage, last_model) = find_last_run_info(entries);

    TapeStats {
        anchors,
        entries_since_last_anchor,
        run_count: run_ids.len(),
        kinds,
        last_token_usage,
        last_model,
    }
}

fn raw_entry_payload_str<'a>(entry: &'a RawEntry, field: &str) -> &'a str {
    entry
        .parsed
        .get("payload")
        .and_then(|p| p.get(field))
        .and_then(|n| n.as_str())
        .unwrap_or("-")
}

fn find_last_run_info(entries: &[RawEntry]) -> (Option<Value>, Option<String>) {
    entries
        .iter()
        .rev()
        .find(|e| e.kind == "event" && raw_entry_payload_str(e, "name") == "run")
        .map(|e| {
            let data = e.parsed.get("payload").and_then(|p| p.get("data"));
            let usage = data.and_then(|d| d.get("usage")).cloned();
            let model = data
                .and_then(|d| d.get("model"))
                .and_then(|m| m.as_str())
                .map(String::from);
            (usage, model)
        })
        .unwrap_or((None, None))
}

async fn get_tape_context(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let Some(path) = resolve_tape_path(&state.tapes_dir, &name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(Value::String("tape not found".into())),
        )
            .into_response();
    };

    let entries = read_jsonl(&path);
    let resp = build_context_response(&entries);
    Json(resp).into_response()
}

fn build_context_response(entries: &[RawEntry]) -> Value {
    let last_anchor_idx = entries.iter().rposition(|e| e.kind == "anchor");
    let start = last_anchor_idx.map(|i| i + 1).unwrap_or(0);
    let after_anchor = &entries[start..];

    let context_entries: Vec<Value> = after_anchor.iter().map(|e| e.parsed.clone()).collect();
    let messages: Vec<Value> = after_anchor
        .iter()
        .filter(|e| e.kind == "message")
        .filter_map(|e| e.parsed.get("payload").cloned())
        .collect();

    let anchor_name = last_anchor_idx.and_then(|i| {
        entries[i]
            .parsed
            .get("payload")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .map(String::from)
    });

    serde_json::json!({
        "anchor": anchor_name,
        "anchor_index": last_anchor_idx,
        "total_tape_entries": entries.len(),
        "context_entries": context_entries.len(),
        "messages_for_llm": messages.len(),
        "entries": context_entries,
        "messages": messages,
    })
}

// ---------------------------------------------------------------------------
// JSONL reader
// ---------------------------------------------------------------------------

struct RawEntry {
    kind: String,
    date: String,
    raw_json: String,
    parsed: Value,
}

fn read_jsonl(path: &std::path::Path) -> Vec<RawEntry> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|raw| !raw.trim().is_empty())
        .filter_map(|raw| {
            let parsed: Value = serde_json::from_str(raw.trim()).ok()?;
            Some(RawEntry {
                kind: parsed
                    .get("kind")
                    .and_then(|k| k.as_str())
                    .unwrap_or("unknown")
                    .to_owned(),
                date: parsed
                    .get("date")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_owned(),
                raw_json: raw,
                parsed,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct TapeListItem {
    name: String,
    size_bytes: u64,
}

#[derive(Serialize)]
struct AnchorInfo {
    index: usize,
    name: String,
    date: String,
}

// ---------------------------------------------------------------------------
// Embedded SPA
// ---------------------------------------------------------------------------

const INDEX_HTML: &str = include_str!("tape_viewer.html");
