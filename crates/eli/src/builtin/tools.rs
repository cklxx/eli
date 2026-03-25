//! Builtin tool implementations: bash, fs, tape, web, subagent, etc.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use futures::future::BoxFuture;
use nexil::tools::schema::ToolResult;
use nexil::{ConduitError, ErrorKind};
use nexil::{TapeEntry, TapeQuery, Tool, ToolContext};
use serde_json::Value;

use crate::builtin::shell_manager::shell_manager;
use crate::builtin::tape::TapeService;
use crate::envelope::ValueExt;
use crate::skills::discover_skills;
use crate::tools::{REGISTRY, shorten_text};

const DEFAULT_COMMAND_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 10;
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024; // 10MB
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024; // 50MB

static HTTP_CLIENT: std::sync::LazyLock<reqwest::Client> =
    std::sync::LazyLock::new(reqwest::Client::new);

tokio::task_local! {
    static CURRENT_TAPE_SERVICE: TapeService;
}

/// Register all builtin tools into the global `REGISTRY`.
pub fn register_builtin_tools() {
    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    reg.extend(builtin_tools().into_iter().map(|t| (t.name.clone(), t)));
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
        tool_decision_set(),
        tool_decision_list(),
        tool_decision_remove(),
        tool_web_fetch(),
        tool_subagent(),
        tool_message_send(),
        tool_help(),
        tool_quit(),
    ];
    if crate::tools::SIDECAR_URL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
    {
        tools.push(tool_sidecar());
    }
    tools
}

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

fn invalid_input(error: anyhow::Error) -> ConduitError {
    ConduitError::new(ErrorKind::InvalidInput, error.to_string())
}

fn get_notice_description(args: &Value) -> Option<&str> {
    args.get_str_field("description")
        .map(str::trim)
        .filter(|s| !s.is_empty())
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

async fn maybe_send_user_facing_notice(ctx: Option<&ToolContext>, args: &Value) {
    if let Some((description, session_id, url)) = extract_notice_params(ctx, args) {
        send_notice(&url, session_id, description).await;
    }
}

fn extract_notice_params<'a>(
    ctx: Option<&'a ToolContext>,
    args: &'a Value,
) -> Option<(&'a str, &'a str, String)> {
    let description = get_notice_description(args)?;
    let ctx = ctx?;
    let output_channel = ctx.state.get("output_channel").and_then(|v| v.as_str())?;
    if output_channel != "webhook" {
        return None;
    }
    let session_id = ctx
        .state
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())?;
    let url = crate::tools::SIDECAR_URL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()?;
    Some((description, session_id, url))
}

async fn send_notice(url: &str, session_id: &str, description: &str) {
    let payload = serde_json::json!({
        "session_id": session_id,
        "text": description,
    });
    let notify_url = format!("{url}/notify");
    let mut req = HTTP_CLIENT
        .post(&notify_url)
        .timeout(Duration::from_secs(3))
        .json(&payload);
    if let Ok(token) = std::env::var("ELI_SIDECAR_TOKEN") {
        req = req.bearer_auth(&token);
    }
    if let Err(err) = req.send().await {
        tracing::debug!(error = %err, session_id, notify_url, "tool.notice request failed");
    }
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
    let preview = entry
        .payload
        .get("content")
        .and_then(|v| v.as_str())
        .filter(|_| matches!(entry.kind.as_str(), "message" | "system"))
        .map(|content| shorten_text(content, 160))
        .unwrap_or_else(|| shorten_text(&entry.payload.to_string(), 160));
    format!("#{} [{}] {} {}", entry.id, entry.kind, entry.date, preview)
}

// ---------------------------------------------------------------------------
// bash
// ---------------------------------------------------------------------------

