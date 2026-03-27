//! Builtin tool implementations: bash, fs, tape, web, subagent, etc.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::future::BoxFuture;
use nexil::tools::schema::ToolResult;
use nexil::{ConduitError, ErrorKind};
use nexil::{TapeEntry, TapeQuery, Tool, ToolContext};
use serde_json::Value;
use tempfile::NamedTempFile;

use crate::builtin::shell_manager::shell_manager;
use crate::builtin::tape::TapeService;
use crate::envelope::ValueExt;
use crate::skills::discover_skills;
use crate::tools::{REGISTRY, shorten_text};

const DEFAULT_COMMAND_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 10;
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024; // 10MB
const DEFAULT_READ_LINE_LIMIT: usize = 500;

static HTTP_CLIENT: std::sync::LazyLock<reqwest::Client> =
    std::sync::LazyLock::new(reqwest::Client::new);

/// Maximum characters of CLI output included in the subagent completion message.
const SUBAGENT_OUTPUT_TAIL: usize = 2000;

// ---------------------------------------------------------------------------
// Subagent CLI detection
// ---------------------------------------------------------------------------

/// Info about a detected coding CLI binary.
#[derive(Clone, Debug)]
struct CliInfo {
    name: String,
    path: String,
}

/// Ordered list of coding CLIs to probe.
const CLI_CANDIDATES: &[&str] = &["claude", "codex", "kimi"];

static DETECTED_CLI: std::sync::LazyLock<std::sync::Mutex<Option<CliInfo>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

fn detect_cli() -> Option<CliInfo> {
    let mut cache = DETECTED_CLI.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref info) = *cache {
        return Some(info.clone());
    }
    for &name in CLI_CANDIDATES {
        if let Ok(output) = std::process::Command::new("which").arg(name).output()
            && output.status.success()
        {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let info = CliInfo {
                name: name.to_owned(),
                path,
            };
            *cache = Some(info.clone());
            return Some(info);
        }
    }
    None
}

fn resolve_cli(explicit: Option<&str>) -> Result<CliInfo, ConduitError> {
    if let Some(name) = explicit {
        let output = std::process::Command::new("which")
            .arg(name)
            .output()
            .map_err(|e| ConduitError::new(ErrorKind::Tool, format!("which {name}: {e}")))?;
        if !output.status.success() {
            return Err(ConduitError::new(
                ErrorKind::Tool,
                format!("CLI '{name}' not found in PATH"),
            ));
        }
        let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        return Ok(CliInfo {
            name: name.to_owned(),
            path,
        });
    }
    detect_cli().ok_or_else(|| {
        ConduitError::new(
            ErrorKind::Tool,
            format!(
                "no coding CLI found in PATH (tried: {})",
                CLI_CANDIDATES.join(", ")
            ),
        )
    })
}

fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_owned();
    }
    if s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_-./=".contains(&b))
    {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn build_cli_command(cli: &CliInfo, prompt_file: &str) -> String {
    let bin = shell_quote(&cli.path);
    let file = shell_quote(prompt_file);
    match cli.name.as_str() {
        // claude -p reads from stdin when no positional prompt is given.
        "claude" => format!("{bin} -p --output-format text < {file}"),
        // codex exec reads from stdin when prompt arg is `-` or omitted.
        "codex" => format!("{bin} exec < {file}"),
        // kimi -p takes the prompt as a direct argument; use $() to read from file.
        "kimi" => format!("{bin} -p \"$(cat {file})\" --print"),
        // Fallback: assume stdin piping works.
        _ => format!("{bin} < {file}"),
    }
}

fn write_prompt_tempfile(prompt: &str) -> Result<NamedTempFile, ConduitError> {
    let mut f = tempfile::Builder::new()
        .prefix(".eli-prompt-")
        .tempfile()
        .map_err(|e| ConduitError::new(ErrorKind::Tool, format!("prompt tempfile: {e}")))?;
    f.write_all(prompt.as_bytes())
        .map_err(|e| ConduitError::new(ErrorKind::Tool, format!("write prompt: {e}")))?;
    Ok(f)
}

fn snapshot_git_head(workspace: &str) -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

async fn collect_artifacts(workspace: &str, pre_head: Option<&str>) -> String {
    let is_git = tokio::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !is_git {
        return "(not a git repo)".to_owned();
    }

    let mut parts: Vec<String> = Vec::new();

    // Current HEAD.
    let current_head = tokio::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace)
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());

    // Commits since spawn.
    if let (Some(pre), Some(cur)) = (pre_head, &current_head)
        && pre != cur
        && let Ok(o) = tokio::process::Command::new("git")
            .args(["log", "--oneline", &format!("{pre}..{cur}")])
            .current_dir(workspace)
            .output()
            .await
    {
        let log = String::from_utf8_lossy(&o.stdout).trim().to_owned();
        if !log.is_empty() {
            parts.push(format!("commits:\n{log}"));
        }
    }

    // Working tree status.
    if let Ok(o) = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workspace)
        .output()
        .await
    {
        let status = String::from_utf8_lossy(&o.stdout).trim().to_owned();
        if !status.is_empty() {
            parts.push(format!("working tree:\n{status}"));
        }
    }

    // Diff stat.
    if let Ok(o) = tokio::process::Command::new("git")
        .args(["diff", "--stat"])
        .current_dir(workspace)
        .output()
        .await
    {
        let stat = String::from_utf8_lossy(&o.stdout).trim().to_owned();
        if !stat.is_empty() {
            parts.push(stat);
        }
    }

    if parts.is_empty() {
        "(no changes)".to_owned()
    } else {
        parts.join("\n")
    }
}

fn build_completion_message(
    agent_id: &str,
    cli_name: &str,
    exit_code: Option<i32>,
    output: &str,
    artifacts: &str,
) -> String {
    let status = match exit_code {
        Some(0) => "success (exit 0)".to_owned(),
        Some(code) => format!("failed (exit {code})"),
        None => "running (no exit code yet)".to_owned(),
    };

    let output_section = if output.trim().is_empty() {
        "(sub-agent produced no output)".to_owned()
    } else if output.len() > SUBAGENT_OUTPUT_TAIL {
        let tail_start = output.len() - SUBAGENT_OUTPUT_TAIL;
        let boundary = output.ceil_char_boundary(tail_start);
        format!("...(truncated)\n{}", &output[boundary..])
    } else {
        output.to_owned()
    };

    format!(
        "[subagent {agent_id} completed ({cli_name})]\n\n\
         status: {status}\n\n\
         output:\n{output_section}\n\n\
         changes:\n{artifacts}"
    )
}

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
        return sanitize_path(&path);
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
    let joined = PathBuf::from(workspace).join(&path);
    sanitize_path(&joined)
}

