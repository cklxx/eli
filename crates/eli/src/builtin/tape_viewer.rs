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

async fn list_tapes(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut tapes: Vec<TapeListItem> = Vec::new();
    if let Ok(entries) = fs::read_dir(&state.tapes_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            // Include all .jsonl files, not just those with __
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            tapes.push(TapeListItem {
                name: stem.to_owned(),
                size_bytes: size,
            });
        }
    }
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
    let path = state.tapes_dir.join(format!("{name}.jsonl"));
    if !path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(Value::String("tape not found".into())),
        )
            .into_response();
    }

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
    let path = state.tapes_dir.join(format!("{name}.jsonl"));
    if !path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(Value::String("tape not found".into())),
        )
            .into_response();
    }

    let entries = read_jsonl(&path);
    let total = entries.len();

    let mut anchors: Vec<AnchorInfo> = Vec::new();
    let mut run_ids: HashMap<String, usize> = HashMap::new();
    let mut kinds: HashMap<String, usize> = HashMap::new();

    for (i, e) in entries.iter().enumerate() {
        *kinds.entry(e.kind.clone()).or_default() += 1;

        if e.kind == "anchor" {
            let anchor_name = e
                .parsed
                .get("payload")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("-")
                .to_owned();
            anchors.push(AnchorInfo {
                index: i,
                name: anchor_name,
                date: e.date.clone(),
            });
        }

        if let Some(rid) = e
            .parsed
            .get("meta")
            .and_then(|m| m.get("run_id"))
            .and_then(|r| r.as_str())
        {
            *run_ids.entry(rid.to_owned()).or_default() += 1;
        }
    }

    let last_anchor_idx = anchors.last().map(|a| a.index);
    let entries_since_last_anchor = last_anchor_idx
        .map(|idx| total.saturating_sub(idx + 1))
        .unwrap_or(total);

    // Token usage from most recent run event
    let mut last_token_usage: Option<Value> = None;
    let mut last_model: Option<String> = None;
    for e in entries.iter().rev() {
        if e.kind == "event"
            && e.parsed
                .get("payload")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                == Some("run")
        {
            last_token_usage = e
                .parsed
                .get("payload")
                .and_then(|p| p.get("data"))
                .and_then(|d| d.get("usage"))
                .cloned();
            last_model = e
                .parsed
                .get("payload")
                .and_then(|p| p.get("data"))
                .and_then(|d| d.get("model"))
                .and_then(|m| m.as_str())
                .map(String::from);
            break;
        }
    }

    let info = serde_json::json!({
        "name": name,
        "total_entries": total,
        "anchors": anchors,
        "entries_since_last_anchor": entries_since_last_anchor,
        "runs": run_ids.len(),
        "kinds": kinds,
        "last_token_usage": last_token_usage,
        "last_model": last_model,
        "size_bytes": fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
    });
    Json(info).into_response()
}

async fn get_tape_context(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let path = state.tapes_dir.join(format!("{name}.jsonl"));
    if !path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(Value::String("tape not found".into())),
        )
            .into_response();
    }

    let entries = read_jsonl(&path);

    // Find last anchor
    let last_anchor_idx = entries.iter().rposition(|e| e.kind == "anchor");
    let start = last_anchor_idx.map(|i| i + 1).unwrap_or(0);

    // Context = entries after last anchor
    let context_entries: Vec<Value> = entries[start..].iter().map(|e| e.parsed.clone()).collect();

    // Also extract just the messages (what LLM actually sees)
    let messages: Vec<Value> = entries[start..]
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

    let resp = serde_json::json!({
        "anchor": anchor_name,
        "anchor_index": last_anchor_idx,
        "total_tape_entries": entries.len(),
        "context_entries": context_entries.len(),
        "messages_for_llm": messages.len(),
        "entries": context_entries,
        "messages": messages,
    });
    Json(resp).into_response()
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
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let Ok(raw) = line else { continue };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        let kind = parsed
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("unknown")
            .to_owned();
        let date = parsed
            .get("date")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_owned();

        entries.push(RawEntry {
            kind,
            date,
            raw_json: raw,
            parsed,
        });
    }
    entries
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
