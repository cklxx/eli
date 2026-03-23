//! Builtin tool implementations: bash, fs, tape, web, subagent, etc.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use conduit::tools::schema::ToolResult;
use conduit::{ConduitError, ErrorKind};
use conduit::{TapeEntry, TapeQuery, Tool, ToolContext};
use futures::future::BoxFuture;
use serde_json::Value;

use crate::builtin::shell_manager::shell_manager;
use crate::builtin::tape::TapeService;
use crate::skills::discover_skills;
use crate::tools::{REGISTRY, shorten_text};

const DEFAULT_COMMAND_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 10;

tokio::task_local! {
    static CURRENT_TAPE_SERVICE: TapeService;
}

// ---------------------------------------------------------------------------
// Tool registry (populated at first access)
// ---------------------------------------------------------------------------

/// Register all builtin tools into the global `REGISTRY`.
pub fn register_builtin_tools() {
    let tools = builtin_tools();
    let mut reg = REGISTRY.lock().unwrap();
    for tool in tools {
        reg.insert(tool.name.clone(), tool);
    }
}

/// Run a future with the current tape service bound for tool handlers.
pub async fn with_tape_runtime<F, T>(tape_service: TapeService, future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    CURRENT_TAPE_SERVICE.scope(tape_service, future).await
}

/// Build the full list of builtin tools.
fn builtin_tools() -> Vec<Tool> {
    let mut tools = vec![
        tool_bash(),
        tool_bash_output(),
        tool_bash_kill(),
        tool_fs_read(),
        tool_fs_write(),
        tool_fs_edit(),
        tool_skill(),
        tool_tape_info(),
        tool_tape_search(),
        tool_tape_reset(),
        tool_tape_handoff(),
        tool_tape_anchors(),
        tool_web_fetch(),
        tool_subagent(),
        tool_help(),
        tool_quit(),
    ];
    // Only register the sidecar bridge tool if a sidecar URL is configured.
    if crate::tools::SIDECAR_URL.lock().unwrap().is_some() {
        tools.push(tool_sidecar());
    }
    tools
}

// ---------------------------------------------------------------------------
// Helper: resolve a path relative to the workspace.
// ---------------------------------------------------------------------------

fn resolve_path(state: &HashMap<String, Value>, raw_path: &str) -> Result<PathBuf, ConduitError> {
    let path = PathBuf::from(shellexpand::tilde(raw_path).as_ref());
    if path.is_absolute() {
        return Ok(path);
    }
    let workspace = state
        .get("_runtime_workspace")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ConduitError::new(
                ErrorKind::InvalidInput,
                format!("relative path '{raw_path}' is not allowed without a workspace"),
            )
        })?;
    Ok(PathBuf::from(workspace).join(&path))
}

fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn get_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

fn get_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

fn ok_val(s: impl Into<String>) -> ToolResult {
    Ok(Value::String(s.into()))
}

fn current_tape_service() -> Result<TapeService, ConduitError> {
    CURRENT_TAPE_SERVICE.try_with(Clone::clone).map_err(|_| {
        ConduitError::new(
            ErrorKind::Tool,
            "tape tools require an active Eli runtime context",
        )
    })
}

fn tape_name_from_context(ctx: Option<&ToolContext>) -> Result<String, ConduitError> {
    ctx.and_then(|c| c.tape.clone()).ok_or_else(|| {
        ConduitError::new(
            ErrorKind::Tool,
            "tool requires an active tape name in context",
        )
    })
}

fn format_tape_info(info: &crate::builtin::tape::TapeInfo) -> String {
    let last_anchor = info.last_anchor.as_deref().unwrap_or("(none)");
    let last_token_usage = info
        .last_token_usage
        .map(|v| v.to_string())
        .unwrap_or_else(|| "(unknown)".to_owned());
    format!(
        "name: {}\nentries: {}\nanchors: {}\nlast_anchor: {}\nentries_since_last_anchor: {}\nlast_token_usage: {}",
        info.name,
        info.entries,
        info.anchors,
        last_anchor,
        info.entries_since_last_anchor,
        last_token_usage,
    )
}