/// Reject paths containing `..` components after normalization to prevent
/// directory traversal attacks (e.g. `../../etc/passwd`).
fn sanitize_path(path: &Path) -> Result<PathBuf, ConduitError> {
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(ConduitError::new(
                ErrorKind::InvalidInput,
                format!(
                    "path '{}' contains '..' traversal and is not allowed. \
                     Use absolute path or workspace-relative path without '..'.",
                    path.display()
                ),
            ));
        }
    }
    Ok(path.to_path_buf())
}

fn read_err(error: impl std::fmt::Display) -> ConduitError {
    ConduitError::new(ErrorKind::Tool, format!("read failed: {error}"))
}

fn write_err(error: impl std::fmt::Display) -> ConduitError {
    ConduitError::new(ErrorKind::Tool, format!("write failed: {error}"))
}

fn resolve_tool_path(ctx: Option<ToolContext>, raw_path: &str) -> Result<PathBuf, ConduitError> {
    resolve_path(&ctx.map(|c| c.state).unwrap_or_default(), raw_path)
}

fn open_text_reader(path: &Path) -> Result<BufReader<std::fs::File>, ConduitError> {
    std::fs::File::open(path)
        .map(BufReader::new)
        .map_err(read_err)
}

fn read_next_line(reader: &mut impl BufRead, line: &mut String) -> Result<bool, ConduitError> {
    line.clear();
    reader
        .read_line(line)
        .map(|count| count > 0)
        .map_err(read_err)
}

fn line_limit_reached(index: usize, offset: usize, limit: Option<usize>) -> bool {
    limit.is_some_and(|limit| index >= offset.saturating_add(limit))
}

fn read_text_window(
    path: &Path,
    offset: usize,
    limit: Option<usize>,
) -> Result<(String, bool), ConduitError> {
    use std::fmt::Write;
    let mut line = String::new();
    let mut index = 0;
    let mut output = String::new();
    let mut reader = open_text_reader(path)?;
    while !line_limit_reached(index, offset, limit) && read_next_line(&mut reader, &mut line)? {
        if index >= offset {
            let _ = write!(output, "{:>6}\t{}", index + 1, &line);
        }
        index += 1;
    }
    // Check if there are more lines beyond the window.
    let truncated = read_next_line(&mut reader, &mut line)?;
    Ok((output, truncated))
}

fn create_parent_dir(path: &Path) -> Result<(), ConduitError> {
    path.parent().map_or(Ok(()), |parent| {
        std::fs::create_dir_all(parent).map_err(write_err)
    })
}

fn existing_permissions(path: &Path) -> Option<std::fs::Permissions> {
    std::fs::metadata(path).ok().map(|meta| meta.permissions())
}

fn apply_permissions(
    path: &Path,
    permissions: Option<std::fs::Permissions>,
) -> Result<(), ConduitError> {
    permissions.map_or(Ok(()), |permissions| {
        std::fs::set_permissions(path, permissions).map_err(write_err)
    })
}

struct AtomicTextWriter {
    path: PathBuf,
    permissions: Option<std::fs::Permissions>,
    writer: BufWriter<NamedTempFile>,
}

impl AtomicTextWriter {
    fn new(path: &Path) -> Result<Self, ConduitError> {
        create_parent_dir(path)?;
        let temp = tempfile::Builder::new()
            .prefix(".eli.")
            .tempfile_in(path.parent().unwrap_or_else(|| Path::new(".")))
            .map_err(write_err)?;
        Ok(Self {
            path: path.to_path_buf(),
            permissions: existing_permissions(path),
            writer: BufWriter::new(temp),
        })
    }

    fn write_str(&mut self, text: &str) -> Result<(), ConduitError> {
        self.writer.write_all(text.as_bytes()).map_err(write_err)
    }

    fn copy_from(&mut self, reader: &mut impl std::io::Read) -> Result<(), ConduitError> {
        std::io::copy(reader, &mut self.writer)
            .map(|_| ())
            .map_err(write_err)
    }

    fn persist(self) -> Result<(), ConduitError> {
        let mut temp = self.writer.into_inner().map_err(|e| write_err(e.error()))?;
        apply_permissions(temp.path(), self.permissions)?;
        temp.as_file_mut().sync_all().map_err(write_err)?;
        temp.persist(&self.path)
            .map(|_| ())
            .map_err(|e| write_err(e.error))
    }
}

fn write_text_file(path: &Path, content: &str) -> Result<(), ConduitError> {
    let mut writer = AtomicTextWriter::new(path)?;
    writer.write_str(content)?;
    writer.persist()
}

fn invalid_edit(path: &Path, old: &str, start: usize) -> ConduitError {
    let preview: String = old.chars().take(80).collect();
    let ellipsis = if old.len() > 80 { "..." } else { "" };
    ConduitError::new(
        ErrorKind::InvalidInput,
        format!(
            "'{preview}{ellipsis}' not found in {} from line {start}. \
             Read the file with fs.read first, then copy exact text into 'old'.",
            path.display()
        ),
    )
}

fn non_empty_old(old: &str) -> Result<(), ConduitError> {
    (!old.is_empty())
        .then_some(())
        .ok_or_else(|| ConduitError::new(ErrorKind::InvalidInput, "'old' must not be empty"))
}

fn flushable_prefix_len(text: &str, keep: usize) -> usize {
    let mut split = text.len().saturating_sub(keep);
    while split > 0 && !text.is_char_boundary(split) {
        split -= 1;
    }
    split
}

fn flush_pending(
    writer: &mut AtomicTextWriter,
    pending: &mut String,
    keep: usize,
) -> Result<(), ConduitError> {
    let split = flushable_prefix_len(pending, keep);
    if split == 0 {
        return Ok(());
    }
    writer.write_str(&pending[..split])?;
    pending.drain(..split);
    Ok(())
}

