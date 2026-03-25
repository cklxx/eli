//! Agent execution loop and command dispatch.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use chrono::Utc;
use conduit::core::results::ToolAutoResultKind;
use conduit::{ConduitError, ErrorKind, TapeContext, TapeEntry, ToolAutoResult};
use serde_json::Value;

use crate::builtin::settings::AgentSettings;
use crate::builtin::tape::TapeService;
use crate::builtin::tools::with_tape_runtime;
use crate::types::PromptValue;

use super::agent_command::{args_to_json, parse_args, parse_internal_command};
use super::agent_request::{
    build_system_prompt, build_tool_context, create_llm, lookup_registered_tool, run_tools_once,
};

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
            let args = parse_args(&arg_tokens);
            let ctx = build_tool_context("run_command", tape_name, tool_state);
            let json_args = args_to_json(&args);
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
// Result resolution
// ---------------------------------------------------------------------------

pub(super) struct ToolAutoOutcome {
    pub kind: String,
    pub text: String,
    pub error: String,
}

pub(super) fn resolve_tool_auto_result(output: &ToolAutoResult) -> ToolAutoOutcome {
    match output.kind {
        ToolAutoResultKind::Text => ToolAutoOutcome {
            kind: "text".to_owned(),
            text: output.text.clone().unwrap_or_default(),
            error: String::new(),
        },
        ToolAutoResultKind::Tools => ToolAutoOutcome {
            kind: "continue".to_owned(),
            text: String::new(),
            error: String::new(),
        },
        ToolAutoResultKind::Error => {
            let error_msg = match &output.error {
                Some(e) => format!("{}: {}", e.kind.as_str(), e.message),
                None => "tool_auto_error: unknown".to_owned(),
            };
            ToolAutoOutcome {
                kind: "error".to_owned(),
                text: String::new(),
                error: error_msg,
            }
        }
    }
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
                anchor: conduit::AnchorSelector::Named(prev_anchor.clone()),
                ..TapeContext::default()
            })
        }
        _ => None,
    }
}

/// Handle auto-handoff state machine after a successful text response.
async fn handle_auto_handoff(
    tapes: &TapeService,
    tape_name: &str,
    output: &ToolAutoResult,
    outcome: &ToolAutoOutcome,
    settings: &AgentSettings,
) {
    if let Ok(Some((remaining, prev_anchor))) = tapes.auto_handoff_grace(tape_name).await {
        if remaining > 0 {
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
        }
    } else if !output.usage.is_empty() {
        let input_tokens = output.usage.last().map(|u| u.input_tokens).unwrap_or(0) as usize;
        let threshold = settings.context_window * 70 / 100;
        if input_tokens >= threshold {
            trigger_auto_handoff(tapes, tape_name, input_tokens, outcome, settings).await;
        }
    }
}

/// Place an auto-handoff anchor when context approaches the limit.
async fn trigger_auto_handoff(
    tapes: &TapeService,
    tape_name: &str,
    input_tokens: usize,
    outcome: &ToolAutoOutcome,
    settings: &AgentSettings,
) {
    let prev_anchor_name = tapes
        .last_anchor_name(tape_name)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let summary: String = outcome.text.chars().take(500).collect();
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
        &format!("[Context summary from auto-handoff]\n{}", summary),
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

/// Record a run event to the tape.
async fn record_run_event(tapes: &TapeService, tape_name: &str, elapsed_ms: i64, event: Value) {
    let _ = tapes.append_event(tape_name, "agent.run", event).await;
    let _ = elapsed_ms; // used by caller for the event payload
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
            let event = serde_json::json!({
                "elapsed_ms": elapsed_ms,
                "status": "error",
                "error": e.message,
                "date": Utc::now().to_rfc3339(),
            });
            record_run_event(tapes, tape_name, elapsed_ms, event).await;
            Err(e)
        }
        Ok(ref output) => {
            let outcome = resolve_tool_auto_result(output);
            match outcome.kind.as_str() {
                "text" => {
                    let event = serde_json::json!({
                        "elapsed_ms": elapsed_ms,
                        "status": "ok",
                        "date": Utc::now().to_rfc3339(),
                    });
                    record_run_event(tapes, tape_name, elapsed_ms, event).await;
                    handle_auto_handoff(tapes, tape_name, output, &outcome, settings).await;
                    Ok(outcome.text)
                }
                _ => {
                    let event = serde_json::json!({
                        "elapsed_ms": elapsed_ms,
                        "status": "error",
                        "error": outcome.error,
                        "date": Utc::now().to_rfc3339(),
                    });
                    record_run_event(tapes, tape_name, elapsed_ms, event).await;
                    Err(ConduitError::new(ErrorKind::Unknown, outcome.error))
                }
            }
        }
    }
}