fn format_anchor_summaries(anchors: &[crate::builtin::tape::AnchorSummary]) -> String {
    if anchors.is_empty() {
        return "(no anchors)".to_owned();
    }

    anchors
        .iter()
        .map(|anchor| {
            let state = if anchor.state.is_empty() {
                "{}".to_owned()
            } else {
                serde_json::to_string(&anchor.state).unwrap_or_else(|_| "{}".to_owned())
            };
            format!("- {} {}", anchor.name, state)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn entry_search_text(entry: &TapeEntry) -> String {
    serde_json::json!({
        "kind": entry.kind,
        "date": entry.date,
        "payload": entry.payload,
        "meta": entry.meta,
    })
    .to_string()
    .to_lowercase()
}

fn render_search_entry(entry: &TapeEntry) -> String {
    let preview = match entry.kind.as_str() {
        "message" => entry
            .payload
            .get("content")
            .and_then(|v| v.as_str())
            .map(|content| shorten_text(content, 160))
            .unwrap_or_else(|| shorten_text(&entry.payload.to_string(), 160)),
        "system" => entry
            .payload
            .get("content")
            .and_then(|v| v.as_str())
            .map(|content| shorten_text(content, 160))
            .unwrap_or_else(|| "(empty system message)".to_owned()),
        "tool_result" | "tool_call" | "event" | "error" | "anchor" => {
            shorten_text(&entry.payload.to_string(), 160)
        }
        _ => shorten_text(&entry.payload.to_string(), 160),
    };
    format!("#{} [{}] {} {}", entry.id, entry.kind, entry.date, preview)
}

// ---------------------------------------------------------------------------
// bash
// ---------------------------------------------------------------------------

fn tool_bash() -> Tool {
    Tool::with_context(
        "bash",
        "Run a shell command. Use background=true to keep it running and fetch output later via bash_output.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "cmd": {"type": "string", "description": "Shell command to execute."},
                "cwd": {"type": "string", "description": "Working directory (optional)."},
                "timeout_seconds": {"type": "integer", "description": "Timeout in seconds (default 30)."},
                "background": {"type": "boolean", "description": "Run in background (default false)."}
            },
            "required": ["cmd"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let cmd = args
                    .get("cmd")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let cwd_arg = args
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned());
                let timeout_secs = get_i64(&args, "timeout_seconds")
                    .unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECONDS as i64)
                    as u64;
                let background = get_bool(&args, "background").unwrap_or(false);

                let workspace = ctx
                    .as_ref()
                    .and_then(|c| c.state.get("_runtime_workspace"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned());
                let target_cwd = cwd_arg.or(workspace);

                let mgr = shell_manager();
                let shell_id = mgr.start(&cmd, target_cwd.as_deref()).await.map_err(|e| {
                    ConduitError::new(ErrorKind::Tool, format!("Failed to start shell: {e}"))
                })?;

                if background {
                    return ok_val(format!("started: {shell_id}"));
                }

                let result = tokio::time::timeout(
                    Duration::from_secs(timeout_secs),
                    mgr.wait_closed(&shell_id),
                )
                .await;

                match result {
                    Ok(Ok((output, returncode, _status))) => {
                        if let Some(code) = returncode
                            && code != 0
                        {
                            let body = output.trim();
                            let body = if body.is_empty() { "(no output)" } else { body };
                            return Err(ConduitError::new(
                                ErrorKind::Tool,
                                format!("command exited with code {code}\noutput:\n{body}"),
                            ));
                        }
                        let trimmed = output.trim();
                        ok_val(if trimmed.is_empty() {
                            "(no output)"
                        } else {
                            trimmed
                        })
                    }
                    Ok(Err(e)) => Err(ConduitError::new(ErrorKind::Tool, format!("{e}"))),
                    Err(_) => {
                        let _ = mgr.terminate(&shell_id).await;
                        ok_val(format!(
                            "command timed out after {timeout_secs} seconds and was terminated"
                        ))
                    }
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// bash.output
// ---------------------------------------------------------------------------

fn tool_bash_output() -> Tool {
    Tool::new(
        "bash.output",
        "Read buffered output from a background shell, with optional offset/limit for incremental polling.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "shell_id": {"type": "string"},
                "offset": {"type": "integer"},
                "limit": {"type": "integer"}
            },
            "required": ["shell_id"]
        }),
        |args: Value, _ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let shell_id = get_str(&args, "shell_id").unwrap_or("").to_owned();
                let offset = get_i64(&args, "offset").unwrap_or(0).max(0) as usize;
                let limit = get_i64(&args, "limit").map(|v| v.max(0) as usize);

                let mgr = shell_manager();
                let (output, returncode, status) = mgr
                    .get(&shell_id)
                    .await
                    .map_err(|e| ConduitError::new(ErrorKind::Tool, format!("{e}")))?;

                // If process exited, finalize.
                if returncode.is_some() {
                    let _ = mgr.wait_closed(&shell_id).await;
                }

                let start = offset.min(output.len());
                let end = match limit {
                    Some(l) => (start + l).min(output.len()),
                    None => output.len(),
                };
                let chunk = output[start..end].trim_end();
                let exit_code = match returncode {
                    Some(c) => c.to_string(),
                    None => "null".to_owned(),
                };
                let body = if chunk.is_empty() {
                    "(no output)"
                } else {
                    chunk
                };
                ok_val(format!(
                    "id: {shell_id}\nstatus: {status}\nexit_code: {exit_code}\nnext_offset: {end}\noutput:\n{body}"
                ))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// bash.kill
// ---------------------------------------------------------------------------

fn tool_bash_kill() -> Tool {
    Tool::new(
        "bash.kill",
        "Terminate a background shell process.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "shell_id": {"type": "string"}
            },
            "required": ["shell_id"]
        }),
        |args: Value, _ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let shell_id = get_str(&args, "shell_id").unwrap_or("").to_owned();
                let mgr = shell_manager();
                let (_output, returncode, status) = mgr
                    .terminate(&shell_id)
                    .await
                    .map_err(|e| ConduitError::new(ErrorKind::Tool, format!("{e}")))?;
                let exit_code = returncode
                    .map(|c| c.to_string())
                    .unwrap_or("null".to_owned());
                ok_val(format!(
                    "id: {shell_id}\nstatus: {status}\nexit_code: {exit_code}"
                ))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// fs.read
// ---------------------------------------------------------------------------

fn tool_fs_read() -> Tool {
    Tool::with_context(
        "fs.read",
        "Read a text file and return its content. Supports optional pagination with offset and limit.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "offset": {"type": "integer"},
                "limit": {"type": "integer"}
            },
            "required": ["path"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let raw_path = get_str(&args, "path").unwrap_or("").to_owned();
                let offset = get_i64(&args, "offset").unwrap_or(0).max(0) as usize;
                let limit = get_i64(&args, "limit").map(|v| v.max(0) as usize);

                let state = ctx.map(|c| c.state).unwrap_or_default();
                let resolved = resolve_path(&state, &raw_path)?;
                let text = std::fs::read_to_string(&resolved)
                    .map_err(|e| ConduitError::new(ErrorKind::Tool, format!("read failed: {e}")))?;

                let lines: Vec<&str> = text.lines().collect();
                let start = offset.min(lines.len());
                let end = match limit {
                    Some(l) => (start + l).min(lines.len()),
                    None => lines.len(),
                };
                ok_val(lines[start..end].join("\n"))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// fs.write
// ---------------------------------------------------------------------------

fn tool_fs_write() -> Tool {
    Tool::with_context(
        "fs.write",
        "Write content to a text file.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let raw_path = get_str(&args, "path").unwrap_or("").to_owned();
                let content = get_str(&args, "content").unwrap_or("").to_owned();

                let state = ctx.map(|c| c.state).unwrap_or_default();
                let resolved = resolve_path(&state, &raw_path)?;

                if let Some(parent) = resolved.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(&resolved, &content).map_err(|e| {
                    ConduitError::new(ErrorKind::Tool, format!("write failed: {e}"))
                })?;
                ok_val(format!("wrote: {}", resolved.display()))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// fs.edit
// ---------------------------------------------------------------------------

fn tool_fs_edit() -> Tool {
    Tool::with_context(
        "fs.edit",
        "Edit a text file by replacing old text with new text. You can specify the line number to start searching.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old": {"type": "string"},
                "new": {"type": "string"},
                "start": {"type": "integer"}
            },
            "required": ["path", "old", "new"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let raw_path = get_str(&args, "path").unwrap_or("").to_owned();
                let old = get_str(&args, "old").unwrap_or("").to_owned();
                let new = get_str(&args, "new").unwrap_or("").to_owned();
                let start = get_i64(&args, "start").unwrap_or(0).max(0) as usize;

                let state = ctx.map(|c| c.state).unwrap_or_default();
                let resolved = resolve_path(&state, &raw_path)?;

                let text = std::fs::read_to_string(&resolved)
                    .map_err(|e| ConduitError::new(ErrorKind::Tool, format!("read failed: {e}")))?;

                let lines: Vec<&str> = text.lines().collect();
                let prefix = lines[..start.min(lines.len())].join("\n");
                let to_replace = lines[start.min(lines.len())..].join("\n");

                if !to_replace.contains(&old) {
                    return Err(ConduitError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "'{}' not found in {} from line {start}",
                            old,
                            resolved.display()
                        ),
                    ));
                }

                let replaced = to_replace.replacen(&old, &new, 1);
                let final_text = if prefix.is_empty() {
                    replaced
                } else {
                    format!("{prefix}\n{replaced}")
                };

                std::fs::write(&resolved, &final_text).map_err(|e| {
                    ConduitError::new(ErrorKind::Tool, format!("write failed: {e}"))
                })?;
                ok_val(format!("edited: {}", resolved.display()))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// skill
// ---------------------------------------------------------------------------

fn tool_skill() -> Tool {
    Tool::with_context(
        "skill",
        "Load the skill content by name. Return the location and skill content.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "required": ["name"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let name = get_str(&args, "name").unwrap_or("").to_owned();
                let state = ctx.map(|c| c.state).unwrap_or_default();

                // Check allowed skills.
                if let Some(Value::Array(allowed)) = state.get("allowed_skills") {
                    let allowed_set: std::collections::HashSet<String> = allowed
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_lowercase())
                        .collect();
                    if !allowed_set.contains(&name.to_lowercase()) {
                        return ok_val(format!("(skill '{name}' is not allowed in this context)"));
                    }
                }

                let workspace = state
                    .get("_runtime_workspace")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

                let skills = discover_skills(&workspace);
                let skill_index: HashMap<String, _> = skills
                    .into_iter()
                    .map(|s| (s.name.to_lowercase(), s))
                    .collect();

                match skill_index.get(&name.to_lowercase()) {
                    Some(skill) => {
                        let body = skill.body().unwrap_or_default();
                        let body_str = if body.is_empty() {
                            "(no content)".to_owned()
                        } else {
                            body
                        };
                        ok_val(format!(
                            "Location: {}\n---\n{body_str}",
                            skill.location.display()
                        ))
                    }
                    None => {
                        // Fallback: check sidecar skills.
                        let sidecar_skills = crate::tools::SIDECAR_SKILLS.lock().unwrap();
                        if let Some(skill) = sidecar_skills
                            .iter()
                            .find(|s| s.name.to_lowercase() == name.to_lowercase())
                        {
                            let body = skill.body().unwrap_or_else(|| "(no content)".to_owned());
                            ok_val(format!("Source: sidecar\n---\n{body}"))
                        } else {
                            ok_val("(no such skill)")
                        }
                    }
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// tape.info
// ---------------------------------------------------------------------------

fn tool_tape_info() -> Tool {
    Tool::with_context(
        "tape.info",
        "Get information about the current tape, such as number of entries and anchors.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
        |_args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let tape_name = tape_name_from_context(ctx.as_ref())?;
                let service = current_tape_service()?;
                let info = service.info(&tape_name).await?;
                ok_val(format_tape_info(&info))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// tape.search
// ---------------------------------------------------------------------------

fn tool_tape_search() -> Tool {
    Tool::with_context(
        "tape.search",
        "Search for entries in the current tape that match the query.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "The search query string."},
                "limit": {"type": "integer", "description": "Max results (default 20)."},
                "start": {"type": "string", "description": "Optional start date (ISO)."},
                "end": {"type": "string", "description": "Optional end date (ISO)."},
                "kinds": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Entry kinds to filter (default: message, tool_result)."
                }
            },
            "required": ["query"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let query_text = get_str(&args, "query").unwrap_or("").to_owned();
                if query_text.trim().is_empty() {
                    return Err(ConduitError::new(
                        ErrorKind::InvalidInput,
                        "query is required",
                    ));
                }
                let limit = get_i64(&args, "limit").unwrap_or(20) as usize;
                let tape_name = tape_name_from_context(ctx.as_ref())?;
                let service = current_tape_service()?;

                let kinds = args
                    .get("kinds")
                    .and_then(|v| v.as_array())
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_str().map(|s| s.to_owned()))
                            .collect::<Vec<_>>()
                    })
                    .filter(|kinds| !kinds.is_empty())
                    .unwrap_or_else(|| vec!["message".to_owned(), "tool_result".to_owned()]);

                let mut query = TapeQuery::new(&tape_name).kinds(kinds);
                if let (Some(start), Some(end)) = (get_str(&args, "start"), get_str(&args, "end")) {
                    query = query.between_dates(start.to_owned(), end.to_owned());
                }

                let entries = service.search(&query).await?;
                let needle = query_text.to_lowercase();
                let matches = entries
                    .into_iter()
                    .filter(|entry| entry_search_text(entry).contains(&needle))
                    .take(limit)
                    .map(|entry| render_search_entry(&entry))
                    .collect::<Vec<_>>();

                if matches.is_empty() {
                    ok_val("(no matches)")
                } else {
                    ok_val(matches.join("\n"))
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// tape.reset
// ---------------------------------------------------------------------------

fn tool_tape_reset() -> Tool {
    Tool::with_context(
        "tape.reset",
        "Reset the current tape, optionally archiving it.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "archive": {"type": "boolean", "description": "Whether to archive before reset."}
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let archive = get_bool(&args, "archive").unwrap_or(false);
                let tape_name = tape_name_from_context(ctx.as_ref())?;
                let service = current_tape_service()?;
                let result = service.reset(&tape_name, archive).await?;
                ok_val(result)
            })
        },
    )
}

// ---------------------------------------------------------------------------
// tape.handoff
// ---------------------------------------------------------------------------

fn tool_tape_handoff() -> Tool {
    Tool::with_context(
        "tape.handoff",
        "Add a handoff anchor to the current tape.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Anchor name (default: handoff)."},
                "summary": {"type": "string", "description": "Summary text."}
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let name = get_str(&args, "name").unwrap_or("handoff").to_owned();
                let summary = get_str(&args, "summary").unwrap_or("").to_owned();
                let tape_name = tape_name_from_context(ctx.as_ref())?;
                let service = current_tape_service()?;
                let state = if summary.is_empty() {
                    None
                } else {
                    Some(serde_json::json!({"summary": summary}))
                };
                service.handoff(&tape_name, &name, state).await?;
                ok_val(format!("anchor added: {name}"))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// tape.anchors
// ---------------------------------------------------------------------------

fn tool_tape_anchors() -> Tool {
    Tool::with_context(
        "tape.anchors",
        "List anchors in the current tape.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "limit": {"type": "integer", "description": "Max anchors to return (default 20)."}
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let limit = get_i64(&args, "limit").unwrap_or(20) as usize;
                let tape_name = tape_name_from_context(ctx.as_ref())?;
                let service = current_tape_service()?;
                let anchors = service.anchors(&tape_name, limit).await?;
                ok_val(format_anchor_summaries(&anchors))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// web.fetch
// ---------------------------------------------------------------------------

fn tool_web_fetch() -> Tool {
    Tool::new(
        "web.fetch",
        "Fetch(GET) the content of a web page, returning markdown if possible.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {"type": "string"},
                "headers": {"type": "object"},
                "timeout": {"type": "integer"}
            },
            "required": ["url"]
        }),
        |args: Value, _ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let url = get_str(&args, "url").unwrap_or("").to_owned();
                let timeout_secs = get_i64(&args, "timeout")
                    .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECONDS as i64)
                    as u64;

                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(timeout_secs))
                    .build()
                    .map_err(|e| {
                        ConduitError::new(ErrorKind::Tool, format!("http client error: {e}"))
                    })?;

                let mut request = client.get(&url);
                request = request.header("accept", "text/markdown");

                // Merge user-provided headers.
                if let Some(Value::Object(headers)) = args.get("headers") {
                    for (k, v) in headers {
                        if let Some(val) = v.as_str() {
                            request = request.header(k.as_str(), val);
                        }
                    }
                }

                let response = request.send().await.map_err(|e| {
                    ConduitError::new(ErrorKind::Tool, format!("fetch failed: {e}"))
                })?;
                let status = response.status();
                if !status.is_success() {
                    return Err(ConduitError::new(
                        ErrorKind::Tool,
                        format!("HTTP {status} for {url}"),
                    ));
                }
                let text = response.text().await.map_err(|e| {
                    ConduitError::new(ErrorKind::Tool, format!("read body failed: {e}"))
                })?;
                ok_val(text)
            })
        },
    )
}

// ---------------------------------------------------------------------------
// subagent
// ---------------------------------------------------------------------------

fn tool_subagent() -> Tool {
    Tool::with_context(
        "subagent",
        "Run a task with sub-agent using specific model and session.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {"description": "The prompt for the sub-agent."},
                "model": {"type": "string", "description": "Model to use (optional)."},
                "session": {"type": "string", "description": "Session strategy: inherit, temp, or custom id."},
                "allowed_tools": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Allowed tool names."
                },
                "allowed_skills": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Allowed skill names."
                }
            },
            "required": ["prompt"]
        }),
        |args: Value, _ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                // The subagent tool requires the full agent runtime in state.
                // In the initial Rust port we return the prompt as acknowledgement.
                let prompt = args
                    .get("prompt")
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => serde_json::to_string(other).unwrap_or_default(),
                    })
                    .unwrap_or_default();
                ok_val(format!("(subagent invoked with prompt: {prompt})"))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// help
