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

const INDEX_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Eli Tape Viewer</title>
<style>
:root {
  --bg: #0d1117;
  --bg2: #161b22;
  --bg3: #21262d;
  --border: #30363d;
  --text: #e6edf3;
  --text2: #8b949e;
  --accent: #58a6ff;
  --green: #3fb950;
  --red: #f85149;
  --orange: #d29922;
  --purple: #bc8cff;
  --cyan: #39d2c0;
  --pink: #f778ba;
}
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
  font-family: 'SF Mono', 'Cascadia Code', 'Fira Code', monospace;
  background: var(--bg);
  color: var(--text);
  font-size: 13px;
  line-height: 1.5;
}
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }

/* Layout */
.app { display: flex; height: 100vh; }
.sidebar {
  width: 280px;
  min-width: 280px;
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  background: var(--bg2);
}
.main { flex: 1; display: flex; flex-direction: column; overflow: hidden; }

/* Sidebar */
.sidebar-header {
  padding: 16px;
  border-bottom: 1px solid var(--border);
  font-size: 15px;
  font-weight: 600;
  display: flex;
  align-items: center;
  gap: 8px;
}
.sidebar-header .logo { color: var(--accent); }
.tape-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px;
}
.tape-item {
  padding: 8px 12px;
  border-radius: 6px;
  cursor: pointer;
  margin-bottom: 2px;
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.tape-item:hover { background: var(--bg3); }
.tape-item.active { background: var(--accent); color: #000; }
.tape-item .name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
}
.tape-item .size { color: var(--text2); font-size: 11px; flex-shrink: 0; margin-left: 8px; }
.tape-item.active .size { color: #000a; }

/* Header bar */
.header-bar {
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  display: flex;
  gap: 12px;
  align-items: center;
  flex-wrap: wrap;
  background: var(--bg2);
}
.header-bar .info-chips {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}
.chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: 12px;
  font-size: 11px;
  background: var(--bg3);
  border: 1px solid var(--border);
}
.chip .label { color: var(--text2); }
.chip .value { font-weight: 600; }

/* Toolbar */
.toolbar {
  padding: 8px 16px;
  border-bottom: 1px solid var(--border);
  display: flex;
  gap: 8px;
  align-items: center;
  background: var(--bg);
}
.toolbar input, .toolbar select {
  background: var(--bg2);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 4px 8px;
  border-radius: 4px;
  font-family: inherit;
  font-size: 12px;
}
.toolbar input { flex: 1; max-width: 300px; }
.toolbar select { min-width: 100px; }
.toolbar .btn {
  background: var(--bg3);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 4px 12px;
  border-radius: 4px;
  cursor: pointer;
  font-family: inherit;
  font-size: 12px;
  white-space: nowrap;
}
.toolbar .btn:hover { border-color: var(--accent); }
.toolbar .btn.active { background: var(--accent); color: #000; border-color: var(--accent); }

/* Entries */
.entries-container {
  flex: 1;
  overflow-y: auto;
  padding: 0;
}
.entry {
  display: flex;
  border-bottom: 1px solid var(--border);
  min-height: 36px;
  transition: background 0.1s;
}
.entry:hover { background: var(--bg2); }
.entry.in-context { background: #58a6ff08; }
.entry.in-context:hover { background: #58a6ff14; }

.entry-gutter {
  width: 60px;
  min-width: 60px;
  padding: 6px 8px;
  text-align: right;
  color: var(--text2);
  font-size: 11px;
  border-right: 1px solid var(--border);
  user-select: none;
}
.entry-kind {
  width: 90px;
  min-width: 90px;
  padding: 6px 8px;
  font-size: 11px;
  font-weight: 600;
  border-right: 1px solid var(--border);
}
.entry-time {
  width: 80px;
  min-width: 80px;
  padding: 6px 8px;
  color: var(--text2);
  font-size: 11px;
  border-right: 1px solid var(--border);
}
.entry-run {
  width: 60px;
  min-width: 60px;
  padding: 6px 8px;
  font-size: 10px;
  border-right: 1px solid var(--border);
  color: var(--text2);
  overflow: hidden;
  text-overflow: ellipsis;
}
.entry-payload {
  flex: 1;
  padding: 6px 12px;
  overflow: hidden;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 400px;
  overflow-y: auto;
}
.entry-payload.collapsed {
  max-height: 60px;
  overflow: hidden;
  cursor: pointer;
}

/* Kind colors */
.kind-message { color: var(--accent); }
.kind-system { color: var(--purple); }
.kind-anchor { color: var(--orange); }
.kind-tool_call { color: var(--cyan); }
.kind-tool_result { color: var(--green); }
.kind-error { color: var(--red); }
.kind-event { color: var(--pink); }

/* Anchor separator */
.anchor-separator {
  display: flex;
  align-items: center;
  padding: 4px 16px;
  background: #d2992210;
  border-bottom: 2px solid var(--orange);
  border-top: 2px solid var(--orange);
  gap: 8px;
  font-size: 12px;
  font-weight: 600;
  color: var(--orange);
}
.anchor-separator .anchor-line {
  flex: 1;
  height: 1px;
  background: var(--orange);
  opacity: 0.3;
}

/* Context window marker */
.context-marker {
  padding: 4px 16px;
  background: #58a6ff15;
  border: 1px dashed var(--accent);
  color: var(--accent);
  font-size: 12px;
  font-weight: 600;
  display: flex;
  align-items: center;
  gap: 8px;
}
.context-marker .marker-line {
  flex: 1;
  height: 1px;
  background: var(--accent);
  opacity: 0.3;
}

/* Role badges */
.role-user { color: var(--green); }
.role-assistant { color: var(--accent); }
.role-system { color: var(--purple); }

/* Empty state */
.empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--text2);
  font-size: 15px;
}

/* Stats panel */
.stats-panel {
  padding: 12px 16px;
  border-top: 1px solid var(--border);
  background: var(--bg2);
  font-size: 11px;
  color: var(--text2);
  display: flex;
  gap: 16px;
  flex-wrap: wrap;
}

/* Scrollbar */
::-webkit-scrollbar { width: 8px; height: 8px; }
::-webkit-scrollbar-track { background: var(--bg); }
::-webkit-scrollbar-thumb { background: var(--bg3); border-radius: 4px; }
::-webkit-scrollbar-thumb:hover { background: #484f58; }

/* JSON pretty print */
.json-key { color: var(--accent); }
.json-string { color: var(--green); }
.json-number { color: var(--orange); }
.json-boolean { color: var(--purple); }
.json-null { color: var(--text2); }

/* Loading */
.loading {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 40px;
  color: var(--text2);
}
@keyframes spin { to { transform: rotate(360deg); } }
.spinner {
  width: 20px; height: 20px;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
  margin-right: 8px;
}
</style>
</head>
<body>
<div class="app">
  <div class="sidebar">
    <div class="sidebar-header">
      <span class="logo">&#9654;</span> Eli Tape Viewer
    </div>
    <div class="tape-list" id="tape-list">
      <div class="loading"><div class="spinner"></div> Loading tapes...</div>
    </div>
  </div>
  <div class="main">
    <div class="header-bar" id="header-bar">
      <div class="info-chips" id="info-chips"></div>
    </div>
    <div class="toolbar" id="toolbar">
      <input type="text" id="search" placeholder="Search entries..." />
      <select id="kind-filter">
        <option value="">All kinds</option>
        <option value="message">message</option>
        <option value="system">system</option>
        <option value="anchor">anchor</option>
        <option value="tool_call">tool_call</option>
        <option value="tool_result">tool_result</option>
        <option value="error">error</option>
        <option value="event">event</option>
      </select>
      <button class="btn" id="btn-context" title="Highlight entries in current context window">Context</button>
      <button class="btn" id="btn-collapse" title="Toggle payload collapse">Collapse</button>
      <button class="btn" id="btn-refresh" title="Reload">Refresh</button>
    </div>
    <div class="entries-container" id="entries-container">
      <div class="empty">Select a tape from the sidebar</div>
    </div>
    <div class="stats-panel" id="stats-panel"></div>
  </div>
</div>

<script>
// ── State ──────────────────────────────────────────────────
let currentTape = null;
let tapeInfo = null;
let allEntries = [];
let contextAnchorIdx = null;
let showContext = false;
let collapsed = true;
let loading = false;

// ── DOM refs ───────────────────────────────────────────────
const $tapeList = document.getElementById('tape-list');
const $infoChips = document.getElementById('info-chips');
const $search = document.getElementById('search');
const $kindFilter = document.getElementById('kind-filter');
const $entries = document.getElementById('entries-container');
const $stats = document.getElementById('stats-panel');
const $btnContext = document.getElementById('btn-context');
const $btnCollapse = document.getElementById('btn-collapse');
const $btnRefresh = document.getElementById('btn-refresh');

// ── API helpers ────────────────────────────────────────────
async function api(path) {
  const res = await fetch(path);
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.json();
}

// ── Boot ───────────────────────────────────────────────────
async function loadTapes() {
  const tapes = await api('/api/tapes');
  $tapeList.innerHTML = '';
  if (tapes.length === 0) {
    $tapeList.innerHTML = '<div class="empty" style="padding:20px;font-size:12px">No tapes found</div>';
    return;
  }
  for (const t of tapes) {
    const el = document.createElement('div');
    el.className = 'tape-item';
    el.innerHTML = `<span class="name" title="${esc(t.name)}">${esc(t.name)}</span><span class="size">${fmtSize(t.size_bytes)}</span>`;
    el.onclick = () => selectTape(t.name);
    $tapeList.appendChild(el);
  }
}

async function selectTape(name) {
  if (loading) return;
  currentTape = name;
  loading = true;

  // Highlight sidebar
  document.querySelectorAll('.tape-item').forEach(el => {
    el.classList.toggle('active', el.querySelector('.name').textContent === name);
  });

  $entries.innerHTML = '<div class="loading"><div class="spinner"></div> Loading entries...</div>';

  try {
    // Load all entries (paginate in chunks of 2000)
    allEntries = [];
    let offset = 0;
    const pageSize = 2000;
    while (true) {
      const data = await api(`/api/tapes/${encodeURIComponent(name)}?offset=${offset}&limit=${pageSize}`);
      allEntries.push(...data.entries);
      if (allEntries.length >= data.total || data.entries.length < pageSize) break;
      offset += pageSize;
    }

    tapeInfo = await api(`/api/tapes/${encodeURIComponent(name)}/info`);

    // Find context anchor index
    contextAnchorIdx = null;
    if (tapeInfo.anchors && tapeInfo.anchors.length > 0) {
      contextAnchorIdx = tapeInfo.anchors[tapeInfo.anchors.length - 1].index;
    }

    renderInfo();
    renderEntries();
  } catch (e) {
    $entries.innerHTML = `<div class="empty" style="color:var(--red)">Error: ${esc(e.message)}</div>`;
  }
  loading = false;
}

// ── Render info chips ──────────────────────────────────────
function renderInfo() {
  if (!tapeInfo) { $infoChips.innerHTML = ''; return; }
  const i = tapeInfo;
  let html = '';
  html += chip('entries', i.total_entries);
  html += chip('anchors', i.anchors?.length || 0);
  html += chip('runs', i.runs);
  html += chip('context window', i.entries_since_last_anchor);
  if (i.last_model) html += chip('model', i.last_model);
  if (i.last_token_usage?.total_tokens) html += chip('tokens', fmtNum(i.last_token_usage.total_tokens));
  html += chip('size', fmtSize(i.size_bytes));
  if (i.kinds) {
    for (const [k, v] of Object.entries(i.kinds)) {
      html += `<span class="chip"><span class="label kind-${k}">${k}</span> <span class="value">${v}</span></span>`;
    }
  }
  $infoChips.innerHTML = html;
}

function chip(label, value) {
  return `<span class="chip"><span class="label">${esc(label)}</span> <span class="value">${esc(String(value))}</span></span>`;
}

// ── Render entries ─────────────────────────────────────────
function renderEntries() {
  const searchQ = $search.value.toLowerCase();
  const kindQ = $kindFilter.value;

  // Filter
  let filtered = allEntries;
  if (searchQ) {
    filtered = filtered.filter(e => JSON.stringify(e).toLowerCase().includes(searchQ));
  }
  if (kindQ) {
    filtered = filtered.filter(e => e.kind === kindQ);
  }

  if (filtered.length === 0) {
    $entries.innerHTML = '<div class="empty">No entries match filters</div>';
    $stats.textContent = '';
    return;
  }

  // Build HTML via document fragment for performance
  const frag = document.createDocumentFragment();

  // Group run_ids for coloring
  const runColors = {};
  let colorIdx = 0;
  const palette = ['#58a6ff','#3fb950','#d29922','#bc8cff','#39d2c0','#f778ba','#f85149','#79c0ff','#56d364','#e3b341'];

  for (let i = 0; i < filtered.length; i++) {
    const e = filtered[i];
    const entryIdx = e.id != null ? e.id : i;
    const isAnchor = e.kind === 'anchor';
    const isInContext = showContext && contextAnchorIdx != null && allEntries.indexOf(e) > contextAnchorIdx;

    // Insert anchor separator
    if (isAnchor) {
      const anchorName = e.payload?.name || '-';
      const sep = document.createElement('div');
      sep.className = 'anchor-separator';
      sep.innerHTML = `<span class="anchor-line"></span> &#9875; ${esc(anchorName)} <span class="anchor-line"></span>`;
      frag.appendChild(sep);

      // Check if this is the context window boundary
      if (showContext && contextAnchorIdx != null && allEntries.indexOf(e) === contextAnchorIdx) {
        const marker = document.createElement('div');
        marker.className = 'context-marker';
        marker.innerHTML = '<span class="marker-line"></span> &#9654; CONTEXT WINDOW START — entries below are what the agent sees <span class="marker-line"></span>';
        frag.appendChild(marker);
      }
    }

    const row = document.createElement('div');
    row.className = 'entry' + (isInContext ? ' in-context' : '');

    // Gutter (id)
    const gutter = document.createElement('div');
    gutter.className = 'entry-gutter';
    gutter.textContent = entryIdx;

    // Kind
    const kindEl = document.createElement('div');
    kindEl.className = `entry-kind kind-${e.kind}`;
    kindEl.textContent = e.kind;

    // Time
    const timeEl = document.createElement('div');
    timeEl.className = 'entry-time';
    timeEl.textContent = fmtTime(e.date);

    // Run ID
    const runEl = document.createElement('div');
    runEl.className = 'entry-run';
    const runId = e.meta?.run_id;
    if (runId) {
      if (!runColors[runId]) {
        runColors[runId] = palette[colorIdx % palette.length];
        colorIdx++;
      }
      runEl.style.color = runColors[runId];
      runEl.textContent = runId.slice(0, 6);
      runEl.title = runId;
    }

    // Payload
    const payloadEl = document.createElement('div');
    payloadEl.className = 'entry-payload' + (collapsed ? ' collapsed' : '');
    payloadEl.innerHTML = renderPayload(e);
    if (collapsed) {
      payloadEl.onclick = function() { this.classList.toggle('collapsed'); };
    }

    row.appendChild(gutter);
    row.appendChild(kindEl);
    row.appendChild(timeEl);
    row.appendChild(runEl);
    row.appendChild(payloadEl);
    frag.appendChild(row);
  }

  $entries.innerHTML = '';
  $entries.appendChild(frag);

  // Stats
  const runCount = Object.keys(runColors).length;
  $stats.textContent = `Showing ${filtered.length} / ${allEntries.length} entries · ${runCount} runs`;
}

function renderPayload(entry) {
  const e = entry;
  switch (e.kind) {
    case 'message': {
      const role = e.payload?.role || '?';
      const content = e.payload?.content || '';
      const text = typeof content === 'string' ? content : JSON.stringify(content, null, 2);
      return `<span class="role-${role}">[${esc(role)}]</span> ${esc(text)}`;
    }
    case 'system': {
      const c = e.payload?.content || '';
      return `<span class="role-system">[system]</span> ${esc(c)}`;
    }
    case 'anchor': {
      const name = e.payload?.name || '-';
      const state = e.payload?.state;
      let s = `<span style="color:var(--orange);font-weight:600">${esc(name)}</span>`;
      if (state && Object.keys(state).length > 0) {
        s += ` <span style="color:var(--text2)">${esc(JSON.stringify(state))}</span>`;
      }
      return s;
    }
    case 'tool_call': {
      const calls = e.payload?.calls || [];
      if (calls.length === 0) return '<span style="color:var(--text2)">(no calls)</span>';
      return calls.map(c => {
        const name = c.function?.name || c.name || '?';
        const args = c.function?.arguments || c.input || c.arguments || '';
        const argsStr = typeof args === 'string' ? args : JSON.stringify(args, null, 2);
        return `<span style="color:var(--cyan);font-weight:600">${esc(name)}</span>(${esc(argsStr)})`;
      }).join('<br>');
    }
    case 'tool_result': {
      const results = e.payload?.results || [];
      return results.map(r => {
        const content = r.content || r.output || r.result || JSON.stringify(r);
        const text = typeof content === 'string' ? content : JSON.stringify(content, null, 2);
        return esc(text);
      }).join('<br>');
    }
    case 'error': {
      const code = e.payload?.code || '';
      const msg = e.payload?.message || JSON.stringify(e.payload);
      return `<span style="color:var(--red);font-weight:600">${esc(code)}</span> ${esc(msg)}`;
    }
    case 'event': {
      const name = e.payload?.name || '?';
      const data = e.payload?.data;
      let s = `<span style="color:var(--pink);font-weight:600">${esc(name)}</span>`;
      if (data) {
        s += ` ${esc(JSON.stringify(data, null, 2))}`;
      }
      return s;
    }
    default:
      return esc(JSON.stringify(e.payload, null, 2));
  }
}

// ── Event handlers ─────────────────────────────────────────
let searchTimeout;
$search.addEventListener('input', () => {
  clearTimeout(searchTimeout);
  searchTimeout = setTimeout(renderEntries, 200);
});
$kindFilter.addEventListener('change', renderEntries);

$btnContext.addEventListener('click', () => {
  showContext = !showContext;
  $btnContext.classList.toggle('active', showContext);
  renderEntries();
  // Scroll to context start
  if (showContext) {
    setTimeout(() => {
      const marker = $entries.querySelector('.context-marker');
      if (marker) marker.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }, 50);
  }
});

$btnCollapse.addEventListener('click', () => {
  collapsed = !collapsed;
  $btnCollapse.classList.toggle('active', collapsed);
  renderEntries();
});
$btnCollapse.classList.add('active'); // starts collapsed

$btnRefresh.addEventListener('click', () => {
  if (currentTape) selectTape(currentTape);
});

// ── Helpers ────────────────────────────────────────────────
function esc(s) {
  if (!s) return '';
  return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

function fmtSize(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024*1024) return (bytes/1024).toFixed(1) + ' KB';
  return (bytes/1024/1024).toFixed(1) + ' MB';
}

function fmtNum(n) {
  return Number(n).toLocaleString();
}

function fmtTime(isoStr) {
  if (!isoStr) return '';
  try {
    const d = new Date(isoStr);
    return d.toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
  } catch { return ''; }
}

// ── Init ───────────────────────────────────────────────────
loadTapes();
</script>
</body>
</html>
"##;