fn tool_bash() -> Tool {
    Tool::with_context(
        "bash",
        "Run a shell command and return its output.\n\nExamples: `cargo build`, `git diff`, `grep -rn TODO src/`, `npm test`, `docker ps`, `ls -la`.\nLong-running processes (servers, watchers, log tails): set background=true, then poll with bash.output.\nFile I/O: prefer fs.read / fs.write over cat / echo redirects.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "cmd": {"type": "string", "description": "Shell command to execute."},
                "description": {"type": "string", "description": "Brief description of what this command does and why."},
                "cwd": {"type": "string", "description": "Absolute working directory for the command."},
                "timeout_seconds": {"type": "integer", "description": "Kill the process after N seconds (default 30). Ignored when background=true."},
                "background": {"type": "boolean", "description": "Run asynchronously. Returns a shell_id immediately — poll with bash.output, stop with bash.kill."}
            },
            "required": ["cmd", "description"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
                let cmd = args
                    .require_str_field("cmd")
                    .map_err(invalid_input)?
                    .to_owned();
                let cwd_arg = args
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned());
                let timeout_secs = args
                    .get_i64_field("timeout_seconds")
                    .unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECONDS as i64)
                    as u64;
                let background = args.get_bool_field("background").unwrap_or(false);

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
        "Read output from a background shell started with bash(background=true).\n\nExamples: tail a dev-server log, watch a long build, capture test output after completion. Pass offset to read only new bytes since last poll.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "shell_id": {"type": "string", "description": "The background shell ID returned by bash."},
                "offset": {"type": "integer", "description": "Character offset to resume reading from (use next_offset from previous call)."},
                "limit": {"type": "integer", "description": "Max characters to return per call."}
            },
            "required": ["shell_id"]
        }),
        |args: Value, _ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let shell_id = args
                    .require_str_field("shell_id")
                    .map_err(invalid_input)?
                    .to_owned();
                let offset = args.get_i64_field("offset").unwrap_or(0).max(0) as usize;
                let limit = args.get_i64_field("limit").map(|v| v.max(0) as usize);

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
        "Terminate a background shell by shell_id.\n\nExamples: stop a dev server after testing, cancel a hung compilation, clean up a finished log tail.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "shell_id": {"type": "string", "description": "The background shell ID to terminate."}
            },
            "required": ["shell_id"]
        }),
        |args: Value, _ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let shell_id = args
                    .require_str_field("shell_id")
                    .map_err(invalid_input)?
                    .to_owned();
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
        "Read a file and return its text content.\n\nExamples: inspect source code, check a config file, view build logs, examine generated output. Use offset/limit to paginate large files. Prefer over bash(cat/head).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (absolute or relative to workspace)."},
                "description": {"type": "string", "description": "Brief user-facing status text to send before reading when the channel supports it."},
                "offset": {"type": "integer", "description": "Line number to start reading from (0-based)."},
                "limit": {"type": "integer", "description": "Max number of lines to return."}
            },
            "required": ["path"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
                let raw_path = args
                    .require_str_field("path")
                    .map_err(invalid_input)?
                    .to_owned();
                let offset = args.get_i64_field("offset").unwrap_or(0).max(0) as usize;
                let limit = args.get_i64_field("limit").map(|v| v.max(0) as usize);

                let state = ctx.map(|c| c.state).unwrap_or_default();
                let resolved = resolve_path(&state, &raw_path)?;

                let meta = std::fs::metadata(&resolved)
                    .map_err(|e| ConduitError::new(ErrorKind::Tool, format!("read failed: {e}")))?;
                if meta.len() > MAX_FILE_BYTES {
                    return Err(ConduitError::new(
                        ErrorKind::Tool,
                        format!(
                            "file too large ({} bytes, limit {})",
                            meta.len(),
                            MAX_FILE_BYTES
                        ),
                    ));
                }

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
        "Create a new file or fully overwrite an existing one.\n\nExamples: scaffold a new module, generate a config, write test fixtures, save structured output. Auto-creates parent dirs. For partial changes, use fs.edit.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (absolute or relative to workspace)."},
                "description": {"type": "string", "description": "Brief user-facing status text to send before writing when the channel supports it."},
                "content": {"type": "string", "description": "Full file content to write."}
            },
            "required": ["path", "content"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
                let raw_path = args
                    .require_str_field("path")
                    .map_err(invalid_input)?
                    .to_owned();
                let content = args
                    .require_str_field("content")
                    .map_err(invalid_input)?
                    .to_owned();

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
        "Find-and-replace text in a file (first match only).\n\nExamples: rename a variable, fix a typo, update an import path, change a config value. Use start line to disambiguate when old text appears multiple times. For full rewrites, use fs.write.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (absolute or relative to workspace)."},
                "description": {"type": "string", "description": "Brief user-facing status text to send before editing when the channel supports it."},
                "old": {"type": "string", "description": "Exact text to find and replace (first occurrence only)."},
                "new": {"type": "string", "description": "Replacement text."},
                "start": {"type": "integer", "description": "Line number to start searching from (0-based, optional)."}
            },
            "required": ["path", "old", "new"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
                let raw_path = args
                    .require_str_field("path")
                    .map_err(invalid_input)?
                    .to_owned();
                let old = args
                    .require_str_field("old")
                    .map_err(invalid_input)?
                    .to_owned();
                let new = args
                    .require_str_field("new")
                    .map_err(invalid_input)?
                    .to_owned();
                let start = args.get_i64_field("start").unwrap_or(0).max(0) as usize;

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
        "Load a skill by name and return its instructions.\n\nExamples: look up a sidecar tool's parameter schema, read a workflow's step-by-step guide, check what capabilities a plugin provides.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Skill name (e.g. 'deploy', 'feishu-calendar')."}
            },
            "required": ["name"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let name = args
                    .require_str_field("name")
                    .map_err(invalid_input)?
                    .to_owned();
                let state = ctx.map(|c| c.state).unwrap_or_default();

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
                    None => ok_val("(no such skill)"),
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
        "Get tape metadata: entry count, anchors, token usage.\n\nExamples: check context size before a handoff, decide whether a reset is needed, monitor token consumption.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": {"type": "string", "description": "Brief user-facing status text to send before reading tape info when the channel supports it."}
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
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
        "Search the conversation tape by keyword.\n\nExamples: recall a previous decision, find an earlier tool result, locate an error from a past turn, review what was discussed in a date range. For file content search, use bash(grep).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Keyword to search for in tape entries."},
                "description": {"type": "string", "description": "Brief user-facing status text to send before searching tape when the channel supports it."},
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
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
                let query_text = args
                    .require_str_field("query")
                    .map_err(invalid_input)?
                    .to_owned();
                if query_text.trim().is_empty() {
                    return Err(ConduitError::new(
                        ErrorKind::InvalidInput,
                        "query must not be empty",
                    ));
                }
                let limit = args.get_i64_field("limit").unwrap_or(20) as usize;
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
                if let (Some(start), Some(end)) =
                    (args.get_str_field("start"), args.get_str_field("end"))
                {
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
        "Wipe the current tape and start fresh.\n\nExamples: context grew too large, task shifted entirely, need to discard a failed exploration. Set archive=true to preserve a snapshot — without it the wipe is irreversible.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": {"type": "string", "description": "Brief user-facing status text to send before resetting tape when the channel supports it."},
                "archive": {"type": "boolean", "description": "Save a tape snapshot before wiping (default false)."}
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
                let archive = args.get_bool_field("archive").unwrap_or(false);
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
        "Save a named checkpoint (anchor) to the tape with a summary.\n\nExamples: mark a phase as complete, create a resumption point before switching tasks, record state before handing off to another agent.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": {"type": "string", "description": "Brief user-facing status text to send before creating a handoff when the channel supports it."},
                "name": {"type": "string", "description": "Anchor name (default: handoff)."},
                "summary": {"type": "string", "description": "What was accomplished — used for context when resuming later."}
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
                let name = args.get_str_field("name").unwrap_or("handoff").to_owned();
                let summary = args.get_str_field("summary").unwrap_or("").to_owned();
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
        "List all anchors (checkpoints) in the tape.\n\nExamples: review the session timeline, find a handoff point to resume from, check how many phases have been completed.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": {"type": "string", "description": "Brief user-facing status text to send before listing anchors when the channel supports it."},
                "limit": {"type": "integer", "description": "Max anchors to return (default 20)."}
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
                let limit = args.get_i64_field("limit").unwrap_or(20) as usize;
                let tape_name = tape_name_from_context(ctx.as_ref())?;
                let service = current_tape_service()?;
                let anchors = service.anchors(&tape_name, limit).await?;
                ok_val(format_anchor_summaries(&anchors))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// decision.set / decision.list / decision.remove
// ---------------------------------------------------------------------------

/// Maximum decision text length before truncation.
const MAX_DECISION_TEXT_LEN: usize = 500;

fn tool_decision_set() -> Tool {
    Tool::with_context(
        "decision.set",
        "Pin a decision so it persists across turns and anchor boundaries.\n\nExamples: lock in a tech choice after discussion, record an agreed architecture constraint before moving on, capture a deployment target once confirmed by the user.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "The decision to record."}
            },
            "required": ["text"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let text = args
                    .require_str_field("text")
                    .map_err(invalid_input)?
                    .to_owned();
                if text.trim().is_empty() {
                    return Err(ConduitError::new(
                        ErrorKind::InvalidInput,
                        "decision text must not be empty",
                    ));
                }
                let text = if text.len() > MAX_DECISION_TEXT_LEN {
                    let truncated = &text[..text.floor_char_boundary(MAX_DECISION_TEXT_LEN)];
                    format!("{}...", truncated)
                } else {
                    text
                };
                let tape_name = tape_name_from_context(ctx.as_ref())?;
                let service = current_tape_service()?;
                let meta = serde_json::json!({});
                let entry = TapeEntry::decision(&text, meta);
                service.store().append(&tape_name, &entry).await?;
                tracing::info!(decision = %text, tape = %tape_name, "decision.set");
                ok_val(format!("Decision recorded: {text}"))
            })
        },
    )
}

