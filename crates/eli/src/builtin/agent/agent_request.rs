//! LLM creation and request building for the agent.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use nexil::llm::{ChatRequest, LLM};
use nexil::{ConduitError, TapeContext, Tool, ToolAutoResult, ToolContext, ToolSet};
use serde_json::Value;

use crate::builtin::settings::{AgentSettings, ApiBaseConfig, ApiKeyConfig};
use crate::builtin::store::ForkTapeStore;
use crate::prompt_builder::{PromptBuilder, PromptMode};
use crate::skill_matcher::{MatchContext, SkillMatcher};
use crate::skills::discover_skills;
use crate::tools::{REGISTRY, model_tools};
use crate::types::PromptValue;

pub(super) fn build_tool_state(
    state: &HashMap<String, Value>,
    settings: &AgentSettings,
    allowed_skills: Option<&HashSet<String>>,
    allowed_tools: Option<&HashSet<String>>,
) -> HashMap<String, Value> {
    let mut tool_state = state.clone();
    tool_state.insert(
        "_runtime_tapes_dir".to_owned(),
        Value::String(settings.home.join("tapes").display().to_string()),
    );

    if let Some(allowed) = allowed_skills {
        let mut skills: Vec<String> = allowed.iter().cloned().collect();
        skills.sort();
        tool_state.insert(
            "allowed_skills".to_owned(),
            Value::Array(skills.into_iter().map(Value::String).collect()),
        );
    }

    if let Some(allowed) = allowed_tools {
        let mut tools: Vec<String> = allowed.iter().cloned().collect();
        tools.sort();
        tool_state.insert(
            "allowed_tools".to_owned(),
            Value::Array(tools.into_iter().map(Value::String).collect()),
        );
    }

    tool_state
}

pub(super) fn build_tool_context(
    run_id: &str,
    tape_name: &str,
    tool_state: &HashMap<String, Value>,
) -> ToolContext {
    let mut ctx = ToolContext::new(run_id).with_tape(tape_name.to_owned());
    for (key, value) in tool_state {
        ctx = ctx.with_state(key.clone(), value.clone());
    }
    ctx
}

pub(super) fn lookup_registered_tool(name: &str) -> Option<Tool> {
    let reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    reg.get(name)
        .cloned()
        .or_else(|| {
            if name.contains('_') {
                reg.get(&name.replace('_', ".")).cloned()
            } else {
                None
            }
        })
        .or_else(|| {
            if name.contains('.') {
                reg.get(&name.replace('.', "_")).cloned()
            } else {
                None
            }
        })
}

/// Resolve an API key from stored OAuth tokens / credentials when no explicit key is set.
///
/// Checks (in order):
/// 1. `~/.eli/config.toml` for active provider profile
/// 2. OpenAI Codex OAuth tokens (from `~/.codex/auth.json`)
/// 3. Anthropic API key (from `~/.eli/auth.json`)
/// 4. GitHub Copilot OAuth tokens
#[allow(clippy::type_complexity)]
fn resolve_stored_api_key(
    model_str: &str,
) -> Option<(
    Option<String>,
    Option<std::collections::HashMap<String, String>>,
)> {
    let provider = model_str.split(':').next().unwrap_or("");

    let mut key_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    if provider.is_empty() || provider == "openai" {
        let resolver = nexil::auth::openai_codex::codex_cli_api_key_resolver(None);
        if let Some(token) = resolver("openai") {
            key_map.insert("openai".to_string(), token);
        }
    }

    if (provider.is_empty() || provider == "anthropic")
        && let Some(key) = crate::builtin::config::load_anthropic_api_key()
    {
        key_map.insert("anthropic".to_string(), key);
    }

    if provider.is_empty() || provider == "github-copilot" {
        let resolver = nexil::auth::github_copilot::github_copilot_oauth_resolver(None, None, None);
        if let Some(token) = resolver("github-copilot") {
            key_map.insert("github-copilot".to_string(), token);
        }
    }

    if key_map.is_empty() {
        None
    } else if key_map.len() == 1 {
        let (_, v) = key_map.into_iter().next().unwrap();
        Some((Some(v), None))
    } else {
        Some((None, Some(key_map)))
    }
}