// ---------------------------------------------------------------------------

fn tool_help() -> Tool {
    Tool::new(
        "help",
        "Show a help message.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
        |_args: Value, _ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                ok_val(
                    "Commands use ',' at line start.\n\
                     Known internal commands:\n\
                     \x20 ,help\n\
                     \x20 ,skill name=foo\n\
                     \x20 ,tape.info\n\
                     \x20 ,tape.search query=error\n\
                     \x20 ,tape.handoff name=phase-1 summary='done'\n\
                     \x20 ,tape.anchors\n\
                     \x20 ,fs.read path=README.md\n\
                     \x20 ,fs.write path=tmp.txt content='hello'\n\
                     \x20 ,fs.edit path=tmp.txt old=hello new=world\n\
                     \x20 ,bash cmd='sleep 5' background=true\n\
                     \x20 ,bash.output shell_id=bsh-12345678\n\
                     \x20 ,bash.kill shell_id=bsh-12345678\n\
                     \x20 ,quit\n\
                     Any unknown command after ',' is executed as shell via bash.",
                )
            })
        },
    )
}

// ---------------------------------------------------------------------------
// quit
// ---------------------------------------------------------------------------

fn tool_quit() -> Tool {
    Tool::with_context(
        "quit",
        "Quit the tasks of the current session.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
        |_args: Value, _ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move { ok_val("Session tasks stopped.") })
        },
    )
}