fn tool_decision_list() -> Tool {
    Tool::with_context(
        "decision.list",
        "Show active decisions for this session.\n\nExamples: verify assumptions before starting a new task, check for stale decisions after scope changes, recap context when resuming after a break.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
        |_args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let tape_name = tape_name_from_context(ctx.as_ref())?;
                let service = current_tape_service()?;
                let query = TapeQuery::new(&tape_name);
                let entries = service.store().fetch_all(&query).await?;
                let decisions = nexil::collect_active_decisions(&entries);
                if decisions.is_empty() {
                    return ok_val("No active decisions.");
                }
                let mut output = format!("Active decisions ({}):\n", decisions.len());
                for (i, d) in decisions.iter().enumerate() {
                    output.push_str(&format!("  {}. {}\n", i + 1, d));
                }
                ok_val(output.trim_end())
            })
        },
    )
}

fn tool_decision_remove() -> Tool {
    Tool::with_context(
        "decision.remove",
        "Revoke a decision by its number (from decision.list).\n\nExamples: drop a tech choice after pivoting, clear a constraint the user overruled, remove a duplicate created by mistake.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "index": {"type": "integer", "description": "The decision number to remove (1-based, from decision.list)."}
            },
            "required": ["index"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let index = args.get_i64_field("index").ok_or_else(|| {
                    ConduitError::new(ErrorKind::InvalidInput, "missing required argument 'index'")
                })? as usize;
                if index == 0 {
                    return Err(ConduitError::new(
                        ErrorKind::InvalidInput,
                        "index must be 1 or greater",
                    ));
                }
                let tape_name = tape_name_from_context(ctx.as_ref())?;
                let service = current_tape_service()?;
                let query = TapeQuery::new(&tape_name);
                let entries = service.store().fetch_all(&query).await?;
                let decisions = nexil::collect_active_decisions(&entries);
                if index > decisions.len() {
                    return Err(ConduitError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "no decision #{index}. There are {} active decisions.",
                            decisions.len()
                        ),
                    ));
                }
                let text = &decisions[index - 1];
                let meta = serde_json::json!({});
                let tombstone = TapeEntry::decision_revoked(text, meta);
                service.store().append(&tape_name, &tombstone).await?;
                tracing::info!(decision = %text, tape = %tape_name, "decision.remove");
                ok_val(format!("Removed decision: {text}"))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// web.fetch