fn write_replacement(
    writer: &mut AtomicTextWriter,
    pending: &str,
    split: usize,
    old: &str,
    new: &str,
) -> Result<(), ConduitError> {
    writer.write_str(&pending[..split])?;
    writer.write_str(new)?;
    writer.write_str(&pending[split + old.len()..])
}

fn copy_prefix_lines(
    reader: &mut impl BufRead,
    writer: &mut AtomicTextWriter,
    start: usize,
) -> Result<(), ConduitError> {
    let mut line = String::new();
    for _ in 0..start {
        if !read_next_line(reader, &mut line)? {
            break;
        }
        writer.write_str(&line)?;
    }
    Ok(())
}

fn replace_stream(
    reader: &mut impl BufRead,
    writer: &mut AtomicTextWriter,
    old: &str,
    new: &str,
) -> Result<bool, ConduitError> {
    let mut line = String::new();
    let mut pending = String::new();
    while read_next_line(reader, &mut line)? {
        pending.push_str(&line);
        if let Some(split) = pending.find(old) {
            write_replacement(writer, &pending, split, old, new)?;
            writer.copy_from(reader)?;
            return Ok(true);
        }
        flush_pending(writer, &mut pending, old.len().saturating_sub(1))?;
    }
    writer.write_str(&pending)?;
    Ok(false)
}

fn edit_text_file(path: &Path, old: &str, new: &str, start: usize) -> Result<(), ConduitError> {
    non_empty_old(old)?;
    let mut reader = open_text_reader(path)?;
    let mut writer = AtomicTextWriter::new(path)?;
    copy_prefix_lines(&mut reader, &mut writer, start)?;
    if replace_stream(&mut reader, &mut writer, old, new)? {
        writer.persist()
    } else {
        Err(invalid_edit(path, old, start))
    }
}

fn invalid_input(error: anyhow::Error) -> ConduitError {
    ConduitError::new(ErrorKind::InvalidInput, error.to_string())
}