// ---------------------------------------------------------------------------
// Sidecar bridge tool — proxies calls to the sidecar's /tools/:name endpoint.
// ---------------------------------------------------------------------------

fn tool_sidecar() -> Tool {
    Tool::new(
        "sidecar",
        "Execute a sidecar plugin tool by name. Use the `skill` tool first to discover available tool names and their parameters.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool": {
                    "type": "string",
                    "description": "The sidecar tool name to execute (e.g. feishu_calendar_event)."
                },
                "params": {
                    "type": "object",
                    "description": "Parameters for the tool."
                }
            },
            "required": ["tool"]
        }),
        |args: Value, _ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let tool_name = get_str(&args, "tool").unwrap_or("").to_owned();
                let params = args.get("params").cloned().unwrap_or(serde_json::json!({}));

                if tool_name.is_empty() {
                    return Err(ConduitError::new(
                        ErrorKind::Tool,
                        "missing required parameter: tool",
                    ));
                }

                // Validate tool exists.
                let exists = {
                    let tools = crate::tools::SIDECAR_TOOLS.lock().unwrap();
                    tools.contains_key(&tool_name)
                };
                if !exists {
                    return Err(ConduitError::new(
                        ErrorKind::Tool,
                        format!("unknown sidecar tool: {tool_name}"),
                    ));
                }

                let url = {
                    let u = crate::tools::SIDECAR_URL.lock().unwrap();
                    u.clone().unwrap_or_default()
                };
                if url.is_empty() {
                    return Err(ConduitError::new(ErrorKind::Tool, "sidecar not running"));
                }

                let tool_url = format!("{url}/tools/{tool_name}");
                let client = reqwest::Client::new();
                let resp = client
                    .post(&tool_url)
                    .json(&serde_json::json!({ "params": params }))
                    .send()
                    .await
                    .map_err(|e| {
                        ConduitError::new(ErrorKind::Tool, format!("sidecar request failed: {e}"))
                    })?;

                let body: serde_json::Value = resp.json().await.map_err(|e| {
                    ConduitError::new(
                        ErrorKind::Tool,
                        format!("sidecar response parse failed: {e}"),
                    )
                })?;

                if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
                    return Err(ConduitError::new(ErrorKind::Tool, err.to_owned()));
                }

                Ok(body)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::store::{FileTapeStore, ForkTapeStore};
    use serde_json::json;

    fn test_tape_service() -> (tempfile::TempDir, TapeService, String) {
        let tmp = tempfile::tempdir().unwrap();
        let tapes_dir = tmp.path().join("tapes");
        let store = ForkTapeStore::from_sync(FileTapeStore::new(tapes_dir.clone()));
        let service = TapeService::new(tapes_dir, store);
        let tape_name = "workspace__session".to_owned();
        (tmp, service, tape_name)
    }

    #[tokio::test]
    async fn test_tape_info_tool_uses_runtime_service() {
        let (_tmp, service, tape_name) = test_tape_service();
        service.ensure_bootstrap_anchor(&tape_name).await.unwrap();
        service
            .append_event(&tape_name, "run", json!({"usage": {"total_tokens": 42}}))
            .await
            .unwrap();

        let tool = tool_tape_info();
        let ctx = ToolContext::new("test-run").with_tape(tape_name.clone());
        let value = with_tape_runtime(service.clone(), async move {
            tool.run(json!({}), Some(ctx)).await.unwrap()
        })
        .await;

        let output = value.as_str().unwrap();
        assert!(output.contains("name: workspace__session"));
        assert!(output.contains("anchors: 1"));
    }

    #[tokio::test]
    async fn test_tape_search_tool_filters_entries() {
        let (_tmp, service, tape_name) = test_tape_service();
        service.ensure_bootstrap_anchor(&tape_name).await.unwrap();
        service
            .store()
            .append(
                &tape_name,
                &TapeEntry::message(
                    json!({"role": "user", "content": "hello needle"}),
                    json!({}),
                ),
            )
            .await
            .unwrap();
        service
            .store()
            .append(
                &tape_name,
                &TapeEntry::message(json!({"role": "user", "content": "different"}), json!({})),
            )
            .await
            .unwrap();

        let tool = tool_tape_search();
        let ctx = ToolContext::new("test-run").with_tape(tape_name.clone());
        let value = with_tape_runtime(service.clone(), async move {
            tool.run(json!({"query": "needle"}), Some(ctx))
                .await
                .unwrap()
        })
        .await;

        let output = value.as_str().unwrap();
        assert!(output.contains("needle"));
        assert!(!output.contains("different"));
    }
}