// ---------------------------------------------------------------------------

fn tool_web_fetch() -> Tool {
    Tool::with_context(
        "web.fetch",
        "Fetch a URL (HTTP GET) and return content as markdown.\n\nExamples: read documentation, check a REST API response, pull a raw GitHub file, retrieve release notes. Supports custom headers and timeout. Static content only — no JS rendering.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "The URL to fetch."},
                "description": {"type": "string", "description": "Brief user-facing status text to send before fetching when the channel supports it."},
                "headers": {"type": "object", "description": "Custom HTTP headers as key-value pairs."},
                "timeout": {"type": "integer", "description": "Request timeout in seconds (default 10)."}
            },
            "required": ["url"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
                let url = args
                    .require_str_field("url")
                    .map_err(invalid_input)?
                    .to_owned();
                let timeout_secs = args
                    .get_i64_field("timeout")
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
                let bytes = response.bytes().await.map_err(|e| {
                    ConduitError::new(ErrorKind::Tool, format!("read body failed: {e}"))
                })?;
                if bytes.len() > MAX_RESPONSE_BYTES {
                    return Err(ConduitError::new(
                        ErrorKind::Tool,
                        format!(
                            "response too large ({} bytes, limit {})",
                            bytes.len(),
                            MAX_RESPONSE_BYTES
                        ),
                    ));
                }
                let text = String::from_utf8_lossy(&bytes).into_owned();
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
        "[EXPERIMENTAL] Spawn an isolated sub-agent with its own context.\n\nExamples: parallelize independent research, delegate a focused coding subtask, explore a codebase without polluting the main tape. Configure model, session strategy, and tool/skill allowlists. Currently returns prompt acknowledgement only — full isolation not yet implemented.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "Task description for the sub-agent."},
                "description": {"type": "string", "description": "Brief user-facing status text to send before delegating when the channel supports it."},
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
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice(ctx.as_ref(), &args).await;
                let prompt = match args.get("prompt") {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => serde_json::to_string(other).unwrap_or_default(),
                    None => {
                        return Err(ConduitError::new(
                            ErrorKind::InvalidInput,
                            "missing required argument 'prompt'",
                        ));
                    }
                };
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
        "Show available commands and their syntax.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
        |_args: Value, _ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                ok_val(
                    "Commands use '/' at line start.\n\
                     Known internal commands:\n\
                     \x20 /help\n\
                     \x20 /skill name=foo\n\
                     \x20 /tape.info\n\
                     \x20 /tape.search query=error\n\
                     \x20 /tape.handoff name=phase-1 summary='done'\n\
                     \x20 /tape.anchors\n\
                     \x20 /fs.read path=README.md\n\
                     \x20 /fs.write path=tmp.txt content='hello'\n\
                     \x20 /fs.edit path=tmp.txt old=hello new=world\n\
                     \x20 /bash cmd='sleep 5' background=true\n\
                     \x20 /bash.output shell_id=bsh-12345678\n\
                     \x20 /bash.kill shell_id=bsh-12345678\n\
                     \x20 /quit\n\
                     Any unknown command after '/' is executed as shell via bash.",
                )
            })
        },
    )
}