/// Build a short, human-readable notice from the tool name and its arguments.
///
/// Examples: "读 src/main.rs", "执行 cargo build", "搜索 tape: error",
/// "写 config.toml", "编辑 lib.rs", "获取 https://…"
fn auto_notice(tool_name: &str, args: &Value) -> String {
    let primary = |key: &str| args.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let shorten = |s: &str, max: usize| -> String {
        if s.len() <= max {
            s.to_owned()
        } else {
            let end = s.floor_char_boundary(max);
            format!("{}…", &s[..end])
        }
    };
    match tool_name {
        "bash" => {
            let cmd = primary("cmd");
            let desc = primary("description");
            if !desc.is_empty() {
                shorten(desc, 60)
            } else {
                format!("执行 {}", shorten(cmd, 50))
            }
        }
        "fs.read" => format!("读 {}", shorten(primary("path"), 60)),
        "fs.write" => format!("写 {}", shorten(primary("path"), 60)),
        "fs.edit" => format!("编辑 {}", shorten(primary("path"), 60)),
        "web.fetch" => format!("获取 {}", shorten(primary("url"), 60)),
        "tape.search" => format!("搜索 tape: {}", shorten(primary("query"), 40)),
        "tape.info" => "查看 tape 信息".to_owned(),
        "tape.reset" => "重置 tape".to_owned(),
        "tape.handoff" => {
            let name = primary("name");
            if name.is_empty() {
                "创建 handoff".to_owned()
            } else {
                format!("handoff: {name}")
            }
        }
        "tape.anchors" => "列出 anchors".to_owned(),
        "subagent" => format!("子任务: {}", shorten(primary("prompt"), 50)),
        _ => tool_name.to_owned(),
    }
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

async fn maybe_send_user_facing_notice(tool_name: &str, ctx: Option<&ToolContext>, args: &Value) {
    if let Some((notice, session_id, url)) = extract_notice_params(tool_name, ctx, args) {
        send_notice(&url, &session_id, &notice).await;
    }
}

fn extract_notice_params(
    tool_name: &str,
    ctx: Option<&ToolContext>,
    args: &Value,
) -> Option<(String, String, String)> {
    if !crate::builtin::config::EliConfig::load().tool_notices {
        return None;
    }
    let ctx = ctx?;
    let output_channel = ctx.state.get("output_channel").and_then(|v| v.as_str())?;
    if output_channel != "webhook" {
        return None;
    }
    let session_id = ctx
        .state
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())?
        .to_owned();
    let url = crate::tools::SIDECAR_URL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()?;
    let notice = auto_notice(tool_name, args);
    Some((notice, session_id, url))
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
        "Run a shell command and return its output.\n\n\
         Prefer fs.read/fs.write/fs.edit for file I/O — faster and more token-efficient than cat/sed/echo redirects.\n\
         Long-running: set background=true, then poll with bash.output.\n\
         Exceeding timeout_seconds kills the command and returns an error.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "cmd": {"type": "string"},
                "description": {"type": "string", "description": "Brief description of what this command does and why."},
                "cwd": {"type": "string", "description": "Absolute path. Defaults to workspace."},
                "timeout_seconds": {"type": "integer", "description": "Kill the process after N seconds (default 30). Ignored when background=true."},
                "background": {"type": "boolean", "description": "Returns shell_id; poll with bash.output."}
            },
            "required": ["cmd", "description"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice("bash", ctx.as_ref(), &args).await;
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
                    return ok_val(format!(
                        "started: {shell_id} — poll with bash.output(shell_id=\"{shell_id}\"), \
                         stop with bash.kill(shell_id=\"{shell_id}\")"
                    ));
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
                            let body = if body.is_empty() {
                                "(command failed with no output — check if the command exists or try adding 2>&1)"
                            } else {
                                body
                            };
                            return Err(ConduitError::new(
                                ErrorKind::Tool,
                                format!(
                                    "command exited with code {code}\noutput:\n{body}\n\n\
                                     [Tip: read the error above to diagnose.]"
                                ),
                            ));
                        }
                        let trimmed = output.trim();
                        ok_val(if trimmed.is_empty() {
                            "(command succeeded, no output)"
                        } else {
                            trimmed
                        })
                    }
                    Ok(Err(e)) => Err(ConduitError::new(ErrorKind::Tool, format!("{e}"))),
                    Err(_) => {
                        let _ = mgr.terminate(&shell_id).await;
                        Err(ConduitError::new(
                            ErrorKind::Tool,
                            format!(
                                "command timed out after {timeout_secs}s and was killed. \
                                 Increase timeout_seconds, use background=true, \
                                 or simplify the command."
                            ),
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
                "shell_id": {"type": "string"},
                "offset": {"type": "integer", "description": "Resume from next_offset of previous call."},
                "limit": {"type": "integer"}
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
                    if returncode.is_some() {
                        "(process exited, no output)"
                    } else {
                        "(no new output since this offset)"
                    }
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
                "shell_id": {"type": "string"}
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

struct FsReadRequest {
    raw_path: String,
    offset: usize,
    limit: Option<usize>,
}

struct FsWriteRequest {
    raw_path: String,
    content: String,
}

struct FsEditRequest {
    raw_path: String,
    old: String,
    new: String,
    start: usize,
}

fn fs_read_request(args: &Value) -> Result<FsReadRequest, ConduitError> {
    Ok(FsReadRequest {
        raw_path: args
            .require_str_field("path")
            .map_err(invalid_input)?
            .to_owned(),
        offset: args.get_i64_field("offset").unwrap_or(0).max(0) as usize,
        limit: args
            .get_i64_field("limit")
            .map(|value| value.max(0) as usize),
    })
}

fn fs_write_request(args: &Value) -> Result<FsWriteRequest, ConduitError> {
    Ok(FsWriteRequest {
        raw_path: args
            .require_str_field("path")
            .map_err(invalid_input)?
            .to_owned(),
        content: args
            .require_str_field("content")
            .map_err(invalid_input)?
            .to_owned(),
    })
}

fn fs_edit_request(args: &Value) -> Result<FsEditRequest, ConduitError> {
    Ok(FsEditRequest {
        raw_path: args
            .require_str_field("path")
            .map_err(invalid_input)?
            .to_owned(),
        old: args
            .require_str_field("old")
            .map_err(invalid_input)?
            .to_owned(),
        new: args
            .require_str_field("new")
            .map_err(invalid_input)?
            .to_owned(),
        start: args.get_i64_field("start").unwrap_or(0).max(0) as usize,
    })
}

async fn run_fs_read(args: Value, ctx: Option<ToolContext>) -> ToolResult {
    maybe_send_user_facing_notice("fs.read", ctx.as_ref(), &args).await;
    let request = fs_read_request(&args)?;
    let path = resolve_tool_path(ctx, &request.raw_path)?;
    let effective_limit = request.limit.or(Some(DEFAULT_READ_LINE_LIMIT));
    let (mut text, truncated) = read_text_window(&path, request.offset, effective_limit)?;
    if truncated && request.limit.is_none() {
        let next = request.offset + DEFAULT_READ_LINE_LIMIT;
        text.push_str(&format!(
            "\n[... truncated at {DEFAULT_READ_LINE_LIMIT} lines. \
             Use offset={next} limit={DEFAULT_READ_LINE_LIMIT} to continue.]"
        ));
    }
    ok_val(text)
}

async fn run_fs_write(args: Value, ctx: Option<ToolContext>) -> ToolResult {
    maybe_send_user_facing_notice("fs.write", ctx.as_ref(), &args).await;
    let request = fs_write_request(&args)?;
    let path = resolve_tool_path(ctx, &request.raw_path)?;
    let line_count = request.content.lines().count();
    let byte_count = request.content.len();
    write_text_file(&path, &request.content)?;
    ok_val(format!(
        "wrote: {} ({line_count} lines, {byte_count} bytes)",
        path.display()
    ))
}

/// Best-effort syntax check after an edit.  Returns `Some(errors)` when the
/// checker reports a problem, `None` when the file looks OK or no checker is
/// available for the file type.
fn syntax_check(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    let (cmd, args): (&str, Vec<&str>) = match ext {
        "rs" => ("rustfmt", vec!["--check", "--edition", "2021"]),
        "py" => ("python3", vec!["-m", "py_compile"]),
        "js" | "mjs" => ("node", vec!["--check"]),
        "json" => ("python3", vec!["-m", "json.tool"]),
        _ => return None,
    };
    let output = std::process::Command::new(cmd)
        .args(&args)
        .arg(path)
        .output()
        .ok()?;
    if output.status.success() {
        None
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let combined = if stderr.is_empty() { stdout } else { stderr };
        if combined.is_empty() {
            None
        } else {
            Some(combined)
        }
    }
}

async fn run_fs_edit(args: Value, ctx: Option<ToolContext>) -> ToolResult {
    maybe_send_user_facing_notice("fs.edit", ctx.as_ref(), &args).await;
    let request = fs_edit_request(&args)?;
    let path = resolve_tool_path(ctx, &request.raw_path)?;
    let old_len = request.old.lines().count();
    let new_len = request.new.lines().count();
    edit_text_file(&path, &request.old, &request.new, request.start)?;
    let mut msg = format!(
        "edited: {} ({old_len} lines → {new_len} lines)",
        path.display()
    );
    if let Some(errors) = syntax_check(&path) {
        msg.push_str(&format!(
            "\n\n⚠ syntax check failed:\n{errors}\n\
             Fix the syntax error with another fs.edit call."
        ));
    }
    ok_val(msg)
}

fn tool_fs_read() -> Tool {
    Tool::with_context(
        "fs.read",
        "Read a text file with line numbers (1-based, like `cat -n`).\n\n\
         Default limit: 500 lines. Use offset/limit to paginate large files.\n\
         Line numbers are for reference only — do NOT include them in fs.edit 'old' parameter.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Absolute or workspace-relative."},
                "offset": {"type": "integer", "description": "0-based line number."},
                "limit": {"type": "integer", "description": "Max lines."}
            },
            "required": ["path"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(run_fs_read(args, ctx))
        },
    )
}

// ---------------------------------------------------------------------------
// fs.write
// ---------------------------------------------------------------------------

fn tool_fs_write() -> Tool {
    Tool::with_context(
        "fs.write",
        "Create a new text file or fully overwrite an existing one.\n\nExamples: scaffold a new module, generate a config, write test fixtures, save structured output. Auto-creates parent dirs and writes atomically. For partial changes, use fs.edit.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Absolute or workspace-relative."},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(run_fs_write(args, ctx))
        },
    )
}

// ---------------------------------------------------------------------------
// fs.edit
// ---------------------------------------------------------------------------

