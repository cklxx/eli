//! Agent execution loop and command dispatch.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use chrono::Utc;
use conduit::core::results::ToolAutoResultKind;
use conduit::{AnchorSelector, ConduitError, ErrorKind, TapeContext, TapeEntry, ToolAutoResult};
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

/// Parse arg tokens into a JSON object. Keyword args (`key=value`) become
/// object fields; bare positional args are joined under a `"value"` key.
fn parse_args_to_json(tokens: &[String]) -> Value {
    let mut map = serde_json::Map::new();
    let mut positional: Vec<&str> = Vec::new();
    let mut seen_kwarg = false;

    for token in tokens {
        if let Some(eq_pos) = token.find('=') {
            map.insert(
                token[..eq_pos].to_owned(),
                Value::String(token[eq_pos + 1..].to_owned()),
            );
            seen_kwarg = true;
        } else if seen_kwarg {
            tracing::warn!("positional argument '{}' after keyword arguments", token);
        } else {
            positional.push(token);
        }
    }

    if !positional.is_empty() && map.is_empty() {
        map.insert("value".to_owned(), Value::String(positional.join(" ")));
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
    let result = with_tape_runtime(tapes.clone(), async {
        let tool = lookup_registered_tool(&name);
        if let Some(tool) = tool {
            let json_args = parse_args_to_json(&arg_tokens);
            let ctx = build_tool_context("run_command", tape_name, tool_state);
            if tool.context {
                tool.run(json_args, Some(ctx)).await
            } else {
                tool.run(json_args, None).await
            }
        } else {
            let ctx = build_tool_context("run_command", tape_name, tool_state);
            let bash_args = serde_json::json!({"cmd": body});
            let bash_tool = lookup_registered_tool("bash");
            if let Some(bash_tool) = bash_tool {
                bash_tool.run(bash_args, Some(ctx)).await
            } else {
                Err(ConduitError::new(ErrorKind::Tool, "bash tool not found"))
            }
        }
    })
    .await;

    let output = match result {
        Ok(val) => match val {
            Value::String(s) => s,
            other => serde_json::to_string(&other).unwrap_or_default(),
        },
        Err(e) => {
            let error_output = e.message.clone();
            let elapsed_ms = start.elapsed().as_millis() as i64;
            let event = serde_json::json!({
                "raw": body,
                "name": name,
                "status": "error",
                "elapsed_ms": elapsed_ms,
                "output": error_output,
                "date": Utc::now().to_rfc3339(),
            });
            let _ = tapes.append_event(tape_name, "command", event).await;
            return Err(e);
        }
    };

    let elapsed_ms = start.elapsed().as_millis() as i64;
    let event = serde_json::json!({
        "raw": body,
        "name": name,
        "status": "ok",
        "elapsed_ms": elapsed_ms,
        "output": output,
        "date": Utc::now().to_rfc3339(),
    });
    let _ = tapes.append_event(tape_name, "command", event).await;

    Ok(output)
}

// ---------------------------------------------------------------------------
// Agent loop helpers
// ---------------------------------------------------------------------------

/// Log active decisions from the tape on session resume.
async fn log_active_decisions(tapes: &TapeService, tape_name: &str) {
    if let Ok(all_entries) = tapes
        .store()
        .fetch_all(&conduit::TapeQuery::new(tape_name))
        .await
    {
        let decisions = conduit::collect_active_decisions(&all_entries);
        if !decisions.is_empty() {
            tracing::info!("Resuming session. Active decisions:");
            for (i, d) in decisions.iter().enumerate() {
                tracing::info!("  {}. {}", i + 1, d);
            }
        }
    }
}

/// If an auto-handoff anchor was placed recently, return a TapeContext override
/// that keeps using the previous anchor during the grace period.
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

/// Handle auto-handoff: decrement grace period or place a new handoff anchor
/// when context approaches the limit.
async fn maybe_auto_handoff(
    tapes: &TapeService,
    tape_name: &str,
    output: &ToolAutoResult,
    response_text: &str,
    settings: &AgentSettings,
) {
    // Decrement existing grace period if active.
    if let Ok(Some((remaining, prev_anchor))) = tapes.auto_handoff_grace(tape_name).await
        && remaining > 0
    {
        let new_remaining = remaining - 1;
        let _ = tapes
            .append_event(
                tape_name,
                "auto-handoff.grace",
                serde_json::json!({
                    "remaining": new_remaining,
                    "prev_anchor": prev_anchor,
                }),
            )
            .await;
        if new_remaining == 0 {
            tracing::info!(
                tape = tape_name,
                "auto-handoff grace ended, context will be trimmed next turn"
            );
        }
        return;
    }

    // Check whether context is approaching the limit.
    if output.usage.is_empty() {
        return;
    }
    let input_tokens = output.usage.last().map(|u| u.input_tokens).unwrap_or(0) as usize;
    let threshold = settings.context_window * 70 / 100;
    if input_tokens < threshold {
        return;
    }

    // Place handoff anchor.
    let prev_anchor_name = tapes
        .last_anchor_name(tape_name)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let summary: String = response_text.chars().take(500).collect();
    let anchor_state = serde_json::json!({
        "reason": "auto-handoff: context approaching limit",
        "input_tokens": input_tokens,
        "context_window": settings.context_window,
        "summary": summary,
    });
    let anchor_name = format!("auto-handoff/{}", Utc::now().format("%Y%m%dT%H%M%S"));
    if let Err(e) = tapes
        .handoff(tape_name, &anchor_name, Some(anchor_state))
        .await
    {
        tracing::warn!(error = %e, "auto-handoff: failed to write anchor");
    }

    let sys_entry = TapeEntry::system(
        &format!("[Context summary from auto-handoff]\n{summary}"),
        Value::Object(Default::default()),
    );
    if let Err(e) = tapes.store().append(tape_name, &sys_entry).await {
        tracing::warn!(error = %e, "auto-handoff: failed to write summary");
    }

    if let Err(e) = tapes
        .append_event(
            tape_name,
            "auto-handoff.grace",
            serde_json::json!({
                "remaining": 2,
                "prev_anchor": prev_anchor_name,
            }),
        )
        .await
    {
        tracing::warn!(error = %e, "auto-handoff: failed to write grace event");
    }

    tracing::info!(
        tape = tape_name,
        input_tokens,
        context_window = settings.context_window,
        "auto-handoff: anchor placed, grace period 2"
    );
}

/// Record a run event to the tape with standardised fields.
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

    let step_event = serde_json::json!({"prompt": prompt_text});
    let _ = tapes
        .append_event(tape_name, "agent.run.start", step_event)
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
            None,
        ),
    )
    .await;

    let elapsed_ms = start.elapsed().as_millis() as i64;

    match result {
        Err(e) => {
            record_run_event(tapes, tape_name, elapsed_ms, "error", Some(&e.message)).await;
            Err(e)
        }
        Ok(ref output) => match output.kind {
            ToolAutoResultKind::Text => {
                let text = output.text.clone().unwrap_or_default();
                record_run_event(tapes, tape_name, elapsed_ms, "ok", None).await;
                maybe_auto_handoff(tapes, tape_name, output, &text, settings).await;
                Ok(text)
            }
            _ => {
                let error_msg = match &output.error {
                    Some(e) => format!("{}: {}", e.kind.as_str(), e.message),
                    None => "tool_auto_error: unknown".to_owned(),
                };
                record_run_event(tapes, tape_name, elapsed_ms, "error", Some(&error_msg)).await;
                Err(ConduitError::new(ErrorKind::Unknown, error_msg))
            }
        },
    }
}