// ---------------------------------------------------------------------------
// quit
// ---------------------------------------------------------------------------

fn tool_message_send() -> Tool {
    Tool::with_context(
        "message.send",
        "Send a message to the user immediately, without waiting for the turn to finish.\n\nUse this to acknowledge the user's request before starting long-running work, or to provide progress updates mid-task. The message is dispatched to the same channel the user sent from.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "The message text to send to the user."}
            },
            "required": ["text"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let text = args
                    .require_str_field("text")
                    .map_err(invalid_input)?
                    .to_owned();
                if text.trim().is_empty() {
                    return ok_val("skipped: empty message");
                }

                let ctx = ctx.ok_or_else(|| {
                    ConduitError::new(ErrorKind::InvalidInput, "no tool context available")
                })?;
                let state = &ctx.state;

                let envelope = serde_json::json!({
                    "content": text,
                    "session_id": state.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "channel": state.get("channel").and_then(|v| v.as_str()).unwrap_or(""),
                    "chat_id": state.get("chat_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "output_channel": state.get("output_channel").and_then(|v| v.as_str()).unwrap_or(""),
                });

                crate::control_plane::dispatch_mid_turn(envelope).await;
                ok_val("sent")
            })
        },
    )
}

fn tool_quit() -> Tool {
    Tool::with_context(
        "quit",
        "End the session and stop all running tasks.",
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
    Tool::with_context(
        "sidecar",
        "Call an external sidecar plugin by tool name.\n\nExamples: create a calendar event, read/write a Feishu doc, trigger a CI pipeline. Always load the skill first to discover the tool's required parameters.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool": {
                    "type": "string",
                    "description": "The sidecar tool name to execute (e.g. feishu_calendar_event)."
                },
                "description": {
                    "type": "string",
                    "description": "Brief user-facing status text to send before executing an external action when the channel supports it."
                },
                "params": {
                    "type": "object",
                    "description": "Parameters for the tool."
                }
            },
            "required": ["tool"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let tool_name = args
                    .require_str_field("tool")
                    .map_err(invalid_input)?
                    .to_owned();
                let description = args
                    .get_str_field("description")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned);
                let params = args.get("params").cloned().unwrap_or(serde_json::json!({}));

                let url = {
                    let u = crate::tools::SIDECAR_URL
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    u.clone().unwrap_or_default()
                };
                if url.is_empty() {
                    return Err(ConduitError::new(ErrorKind::Tool, "sidecar not running"));
                }

                let session_id = ctx
                    .as_ref()
                    .and_then(|c| c.state.get("session_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let tool_url = format!("{url}/tools/{tool_name}");
                let payload =
                    build_sidecar_request_payload(params, description.as_deref(), session_id);
                let mut req = HTTP_CLIENT.post(&tool_url).json(&payload);
                if let Ok(token) = std::env::var("ELI_SIDECAR_TOKEN") {
                    req = req.bearer_auth(&token);
                }
                let resp = req.send().await.map_err(|e| {
                    ConduitError::new(ErrorKind::Tool, format!("sidecar request failed: {e}"))
                })?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(ConduitError::new(
                        ErrorKind::Tool,
                        format!("sidecar returned {status}: {body}"),
                    ));
                }

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

fn build_sidecar_request_payload(
    params: Value,
    description: Option<&str>,
    session_id: &str,
) -> Value {
    let mut payload = serde_json::json!({ "params": params });
    if let Some(description) = description.map(str::trim).filter(|s| !s.is_empty()) {
        payload["description"] = Value::String(description.to_owned());
    }
    if !session_id.is_empty() {
        payload["session_id"] = Value::String(session_id.to_owned());
    }
    payload
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

    #[test]
    fn test_build_sidecar_request_payload_uses_top_level_description_metadata() {
        let payload = build_sidecar_request_payload(
            json!({"action": "create", "description": "domain field"}),
            Some("同步飞书日程"),
            "session-1",
        );

        assert_eq!(payload["description"], json!("同步飞书日程"));
        assert_eq!(payload["session_id"], json!("session-1"));
        assert_eq!(
            payload["params"],
            json!({"action": "create", "description": "domain field"})
        );
    }

    #[test]
    fn test_user_facing_tools_expose_description_field() {
        let tools = [
            tool_bash(),
            tool_fs_read(),
            tool_fs_write(),
            tool_fs_edit(),
            tool_tape_info(),
            tool_tape_search(),
            tool_tape_reset(),
            tool_tape_handoff(),
            tool_tape_anchors(),
            tool_web_fetch(),
            tool_subagent(),
            tool_sidecar(),
        ];

        for tool in tools {
            assert_eq!(
                tool.parameters["properties"]["description"]["type"],
                json!("string"),
                "tool {} should expose a description field",
                tool.name
            );
        }
    }
}