fn tool_fs_edit() -> Tool {
    Tool::with_context(
        "fs.edit",
        "Find-and-replace exact text in a file (first match only).\n\n\
         IMPORTANT: fs.read the target range first, then copy the exact file content \
         (without line numbers) into 'old'. Mismatched text is the #1 cause of failures.\n\
         Runs syntax check after edit and warns if errors are detected.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Absolute or workspace-relative."},
                "old": {"type": "string"},
                "new": {"type": "string"},
                "start": {"type": "integer", "description": "0-based line to start search."}
            },
            "required": ["path", "old", "new"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(run_fs_edit(args, ctx))
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
                "name": {"type": "string", "description": "e.g. 'deploy', 'feishu-calendar'."}
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
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice("tape.info", ctx.as_ref(), &args).await;
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
                "query": {"type": "string"},
                "limit": {"type": "integer", "description": "Default 20."},
                "start": {"type": "string", "description": "ISO date."},
                "end": {"type": "string", "description": "ISO date."},
                "kinds": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Default: message, tool_result."
                }
            },
            "required": ["query"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice("tape.search", ctx.as_ref(), &args).await;
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
                "archive": {"type": "boolean", "description": "Default false."}
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice("tape.reset", ctx.as_ref(), &args).await;
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
        "Save a named checkpoint (anchor) to the tape with a summary.\n\n\
         Examples: mark a phase as complete, create a resumption point before switching tasks, record state before handing off to another agent.\n\n\
         Compact instructions — when writing the summary, preserve in priority order:\n\
         1. Architecture decisions (NEVER summarize)\n\
         2. Modified files and their key changes\n\
         3. Current verification status (pass/fail)\n\
         4. Open TODOs and rollback notes\n\
         5. Tool outputs (can delete, keep pass/fail only)",
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Default: handoff."},
                "summary": {"type": "string", "description": "Context for resuming later."}
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice("tape.handoff", ctx.as_ref(), &args).await;
                let name = args.get_str_field("name").unwrap_or("handoff").to_owned();
                let summary = args.get_str_field("summary").unwrap_or("").to_owned();
                let tape_name = tape_name_from_context(ctx.as_ref())?;
                let service = current_tape_service()?;
                // Capture entries since last anchor before creating the new one.
                let info = service.info(&tape_name).await?;
                let captured = info.entries_since_last_anchor;
                let state = if summary.is_empty() {
                    None
                } else {
                    Some(serde_json::json!({"summary": summary}))
                };
                service.handoff(&tape_name, &name, state).await?;
                ok_val(format!(
                    "anchor added: {name} (captured {captured} entries since last anchor)"
                ))
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
                "limit": {"type": "integer", "description": "Default 20."}
            }
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice("tape.anchors", ctx.as_ref(), &args).await;
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
                "text": {"type": "string"}
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
                // Count active decisions after this append.
                let query = TapeQuery::new(&tape_name);
                let entries = service.store().fetch_all(&query).await?;
                let total = nexil::collect_active_decisions(&entries).len();
                tracing::info!(decision = %text, tape = %tape_name, "decision.set");
                ok_val(format!("Decision recorded: {text} ({total} active)"))
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
                "index": {"type": "integer", "description": "1-based, from decision.list."}
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
                let remaining = decisions.len() - 1;
                let meta = serde_json::json!({});
                let tombstone = TapeEntry::decision_revoked(text, meta);
                service.store().append(&tape_name, &tombstone).await?;
                tracing::info!(decision = %text, tape = %tape_name, "decision.remove");
                ok_val(format!("Removed decision: {text} ({remaining} remaining)"))
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
                "url": {"type": "string"},
                "headers": {"type": "object"},
                "timeout": {"type": "integer", "description": "Seconds. Default 10."}
            },
            "required": ["url"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice("web.fetch", ctx.as_ref(), &args).await;
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
                    /// Headers that must not be set by LLM-controlled input to
                    /// prevent request smuggling and protocol-level attacks.
                    const BLOCKED_HEADERS: &[&str] = &[
                        "host",
                        "content-length",
                        "transfer-encoding",
                        "connection",
                        "upgrade",
                        "proxy-authorization",
                        "te",
                    ];
                    for (k, v) in headers {
                        if BLOCKED_HEADERS.contains(&k.to_ascii_lowercase().as_str()) {
                            continue;
                        }
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
                        format!(
                            "HTTP {status} for {url}. \
                             For 404: check URL. For 401/403: set headers."
                        ),
                    ));
                }
                let bytes = response.bytes().await.map_err(|e| {
                    ConduitError::new(ErrorKind::Tool, format!("read body failed: {e}"))
                })?;
                if bytes.len() > MAX_RESPONSE_BYTES {
                    return Err(ConduitError::new(
                        ErrorKind::Tool,
                        format!(
                            "response too large ({} bytes, limit {}). \
                             Try a more specific endpoint, add query params to narrow results, \
                             or use bash with curl piped through head/jq.",
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
        "Spawn an async coding sub-agent that runs in the background via an external CLI (claude, codex, kimi).\n\n\
         Returns immediately with an agent ID. When the sub-agent finishes, its full output and a git diff of any file changes are injected back as an inbound message for you to review.\n\n\
         WHEN TO USE:\n\
         - Task is independent: the subtask doesn't need your current conversation context or intermediate results from other work.\n\
         - Task is well-scoped: you can describe it fully in a single prompt (the sub-agent starts fresh with no shared memory).\n\
         - Parallelizable work: you have 2+ independent coding tasks and want them done concurrently instead of sequentially.\n\
         - Long-running changes: a refactor, migration, or large code generation that would block the main conversation.\n\
         - Cross-repo work: you need changes in a different directory or repository while continuing work here.\n\
         - Research + implement split: spawn a sub-agent to research/explore while you implement something else.\n\
         - Review delegation: ask a sub-agent to review code, run tests, or audit a module while you keep building.\n\n\
         WHEN NOT TO USE:\n\
         - The task depends on results from your current work — do it yourself instead.\n\
         - The task is trivial (< 30 seconds) — overhead of spawning isn't worth it.\n\
         - The task needs interactive back-and-forth with the user — sub-agents can't ask questions.\n\n\
         EXAMPLES:\n\
         - \"Write unit tests for the auth module in crates/eli/src/builtin/auth.rs\"\n\
         - \"Refactor all error handling in crates/nexil/ to use thiserror instead of manual impls\"\n\
         - \"Search the codebase for all uses of unwrap() and assess which ones could panic in production\"\n\
         - \"Add OpenAPI doc comments to every public function in crates/eli/src/builtin/tools.rs\"\n\
         - \"Run cargo clippy --workspace and fix all warnings\"",
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string"},
                "cwd": {"type": "string", "description": "Absolute path. Defaults to workspace."},
                "cli": {"type": "string", "description": "e.g. 'claude', 'codex'. Auto-detected if omitted."}
            },
            "required": ["prompt"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                maybe_send_user_facing_notice("subagent", ctx.as_ref(), &args).await;

                let prompt = args
                    .require_str_field("prompt")
                    .map_err(invalid_input)?
                    .to_owned();
                if prompt.trim().is_empty() {
                    return Err(ConduitError::new(
                        ErrorKind::InvalidInput,
                        "prompt must not be empty",
                    ));
                }

                let cli_arg = args
                    .get_str_field("cli")
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let cli = resolve_cli(cli_arg)?;

                let state = ctx.map(|c| c.state).unwrap_or_default();
                let workspace = args
                    .get_str_field("cwd")
                    .map(|s| s.to_owned())
                    .or_else(|| {
                        state
                            .get("_runtime_workspace")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_owned())
                    })
                    .unwrap_or_else(|| {
                        std::env::current_dir()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| ".".to_owned())
                    });

                // Write prompt to temp file, pipe to CLI via stdin redirection.
                let prompt_tempfile = write_prompt_tempfile(&prompt)?;
                let prompt_path = prompt_tempfile
                    .path()
                    .to_str()
                    .ok_or_else(|| {
                        ConduitError::new(ErrorKind::Tool, "prompt tempfile path not UTF-8")
                    })?
                    .to_owned();

                let pre_head = snapshot_git_head(&workspace);

                let full_cmd = build_cli_command(&cli, &prompt_path);

                let mgr = shell_manager();
                let shell_id = mgr.start(&full_cmd, Some(&workspace)).await.map_err(|e| {
                    ConduitError::new(ErrorKind::Tool, format!("failed to start CLI: {e}"))
                })?;

                let agent_id = shell_id.replace("bash-", "agent-");
                let cli_name = cli.name.clone();

                // Capture everything the monitor task needs.
                let inject_fn = crate::control_plane::inbound_injector();
                let session_id = state
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let chat_id = state
                    .get("chat_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let output_channel = state
                    .get("output_channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let inbound_context = state
                    .get("_inbound_context")
                    .and_then(|v| v.as_object())
                    .cloned()
                    .unwrap_or_default();

                let monitor_agent_id = agent_id.clone();
                let monitor_cli_name = cli_name.clone();
                let monitor_shell_id = shell_id.clone();
                let monitor_workspace = workspace.clone();

                tokio::spawn(async move {
                    // Keep prompt file alive until the CLI has a chance to read it.
                    let _prompt_file = prompt_tempfile;

                    let mgr = shell_manager();
                    let (output, exit_code, _) = mgr
                        .wait_closed(&monitor_shell_id)
                        .await
                        .unwrap_or_else(|e| (e.to_string(), Some(-1), "error".to_owned()));

                    let artifacts =
                        collect_artifacts(&monitor_workspace, pre_head.as_deref()).await;
                    let message = build_completion_message(
                        &monitor_agent_id,
                        &monitor_cli_name,
                        exit_code,
                        &output,
                        &artifacts,
                    );

                    if let Some(inject) = inject_fn {
                        // Start with the original inbound context (carries
                        // source_channel, account_id, channel_target, etc.
                        // needed for outbound routing through the sidecar).
                        let mut ctx = inbound_context;
                        ctx.insert("source".to_owned(), serde_json::json!("subagent"));
                        ctx.insert("agent_id".to_owned(), serde_json::json!(monitor_agent_id));
                        ctx.insert("exit_code".to_owned(), serde_json::json!(exit_code));

                        inject(serde_json::json!({
                            "session_id": session_id,
                            "channel": "subagent",
                            "chat_id": chat_id,
                            "content": message,
                            "output_channel": output_channel,
                            "context": ctx
                        }))
                        .await;
                    } else {
                        tracing::warn!(
                            agent_id = %monitor_agent_id,
                            "subagent completed but no inbound injector set"
                        );
                    }
                });

                ok_val(format!(
                    "{agent_id} spawned ({cli_name}). \
                     Its output and file changes will arrive as an inbound message when it completes — \
                     continue with other work."
                ))
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
                "text": {"type": "string"},
                "media_path": {"type": "string", "description": "Local file path."},
                "media_paths": {"type": "array", "items": {"type": "string"}, "description": "Multiple local file paths."},
                "image_path": {"type": "string", "description": "Deprecated; use media_path."}
            },
            "required": ["text"]
        }),
        |args: Value, ctx: Option<ToolContext>| -> BoxFuture<'static, ToolResult> {
            Box::pin(async move {
                let text = args
                    .require_str_field("text")
                    .map_err(invalid_input)?
                    .to_owned();
                let image_path = args
                    .get("image_path")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.to_owned());
                if text.trim().is_empty() && image_path.is_none() {
                    return ok_val("skipped: empty message");
                }

                let ctx = ctx.ok_or_else(|| {
                    ConduitError::new(ErrorKind::InvalidInput, "no tool context available")
                })?;
                let state = &ctx.state;

                let mut envelope = serde_json::json!({
                    "content": text,
                    "session_id": state.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "channel": state.get("channel").and_then(|v| v.as_str()).unwrap_or(""),
                    "chat_id": state.get("chat_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "output_channel": state.get("output_channel").and_then(|v| v.as_str()).unwrap_or(""),
                });
                if let Some(path) = image_path {
                    let path_obj = std::path::Path::new(&path);
                    if !path_obj.exists() {
                        return Err(ConduitError::new(
                            ErrorKind::InvalidInput,
                            format!("image_path not found: {path}"),
                        ));
                    }
                    let mime = crate::control_plane::mime_from_extension(path_obj);
                    let media_type = crate::control_plane::media_type_from_mime(mime);
                    envelope["outbound_media"] = serde_json::json!([
                        {
                            "path": path,
                            "mime_type": mime,
                            "media_type": media_type,
                        }
                    ]);
                }

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
                    "description": "e.g. feishu_calendar_event."
                },
                "description": {
                    "type": "string",
                },
                "params": {
                    "type": "object",
                    "description": "Tool-specific parameters."
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

                // Validate tool_name to prevent path injection / SSRF.
                if !tool_name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
                {
                    return Err(ConduitError::new(
                        ErrorKind::InvalidInput,
                        format!("invalid sidecar tool name: {tool_name}"),
                    ));
                }
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
    use std::io::BufWriter;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    const LARGE_FILE_BYTES: u64 = 50 * 1024 * 1024;

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

    #[tokio::test]
    async fn test_fs_edit_preserves_crlf_and_trailing_newline() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("note.txt");
        std::fs::write(&path, "first\r\nsecond\r\nthird\r\n").unwrap();
        tool_fs_edit()
            .run(
                json!({"path": path.to_string_lossy(), "old": "second", "new": "2nd"}),
                Some(ToolContext::new("test-run")),
            )
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first\r\n2nd\r\nthird\r\n");
    }

    #[tokio::test]
    async fn test_fs_read_preserves_original_newlines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("note.txt");
        std::fs::write(&path, "first\r\nsecond\r\nthird").unwrap();
        let value = tool_fs_read()
            .run(
                json!({"path": path.to_string_lossy(), "offset": 1, "limit": 1}),
                Some(ToolContext::new("test-run")),
            )
            .await
            .unwrap();
        assert_eq!(value.as_str().unwrap(), "     2\tsecond\r\n");
    }

    #[tokio::test]
    async fn test_fs_edit_streams_large_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("huge.txt");
        let line = "a".repeat(8 * 1024);
        let mut writer = BufWriter::new(std::fs::File::create(&path).unwrap());
        for _ in 0..6_300 {
            writeln!(writer, "{line}").unwrap();
        }
        writeln!(writer, "prefix NEEDLE suffix").unwrap();
        for _ in 0..200 {
            writeln!(writer, "{line}").unwrap();
        }
        writer.flush().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > LARGE_FILE_BYTES);
        tool_fs_edit()
            .run(
                json!({"path": path.to_string_lossy(), "old": "NEEDLE", "new": "updated"}),
                Some(ToolContext::new("test-run")),
            )
            .await
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("prefix updated suffix"));
        assert!(!text.contains("prefix NEEDLE suffix"));
    }

    #[tokio::test]
    async fn test_fs_edit_start_skips_earlier_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("note.txt");
        std::fs::write(&path, "target\nkeep\ntarget\n").unwrap();
        tool_fs_edit()
            .run(
                json!({"path": path.to_string_lossy(), "old": "target", "new": "done", "start": 2}),
                Some(ToolContext::new("test-run")),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "target\nkeep\ndone\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_fs_write_preserves_existing_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("script.sh");
        std::fs::write(&path, "echo hi\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        tool_fs_write()
            .run(
                json!({"path": path.to_string_lossy(), "content": "echo bye\n"}),
                Some(ToolContext::new("test-run")),
            )
            .await
            .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn test_bash_exposes_description_field() {
        let tool = tool_bash();
        assert_eq!(
            tool.parameters["properties"]["description"]["type"],
            json!("string"),
            "bash should expose a description field for command purpose"
        );
    }

    #[test]
    fn test_non_bash_tools_omit_description_field() {
        let tools = [
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
        ];

        for tool in tools {
            assert!(
                tool.parameters["properties"].get("description").is_none(),
                "tool {} should not expose a description field (auto-generated notices instead)",
                tool.name
            );
        }
    }

    #[test]
    fn test_auto_notice_generates_semantic_descriptions() {
        assert_eq!(
            auto_notice("fs.read", &json!({"path": "src/main.rs"})),
            "读 src/main.rs"
        );
        assert_eq!(
            auto_notice("fs.write", &json!({"path": "out.txt"})),
            "写 out.txt"
        );
        assert_eq!(
            auto_notice("fs.edit", &json!({"path": "lib.rs"})),
            "编辑 lib.rs"
        );
        assert_eq!(
            auto_notice("web.fetch", &json!({"url": "https://example.com"})),
            "获取 https://example.com"
        );
        assert_eq!(
            auto_notice(
                "bash",
                &json!({"cmd": "cargo build", "description": "编译项目"})
            ),
            "编译项目"
        );
        assert_eq!(
            auto_notice("bash", &json!({"cmd": "cargo build"})),
            "执行 cargo build"
        );
        assert_eq!(
            auto_notice("tape.search", &json!({"query": "error"})),
            "搜索 tape: error"
        );
        assert_eq!(auto_notice("tape.info", &json!({})), "查看 tape 信息");
        assert_eq!(auto_notice("tape.reset", &json!({})), "重置 tape");
        assert_eq!(auto_notice("tape.anchors", &json!({})), "列出 anchors");
        assert_eq!(
            auto_notice("tape.handoff", &json!({"name": "phase-1"})),
            "handoff: phase-1"
        );
        assert_eq!(auto_notice("tape.handoff", &json!({})), "创建 handoff");
        // Unknown tools fall back to tool name
        assert_eq!(auto_notice("unknown.tool", &json!({})), "unknown.tool");
    }

    // -----------------------------------------------------------------------
    // Subagent helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_cli_command_claude() {
        let cli = CliInfo {
            name: "claude".to_owned(),
            path: "/usr/local/bin/claude".to_owned(),
        };
        let cmd = build_cli_command(&cli, "/tmp/prompt.txt");
        assert_eq!(
            cmd,
            "/usr/local/bin/claude -p --output-format text < /tmp/prompt.txt"
        );
    }

    #[test]
    fn test_build_cli_command_codex() {
        let cli = CliInfo {
            name: "codex".to_owned(),
            path: "/usr/bin/codex".to_owned(),
        };
        let cmd = build_cli_command(&cli, "/tmp/prompt.txt");
        assert_eq!(cmd, "/usr/bin/codex exec < /tmp/prompt.txt");
    }

    #[test]
    fn test_build_cli_command_kimi() {
        let cli = CliInfo {
            name: "kimi".to_owned(),
            path: "/opt/bin/kimi".to_owned(),
        };
        let cmd = build_cli_command(&cli, "/tmp/prompt.txt");
        assert!(cmd.contains("-p"));
        assert!(cmd.contains("--print"));
        assert!(cmd.contains("$(cat /tmp/prompt.txt)"));
    }

    #[test]
    fn test_build_cli_command_path_with_spaces() {
        let cli = CliInfo {
            name: "claude".to_owned(),
            path: "/my path/claude".to_owned(),
        };
        let cmd = build_cli_command(&cli, "/tmp/prompt.txt");
        assert!(cmd.starts_with("'/my path/claude'"));
    }

    #[test]
    fn test_shell_quote_simple() {
        assert_eq!(shell_quote("hello"), "hello");
        assert_eq!(shell_quote("/usr/bin/foo"), "/usr/bin/foo");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn test_shell_quote_special_chars() {
        assert_eq!(shell_quote("has space"), "'has space'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_build_completion_message_success() {
        let msg =
            build_completion_message("agent-abc", "claude", Some(0), "all good", "(no changes)");
        assert!(msg.contains("agent-abc"));
        assert!(msg.contains("claude"));
        assert!(msg.contains("success (exit 0)"));
        assert!(msg.contains("all good"));
        assert!(msg.contains("(no changes)"));
    }

    #[test]
    fn test_build_completion_message_failure() {
        let msg = build_completion_message("agent-def", "codex", Some(1), "error!", "M foo.rs");
        assert!(msg.contains("failed (exit 1)"));
        assert!(msg.contains("error!"));
        assert!(msg.contains("M foo.rs"));
    }

    #[test]
    fn test_build_completion_message_truncates_long_output() {
        let long_output = "x".repeat(5000);
        let msg =
            build_completion_message("agent-trunc", "claude", Some(0), &long_output, "(clean)");
        assert!(msg.contains("(truncated)"));
        // The output section should be at most SUBAGENT_OUTPUT_TAIL chars + overhead
        let output_section = msg.split("output:\n").nth(1).unwrap_or("");
        let output_before_changes = output_section.split("\n\nchanges:").next().unwrap_or("");
        assert!(output_before_changes.len() <= SUBAGENT_OUTPUT_TAIL + 20);
    }

    #[test]
    fn test_build_completion_message_empty_output() {
        let msg = build_completion_message("agent-empty", "claude", Some(0), "", "(clean)");
        assert!(msg.contains("(sub-agent produced no output)"));
    }

    #[test]
    fn test_write_prompt_tempfile() {
        let f = write_prompt_tempfile("hello world").unwrap();
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_collect_artifacts_non_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = collect_artifacts(tmp.path().to_str().unwrap(), None).await;
        assert_eq!(result, "(not a git repo)");
    }

    #[tokio::test]
    async fn test_collect_artifacts_clean_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        // Initialize a git repo with one commit.
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir)
            .output()
            .unwrap();
        let head = snapshot_git_head(dir);
        let result = collect_artifacts(dir, head.as_deref()).await;
        assert_eq!(result, "(no changes)");
    }

    // -----------------------------------------------------------------------
    // Tool polish: new coverage
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_fs_read_line_numbers_format() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("lines.txt");
        std::fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();
        let value = tool_fs_read()
            .run(
                json!({"path": path.to_string_lossy()}),
                Some(ToolContext::new("test-run")),
            )
            .await
            .unwrap();
        let text = value.as_str().unwrap();
        assert!(text.starts_with("     1\talpha\n"));
        assert!(text.contains("     2\tbeta\n"));
        assert!(text.contains("     3\tgamma\n"));
    }

    #[tokio::test]
    async fn test_fs_read_default_limit_truncates() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.txt");
        let content: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, &content).unwrap();
        let value = tool_fs_read()
            .run(
                json!({"path": path.to_string_lossy()}),
                Some(ToolContext::new("test-run")),
            )
            .await
            .unwrap();
        let text = value.as_str().unwrap();
        assert!(text.contains("truncated at 500 lines"));
        assert!(text.contains("offset=500"));
        assert!(text.contains("   500\t"));
        assert!(!text.contains("   501\t"));
    }

    #[tokio::test]
    async fn test_fs_read_explicit_limit_no_truncation_note() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.txt");
        let content: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, &content).unwrap();
        let value = tool_fs_read()
            .run(
                json!({"path": path.to_string_lossy(), "limit": 10}),
                Some(ToolContext::new("test-run")),
            )
            .await
            .unwrap();
        let text = value.as_str().unwrap();
        assert!(!text.contains("truncated"));
    }

    #[tokio::test]
    async fn test_fs_edit_invalid_old_shows_hint() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("note.txt");
        std::fs::write(&path, "hello world").unwrap();
        let result = tool_fs_edit()
            .run(
                json!({"path": path.to_string_lossy(), "old": "not here", "new": "x"}),
                Some(ToolContext::new("test-run")),
            )
            .await;
        let err = result.unwrap_err();
        assert!(
            err.message.contains("fs.read"),
            "error should suggest fs.read: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn test_fs_edit_syntax_check_warns_on_bad_python() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.py");
        std::fs::write(&path, "def foo():\n    pass\n").unwrap();
        let value = tool_fs_edit()
            .run(
                json!({
                    "path": path.to_string_lossy().as_ref(),
                    "old": "def foo():\n    pass",
                    "new": "def foo(\n    pass"
                }),
                Some(ToolContext::new("test-run")),
            )
            .await
            .unwrap();
        let text = value.as_str().unwrap();
        assert!(
            text.contains("syntax check failed"),
            "should warn about syntax: {text}"
        );
    }

    #[tokio::test]
    async fn test_fs_edit_syntax_check_silent_on_valid_python() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("good.py");
        std::fs::write(&path, "x = 1\n").unwrap();
        let value = tool_fs_edit()
            .run(
                json!({"path": path.to_string_lossy().as_ref(), "old": "x = 1", "new": "x = 2"}),
                Some(ToolContext::new("test-run")),
            )
            .await
            .unwrap();
        let text = value.as_str().unwrap();
        assert!(
            !text.contains("syntax check"),
            "valid edit should not warn: {text}"
        );
    }

    #[test]
    fn test_invalid_edit_truncates_long_old_text() {
        let long_old = "a".repeat(200);
        let err = invalid_edit(Path::new("test.rs"), &long_old, 0);
        assert!(
            err.message.contains("..."),
            "long old text should be truncated: {}",
            err.message
        );
        assert!(
            err.message.contains("fs.read"),
            "should suggest fs.read: {}",
            err.message
        );
    }
}
