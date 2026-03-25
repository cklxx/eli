//! Agent execution loop and command dispatch.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use chrono::Utc;
use nexil::core::results::ToolAutoResultKind;
use nexil::{AnchorSelector, ConduitError, ErrorKind, TapeContext, TapeEntry, ToolAutoResult};
use serde_json::Value;

use crate::builtin::settings::AgentSettings;
use crate::builtin::tape::TapeService;
use crate::builtin::tools::with_tape_runtime;
use crate::types::PromptValue;

use super::agent_request::{
    build_system_prompt, build_tool_context, create_llm, lookup_registered_tool, run_tools_once,
};

// ---------------------------------------------------------------------------
// Command parsing (inlined from agent_command)
// ---------------------------------------------------------------------------

fn parse_internal_command(line: &str) -> (String, Vec<String>) {
    let parts: Vec<String> = shell_words::split(line)
        .unwrap_or_else(|_| line.split_whitespace().map(|s| s.to_owned()).collect());
    if parts.is_empty() {
        return (String::new(), Vec::new());
    }
    let name = parts[0].clone();
    let rest = parts[1..].to_vec();
    (name, rest)
}

fn parse_args_to_json(tokens: &[String]) -> Value {
    let kwargs: serde_json::Map<String, Value> = tokens
        .iter()
        .filter_map(|t| {
            t.find('=')
                .map(|pos| (t[..pos].to_owned(), Value::String(t[pos + 1..].to_owned())))
        })
        .collect();

    if !kwargs.is_empty() {
        tokens
            .iter()
            .filter(|t| !t.contains('='))
            .for_each(|t| tracing::warn!("positional argument '{t}' after keyword arguments"));
        return Value::Object(kwargs);
    }

    let joined: String = tokens
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let mut map = serde_json::Map::new();
    if !joined.is_empty() {
        map.insert("value".to_owned(), Value::String(joined));
    }
    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Internal command execution
// ---------------------------------------------------------------------------

pub(super) async fn run_command(
    tapes: &TapeService,
    tape_name: &str,
    line: &str,
    tool_state: &HashMap<String, Value>,
) -> Result<String, ConduitError> {
    let body = line[1..].trim();
    if body.is_empty() {
        return Err(ConduitError::new(ErrorKind::InvalidInput, "empty command"));
    }

    let (name, arg_tokens) = parse_internal_command(body);
    let start = Instant::now();

    let result = with_tape_runtime(
        tapes.clone(),
        execute_tool_or_bash(&name, &arg_tokens, body, tape_name, tool_state),
    )
    .await;

    let elapsed_ms = start.elapsed().as_millis() as i64;
    record_command_event(tapes, tape_name, body, &name, elapsed_ms, &result).await;

    match result {
        Ok(val) => Ok(value_to_string(val)),
        Err(e) => Err(e),
    }
}

async fn execute_tool_or_bash(
    name: &str,
    arg_tokens: &[String],
    body: &str,
    tape_name: &str,
    tool_state: &HashMap<String, Value>,
) -> Result<Value, ConduitError> {
    let ctx = build_tool_context("run_command", tape_name, tool_state);
    if let Some(tool) = lookup_registered_tool(name) {
        let json_args = parse_args_to_json(arg_tokens);
        let ctx_arg = if tool.context { Some(ctx) } else { None };
        tool.run(json_args, ctx_arg).await
    } else {
        let bash_tool = lookup_registered_tool("bash")
            .ok_or_else(|| ConduitError::new(ErrorKind::Tool, "bash tool not found"))?;
        bash_tool
            .run(serde_json::json!({"cmd": body}), Some(ctx))
            .await
    }
}

fn value_to_string(val: Value) -> String {
    match val {
        Value::String(s) => s,
        other => serde_json::to_string(&other).unwrap_or_default(),
    }
}

async fn record_command_event(
    tapes: &TapeService,
    tape_name: &str,
    body: &str,
    name: &str,
    elapsed_ms: i64,
    result: &Result<Value, ConduitError>,
) {
    let (status, output) = match result {
        Ok(val) => ("ok", value_to_string(val.clone())),
        Err(e) => ("error", e.message.clone()),
    };
    let event = serde_json::json!({
        "raw": body,
        "name": name,
        "status": status,
        "elapsed_ms": elapsed_ms,
        "output": output,
        "date": Utc::now().to_rfc3339(),
    });
    let _ = tapes.append_event(tape_name, "command", event).await;
}

// ---------------------------------------------------------------------------
// Agent loop helpers
// ---------------------------------------------------------------------------

async fn log_active_decisions(tapes: &TapeService, tape_name: &str) {
    if let Ok(all_entries) = tapes
        .store()
        .fetch_all(&nexil::TapeQuery::new(tape_name))
        .await
    {
        let decisions = nexil::collect_active_decisions(&all_entries);
        if !decisions.is_empty() {
            tracing::info!("Resuming session. Active decisions:");
            for (i, d) in decisions.iter().enumerate() {
                tracing::info!("  {}. {}", i + 1, d);
            }
        }
    }
}

async fn resolve_tape_context_override(
    tapes: &TapeService,
    tape_name: &str,
) -> Option<TapeContext> {
    match tapes.auto_handoff_grace(tape_name).await {
        Ok(Some((remaining, ref prev_anchor))) if remaining > 0 && !prev_anchor.is_empty() => {
            tracing::info!(
                tape = tape_name,
                remaining,
                prev_anchor,
                "auto-handoff grace: using prev anchor"
            );
            Some(TapeContext {
                anchor: AnchorSelector::Named(prev_anchor.clone()),
                ..TapeContext::default()
            })
        }
        _ => None,
    }
}

async fn maybe_auto_handoff(
    tapes: &TapeService,
    tape_name: &str,
    output: &ToolAutoResult,
    response_text: &str,
    settings: &AgentSettings,
) {
    if try_decrement_grace(tapes, tape_name).await {
        return;
    }
    if let Some(input_tokens) = should_handoff(output, settings) {
        place_handoff_anchor(tapes, tape_name, response_text, input_tokens, settings).await;
    }
}

async fn try_decrement_grace(tapes: &TapeService, tape_name: &str) -> bool {
    let Ok(Some((remaining, prev_anchor))) = tapes.auto_handoff_grace(tape_name).await else {
        return false;
    };
    if remaining == 0 {
        return false;
    }
    let new_remaining = remaining - 1;
    let _ = tapes
        .append_event(
            tape_name,
            "auto-handoff.grace",
            serde_json::json!({ "remaining": new_remaining, "prev_anchor": prev_anchor }),
        )
        .await;
    if new_remaining == 0 {
        tracing::info!(
            tape = tape_name,
            "auto-handoff grace ended, context will be trimmed next turn"
        );
    }
    true
}

fn should_handoff(output: &ToolAutoResult, settings: &AgentSettings) -> Option<usize> {
    let input_tokens = output.usage.last().map(|u| u.input_tokens).unwrap_or(0) as usize;
    let threshold = settings.context_window * 70 / 100;
    (input_tokens >= threshold).then_some(input_tokens)
}

async fn place_handoff_anchor(
    tapes: &TapeService,
    tape_name: &str,
    response_text: &str,
    input_tokens: usize,
    settings: &AgentSettings,
) {
    let prev_anchor_name = tapes
        .last_anchor_name(tape_name)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let summary: String = response_text.chars().take(500).collect();
    write_handoff_anchor(tapes, tape_name, &summary, input_tokens, settings).await;
    write_handoff_summary(tapes, tape_name, &summary).await;
    write_handoff_grace(tapes, tape_name, &prev_anchor_name).await;

    tracing::info!(
        tape = tape_name,
        input_tokens,
        context_window = settings.context_window,
        "auto-handoff: anchor placed, grace period 2"
    );
}

async fn write_handoff_anchor(
    tapes: &TapeService,
    tape_name: &str,
    summary: &str,
    input_tokens: usize,
    settings: &AgentSettings,
) {
    let anchor_name = format!("auto-handoff/{}", Utc::now().format("%Y%m%dT%H%M%S"));
    let anchor_state = serde_json::json!({
        "reason": "auto-handoff: context approaching limit",
        "input_tokens": input_tokens,
        "context_window": settings.context_window,
        "summary": summary,
    });
    if let Err(e) = tapes
        .handoff(tape_name, &anchor_name, Some(anchor_state))
        .await
    {
        tracing::warn!(error = %e, "auto-handoff: failed to write anchor");
    }
}

async fn write_handoff_summary(tapes: &TapeService, tape_name: &str, summary: &str) {
    let sys_entry = TapeEntry::system(
        &format!("[Context summary from auto-handoff]\n{summary}"),
        Value::Object(Default::default()),
    );
    if let Err(e) = tapes.store().append(tape_name, &sys_entry).await {
        tracing::warn!(error = %e, "auto-handoff: failed to write summary");
    }
}

async fn write_handoff_grace(tapes: &TapeService, tape_name: &str, prev_anchor_name: &str) {
    if let Err(e) = tapes
        .append_event(
            tape_name,
            "auto-handoff.grace",
            serde_json::json!({ "remaining": 2, "prev_anchor": prev_anchor_name }),
        )
        .await
    {
        tracing::warn!(error = %e, "auto-handoff: failed to write grace event");
    }
}

async fn record_run_event(
    tapes: &TapeService,
    tape_name: &str,
    elapsed_ms: i64,
    status: &str,
    error: Option<&str>,
) {
    let mut event = serde_json::json!({
        "elapsed_ms": elapsed_ms,
        "status": status,
        "date": Utc::now().to_rfc3339(),
    });
    if let Some(err) = error {
        event["error"] = Value::String(err.to_owned());
    }
    let _ = tapes.append_event(tape_name, "agent.run", event).await;
}

// ---------------------------------------------------------------------------
// Agent loop
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) async fn agent_loop(
    tapes: &TapeService,
    tape_name: &str,
    initial_prompt: PromptValue,
    settings: &AgentSettings,
    model: Option<&str>,
    state: &HashMap<String, Value>,
    allowed_skills: Option<&HashSet<String>>,
    allowed_tools: Option<&HashSet<String>>,
    tool_state: &HashMap<String, Value>,
    workspace: &Path,
) -> Result<String, ConduitError> {
    let mut llm = create_llm(settings, model, tapes.store().clone())?;
    let prompt_text = initial_prompt.strict_text();
    let system_prompt =
        build_system_prompt(settings, &prompt_text, state, allowed_skills, workspace);
    let display_model = model.unwrap_or(&settings.model);

    let start = Instant::now();
    tracing::info!(tape = tape_name, model = display_model, "agent.run");
    let _ = tapes
        .append_event(
            tape_name,
            "agent.run.start",
            serde_json::json!({"prompt": prompt_text}),
        )
        .await;

    log_active_decisions(tapes, tape_name).await;
    let tape_ctx_override = resolve_tape_context_override(tapes, tape_name).await;

    let result = with_tape_runtime(
        tapes.clone(),
        run_tools_once(
            &mut llm,
            &system_prompt,
            tape_name,
            &initial_prompt,
            tool_state,
            settings,
            allowed_tools,
            tape_ctx_override.as_ref(),
        ),
    )
    .await;

    let elapsed_ms = start.elapsed().as_millis() as i64;
    process_agent_result(tapes, tape_name, result, elapsed_ms, settings).await
}

async fn process_agent_result(
    tapes: &TapeService,
    tape_name: &str,
    result: Result<ToolAutoResult, ConduitError>,
    elapsed_ms: i64,
    settings: &AgentSettings,
) -> Result<String, ConduitError> {
    match result {
        Err(e) => {
            record_run_event(tapes, tape_name, elapsed_ms, "error", Some(&e.message)).await;
            Err(e)
        }
        Ok(ref output) if output.kind == ToolAutoResultKind::Text => {
            let text = output.text.clone().unwrap_or_default();
            record_run_event(tapes, tape_name, elapsed_ms, "ok", None).await;
            maybe_auto_handoff(tapes, tape_name, output, &text, settings).await;
            Ok(text)
        }
        Ok(ref output) => {
            let error_msg = output
                .error
                .as_ref()
                .map(|e| format!("{}: {}", e.kind.as_str(), e.message))
                .unwrap_or_else(|| "tool_auto_error: unknown".to_owned());
            record_run_event(tapes, tape_name, elapsed_ms, "error", Some(&error_msg)).await;
            Err(ConduitError::new(ErrorKind::Unknown, error_msg))
        }
    }
}