/// Create a `nexil::LLM` instance from agent settings.
pub(super) fn create_llm(
    settings: &AgentSettings,
    model_override: Option<&str>,
    tape_store: ForkTapeStore,
) -> Result<LLM, ConduitError> {
    let model_str = model_override.unwrap_or(&settings.model);

    let model_string: String;
    let model_str = if model_str.contains(':') {
        model_str
    } else {
        let config = crate::builtin::config::EliConfig::load();
        let provider = config
            .resolve_provider()
            .unwrap_or_else(|| "openai".to_string());
        model_string = format!("{provider}:{model_str}");
        &model_string
    };
    let (api_key, api_key_map) = match settings.api_key.clone() {
        ApiKeyConfig::Single(k) => (Some(k), None),
        ApiKeyConfig::PerProvider(m) => (None, Some(m)),
        ApiKeyConfig::None => match resolve_stored_api_key(model_str) {
            Some((key, map)) => (key, map),
            None => (None, None),
        },
    };

    let (api_base, api_base_map) = match settings.api_base.clone() {
        ApiBaseConfig::Single(b) => (Some(b), None),
        ApiBaseConfig::PerProvider(m) => (None, Some(m)),
        ApiBaseConfig::None => (None, None),
    };

    let mut builder = LLM::builder()
        .model(model_str)
        .api_format(settings.api_format)
        .verbose(settings.verbose as u32)
        .tape_store(tape_store);

    if let Some(fallback_models) = settings.fallback_models.clone() {
        builder = builder.fallback_models(fallback_models);
    }
    if let Some(api_key) = api_key {
        builder = builder.api_key(&api_key);
    } else if let Some(api_key_map) = api_key_map {
        builder = builder.api_key_map(api_key_map);
    }
    if let Some(api_base) = api_base {
        builder = builder.api_base(&api_base);
    } else if let Some(api_base_map) = api_base_map {
        builder = builder.api_base_map(api_base_map);
    }

    builder.build()
}

/// Build the system prompt for the agent loop.
///
/// Delegates to [`PromptBuilder`] for sectioned composition with mode support.
/// Uses [`SkillMatcher`] for multi-signal skill auto-activation alongside
/// the existing `$hint` regex expansion.
pub(super) fn build_system_prompt(
    settings: &AgentSettings,
    prompt_text: &str,
    state: &HashMap<String, Value>,
    allowed_skills: Option<&HashSet<String>>,
    workspace: &Path,
) -> String {
    let skills = discover_skills(workspace);
    let matcher = SkillMatcher::new();
    let session_id = state
        .get("_session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let match_ctx = MatchContext {
        task_input: prompt_text,
        recent_tools: &[],
        session_id,
    };
    let auto_expanded = matcher.match_skills(&skills, &match_ctx);

    PromptBuilder::new(PromptMode::Full).build(
        settings,
        prompt_text,
        state,
        allowed_skills,
        &auto_expanded,
        workspace,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_tools_once(
    llm: &mut LLM,
    system_prompt: &str,
    tape_name: &str,
    prompt: &PromptValue,
    tool_state: &HashMap<String, Value>,
    settings: &AgentSettings,
    allowed_tools: Option<&HashSet<String>>,
    tape_context: Option<&TapeContext>,
    wrap_tools_fn: Option<&(dyn Fn(Vec<Tool>) -> Vec<Tool> + Send + Sync)>,
) -> Result<ToolAutoResult, ConduitError> {
    let mut tools: Vec<Tool> = {
        let reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(allowed) = allowed_tools {
            reg.values()
                .filter(|t| allowed.contains(&t.name.to_lowercase()))
                .cloned()
                .collect()
        } else {
            reg.values().cloned().collect()
        }
    };
    if let Some(wrap_fn) = wrap_tools_fn {
        tools = wrap_fn(tools);
    }

    let model_tool_list = model_tools(&tools);
    let schemas: Vec<Value> = model_tool_list.iter().map(|t| t.schema()).collect();
    let tool_set = ToolSet {
        schemas,
        runnable: tools,
    };

    let (prompt_str, user_content) = match prompt {
        PromptValue::Parts(parts) => (None, Some(parts.clone())),
        _ => (Some(prompt.strict_text()), None),
    };
    let prompt_ref = prompt_str.as_deref();

    let tool_ctx = build_tool_context("agent_loop", tape_name, tool_state);

    let result = llm
        .run_tools(ChatRequest {
            prompt: prompt_ref,
            user_content,
            system_prompt: Some(system_prompt),
            max_tokens: Some(settings.max_tokens as u32),
            tools: Some(&tool_set),
            tool_context: Some(&tool_ctx),
            tape: Some(tape_name),
            tape_context,
            ..Default::default()
        })
        .await?;

    Ok(result)
}
