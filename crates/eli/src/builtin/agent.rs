//! Conduit-driven runtime engine to process prompts.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::Utc;
use conduit::core::results::ToolAutoResultKind;
use conduit::llm::LLM;
use conduit::{ConduitError, ErrorKind, TapeEntry, Tool, ToolAutoResult, ToolContext, ToolSet};
use regex::Regex;
use serde_json::Value;

use crate::builtin::settings::{AgentSettings, ApiBaseConfig, ApiKeyConfig};
use crate::builtin::store::{FileTapeStore, ForkTapeStore};
use crate::builtin::tape::TapeService;
use crate::builtin::tools::with_tape_runtime;
use crate::skills::{discover_skills, render_skills_prompt};
use crate::tools::{REGISTRY, model_tools, render_tools_prompt};

/// Default HTTP headers sent with OpenRouter requests.
#[allow(dead_code)]
const DEFAULT_ELI_HEADERS: [(&str, &str); 2] = [
    ("HTTP-Referer", "https://eliagent.github.io/"),
    ("X-Title", "Eli"),
];

/// Regex for skill hints like `$skill_name` in prompts.
fn hint_regex() -> Regex {
    Regex::new(r"\$([A-Za-z0-9_.\-]+)").unwrap()
}

// ---------------------------------------------------------------------------
// Agent
// ---------------------------------------------------------------------------

/// Agent that processes prompts using hooks and tools. Backed by conduit.
pub struct Agent {
    pub settings: AgentSettings,
    tapes: Option<TapeService>,
}

#[allow(clippy::new_without_default)]
impl Agent {
    /// Create a new agent with settings loaded from the environment.
    pub fn new() -> Self {
        Self {
            settings: AgentSettings::from_env(),
            tapes: None,
        }
    }

    /// Lazily initialise and return the tape service.
    pub fn tapes(&mut self) -> &TapeService {
        if self.tapes.is_none() {
            let tapes_dir = self.settings.home.join("tapes");
            let file_store = FileTapeStore::new(tapes_dir.clone());
            let fork_store = ForkTapeStore::from_sync(file_store);
            self.tapes = Some(TapeService::new(tapes_dir, fork_store));
        }
        self.tapes.as_ref().unwrap()
    }

    /// Mutable access to the tape service.
    pub fn tapes_mut(&mut self) -> &mut TapeService {
        if self.tapes.is_none() {
            let tapes_dir = self.settings.home.join("tapes");
            let file_store = FileTapeStore::new(tapes_dir.clone());
            let fork_store = ForkTapeStore::from_sync(file_store);
            self.tapes = Some(TapeService::new(tapes_dir, fork_store));
        }
        self.tapes.as_mut().unwrap()
    }

    /// Run a prompt to completion within a session.
    pub async fn run(
        &mut self,
        session_id: &str,
        prompt: PromptInput,
        state: &HashMap<String, Value>,
        model: Option<&str>,
        allowed_skills: Option<&HashSet<String>>,
        allowed_tools: Option<&HashSet<String>>,
    ) -> Result<String, ConduitError> {
        if prompt.is_empty() {
            return Err(ConduitError::new(ErrorKind::InvalidInput, "empty prompt"));
        }

        let workspace = state
            .get("_runtime_workspace")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let tape_name = TapeService::session_tape_name(session_id, &workspace);
        let _merge_back = !session_id.starts_with("temp/");

        // Fork the tape for this run.
        let settings = self.settings.clone();
        let tapes = self.tapes_mut();
        let tool_state = build_tool_state(state, &settings, allowed_skills, allowed_tools);

        // Ensure bootstrap anchor exists.
        tapes.ensure_bootstrap_anchor(&tape_name).await?;

        // Check for comma-command.
        if let PromptInput::Text(ref text) = prompt {
            let trimmed = text.trim();
            if trimmed.starts_with(',') {
                return run_command(tapes, &tape_name, trimmed, &tool_state).await;
            }
        }

        // Run the agent loop.
        agent_loop(
            tapes,
            &tape_name,
            prompt,
            &settings,
            model,
            state,
            allowed_skills,
            allowed_tools,
            &tool_state,
            &workspace,
        )
        .await
    }

    /// Build the system prompt from hooks, tools, and skills.
    pub fn system_prompt(
        &self,
        prompt_text: &str,
        state: &HashMap<String, Value>,
        allowed_skills: Option<&HashSet<String>>,
    ) -> String {
        let mut blocks: Vec<String> = Vec::new();

        // Default system prompt.
        blocks.push(default_system_prompt().to_owned());

        let workspace = state
            .get("_runtime_workspace")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        // Tools prompt.
        {
            let reg = REGISTRY.lock().unwrap();
            let tools_prompt = render_tools_prompt(reg.values());
            if !tools_prompt.is_empty() {
                blocks.push(tools_prompt);
            }
        }

        // Skills prompt (filesystem — sidecar writes SKILL.md to .agents/skills/).
        let skills = discover_skills(&workspace);
        let filtered_skills: Vec<_> = if let Some(allowed) = allowed_skills {
            skills
                .into_iter()
                .filter(|s| allowed.contains(&s.name.to_lowercase()))
                .collect()
        } else {
            skills
        };
        let hint_re = hint_regex();
        let expanded: HashSet<String> = hint_re
            .captures_iter(prompt_text)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_owned()))
            .collect();
        let skills_prompt = render_skills_prompt(&filtered_skills, &expanded);
        if !skills_prompt.is_empty() {
            blocks.push(skills_prompt);
        }

        blocks.join("\n\n")
    }
}

// ---------------------------------------------------------------------------
// PromptInput
// ---------------------------------------------------------------------------

/// A prompt can be either plain text or multimodal content parts.
#[derive(Debug, Clone)]
pub enum PromptInput {
    Text(String),
    Parts(Vec<Value>),
}

impl PromptInput {
    pub fn is_empty(&self) -> bool {
        match self {
            PromptInput::Text(s) => s.trim().is_empty(),
            PromptInput::Parts(parts) => parts.is_empty(),
        }
    }

    /// Extract text content (for system prompt building).
    pub fn text(&self) -> String {
        match self {
            PromptInput::Text(s) => s.clone(),
            PromptInput::Parts(parts) => parts
                .iter()
                .filter_map(|p| {
                    if p.get("type").and_then(|v| v.as_str()) == Some("text") {
                        p.get("text").and_then(|v| v.as_str()).map(|s| s.to_owned())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

fn build_tool_state(
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

fn build_tool_context(
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

fn lookup_registered_tool(name: &str) -> Option<Tool> {
    let reg = REGISTRY.lock().unwrap();
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

// ---------------------------------------------------------------------------
// Internal command execution
// ---------------------------------------------------------------------------

async fn run_command(
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
// Agent loop
// ---------------------------------------------------------------------------

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
    // Determine the provider from the model string.
    let provider = model_str.split(':').next().unwrap_or("");

    let mut key_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    // Try OpenAI Codex OAuth tokens.
    if provider.is_empty() || provider == "openai" {
        let resolver = conduit::auth::openai_codex::codex_cli_api_key_resolver(None);
        if let Some(token) = resolver("openai") {
            key_map.insert("openai".to_string(), token);
        }
    }

    // Try Anthropic key from ~/.eli/auth.json.
    if (provider.is_empty() || provider == "anthropic")
        && let Some(key) = crate::builtin::config::load_anthropic_api_key()
    {
        key_map.insert("anthropic".to_string(), key);
    }

    // Try GitHub Copilot resolver.
    if provider.is_empty() || provider == "github-copilot" {
        let resolver =
            conduit::auth::github_copilot::github_copilot_oauth_resolver(None, None, None);
        if let Some(token) = resolver("github-copilot") {
            key_map.insert("github-copilot".to_string(), token);
        }
    }

    if key_map.is_empty() {
        None
    } else if key_map.len() == 1 {
        // Single provider — pass as a single key for simplicity.
        let (_, v) = key_map.into_iter().next().unwrap();
        Some((Some(v), None))
    } else {
        Some((None, Some(key_map)))
    }
}

/// Create a `conduit::LLM` instance from agent settings.
fn create_llm(
    settings: &AgentSettings,
    model_override: Option<&str>,
    tape_store: ForkTapeStore,
) -> Result<LLM, ConduitError> {
    let model_str = model_override.unwrap_or(&settings.model);

    // Ensure model is in provider:model format.
    let model_string: String;
    let model_str = if model_str.contains(':') {
        model_str
    } else {
        // Try to get provider from config profile.
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
        ApiKeyConfig::None => {
            // No explicit API key set — try to resolve from stored tokens.
            match resolve_stored_api_key(model_str) {
                Some((key, map)) => (key, map),
                None => (None, None),
            }
        }
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

/// Load system prompt from md files with precedence:
///
/// 1. Built-in default (hardcoded fallback)
/// 2. ~/.eli/PROMPT.md (global user-level override)
/// 3. .agents/PROMPT.md (project-level override)
///
/// Later files override earlier ones (not append).
fn load_system_prompt_base(settings: &AgentSettings, workspace: &Path) -> String {
    // Try project-level first (highest priority)
    let project_prompt = workspace.join(".agents").join("PROMPT.md");
    if project_prompt.is_file()
        && let Ok(content) = std::fs::read_to_string(&project_prompt)
    {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }

    // Try global user-level
    let global_prompt = settings.home.join("PROMPT.md");
    if global_prompt.is_file()
        && let Ok(content) = std::fs::read_to_string(&global_prompt)
    {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }

    // Fall back to built-in default
    default_system_prompt().to_owned()
}

/// Build the system prompt for the agent loop.
fn build_system_prompt(
    settings: &AgentSettings,
    prompt_text: &str,
    _state: &HashMap<String, Value>,
    allowed_skills: Option<&HashSet<String>>,
    workspace: &Path,
) -> String {
    let mut blocks: Vec<String> = Vec::new();

    // Base system prompt (from md files or built-in default).
    blocks.push(load_system_prompt_base(settings, workspace));

    // Tools prompt.
    {
        let reg = REGISTRY.lock().unwrap();
        let tools_prompt = render_tools_prompt(reg.values());
        if !tools_prompt.is_empty() {
            blocks.push(tools_prompt);
        }
    }

    // Skills prompt (filesystem — sidecar writes SKILL.md to .agents/skills/).
    let skills = discover_skills(workspace);
    let filtered_skills: Vec<_> = if let Some(allowed) = allowed_skills {
        skills
            .into_iter()
            .filter(|s| allowed.contains(&s.name.to_lowercase()))
            .collect()
    } else {
        skills
    };
    let hint_re = hint_regex();
    let expanded: HashSet<String> = hint_re
        .captures_iter(prompt_text)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_owned()))
        .collect();
    let skills_prompt = render_skills_prompt(&filtered_skills, &expanded);
    if !skills_prompt.is_empty() {
        blocks.push(skills_prompt);
    }

    blocks.join("\n\n")
}

#[allow(clippy::too_many_arguments)]
async fn agent_loop(
    tapes: &TapeService,
    tape_name: &str,
    initial_prompt: PromptInput,
    settings: &AgentSettings,
    model: Option<&str>,
    state: &HashMap<String, Value>,
    allowed_skills: Option<&HashSet<String>>,
    allowed_tools: Option<&HashSet<String>>,
    tool_state: &HashMap<String, Value>,
    workspace: &Path,
) -> Result<String, ConduitError> {
    let mut llm = create_llm(settings, model, tapes.store().clone())?;
    let prompt_text = initial_prompt.text();
    let system_prompt =
        build_system_prompt(settings, &prompt_text, state, allowed_skills, workspace);
    let display_model = model.unwrap_or(&settings.model);

    let start = Instant::now();
    tracing::info!(tape = tape_name, model = display_model, "agent.run");

    let step_event = serde_json::json!({"prompt": prompt_text});
    let _ = tapes
        .append_event(tape_name, "agent.run.start", step_event)
        .await;

    let result = with_tape_runtime(
        tapes.clone(),
        run_tools_once(
            &mut llm,
            &system_prompt,
            tapes,
            tape_name,
            &initial_prompt,
            tool_state,
            settings,
            allowed_tools,
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
            let _ = tapes.append_event(tape_name, "agent.run", event).await;
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
                    let _ = tapes.append_event(tape_name, "agent.run", event).await;

                    // Auto-handoff when context approaches the budget limit.
                    // MAX_TOTAL_CONTEXT_CHARS in conduit is 400k; trigger at 80%.
                    const AUTO_HANDOFF_THRESHOLD: usize = 320_000;
                    if let Ok(chars) = tapes.context_chars_since_anchor(tape_name).await
                        && chars >= AUTO_HANDOFF_THRESHOLD
                    {
                        let summary = outcome
                            .text
                            .chars()
                            .take(500)
                            .collect::<String>();
                        let state = serde_json::json!({
                            "reason": "auto-handoff: context approaching limit",
                            "context_chars": chars,
                            "summary": summary,
                        });
                        if let Err(e) =
                            tapes.handoff(tape_name, "auto-handoff", Some(state)).await
                        {
                            tracing::warn!(error = %e, "auto-handoff failed");
                        } else {
                            tracing::info!(
                                tape = tape_name,
                                chars = chars,
                                "auto-handoff: context trimmed"
                            );
                        }
                    }

                    Ok(outcome.text)
                }
                _ => {
                    let event = serde_json::json!({
                        "elapsed_ms": elapsed_ms,
                        "status": "error",
                        "error": outcome.error,
                        "date": Utc::now().to_rfc3339(),
                    });
                    let _ = tapes.append_event(tape_name, "agent.run", event).await;
                    Err(ConduitError::new(ErrorKind::Unknown, outcome.error))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Single tool-execution step
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn run_tools_once(
    llm: &mut LLM,
    system_prompt: &str,
    tapes: &TapeService,
    tape_name: &str,
    prompt: &PromptInput,
    tool_state: &HashMap<String, Value>,
    settings: &AgentSettings,
    allowed_tools: Option<&HashSet<String>>,
) -> Result<ToolAutoResult, ConduitError> {
    // Build tools list from registry.
    let tools: Vec<Tool> = {
        let reg = REGISTRY.lock().unwrap();
        if let Some(allowed) = allowed_tools {
            reg.values()
                .filter(|t| allowed.contains(&t.name.to_lowercase()))
                .cloned()
                .collect()
        } else {
            reg.values().cloned().collect()
        }
    };

    // Schemas are model-facing, but runnable tools keep their canonical names.
    let model_tool_list = model_tools(&tools);
    let schemas: Vec<Value> = model_tool_list.iter().map(|t| t.schema()).collect();
    let tool_set = ToolSet {
        schemas,
        runnable: tools,
    };

    // Record the system prompt and user prompt as tape entries.
    // (run_tools no longer writes these — the caller is responsible.)
    let prompt_text = prompt.text();

    let system_entry = TapeEntry::system(system_prompt, Value::Object(Default::default()));
    let _ = tapes.store().append(tape_name, &system_entry).await;

    let user_message = TapeEntry::message(
        serde_json::json!({"role": "user", "content": prompt_text}),
        Value::Object(Default::default()),
    );
    let _ = tapes.store().append(tape_name, &user_message).await;

    // Create tool context for execution.
    let tool_ctx = build_tool_context("agent_loop", tape_name, tool_state);

    // Call run_tools — it handles tape reading/writing internally.
    let result = llm
        .run_tools(
            Some(&prompt_text),
            Some(system_prompt),
            None, // model override (already set on LLM)
            None, // provider override
            None, // messages — run_tools reads from tape itself
            Some(settings.max_tokens as u32),
            &tool_set,
            Some(&tool_ctx),
            Some(tape_name), // tape name for internal read/write
        )
        .await?;

    // No need to manually record anything — run_tools already wrote to tape.

    Ok(result)
}

// ---------------------------------------------------------------------------
// Result resolution
// ---------------------------------------------------------------------------

struct ToolAutoOutcome {
    kind: String,
    text: String,
    error: String,
}

fn resolve_tool_auto_result(output: &ToolAutoResult) -> ToolAutoOutcome {
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
// Command parsing helpers
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

struct Args {
    positional: Vec<String>,
    kwargs: HashMap<String, String>,
}

fn parse_args(tokens: &[String]) -> Args {
    let mut positional: Vec<String> = Vec::new();
    let mut kwargs: HashMap<String, String> = HashMap::new();
    let mut seen_kwarg = false;

    for token in tokens {
        if let Some(eq_pos) = token.find('=') {
            let key = token[..eq_pos].to_owned();
            let value = token[eq_pos + 1..].to_owned();
            kwargs.insert(key, value);
            seen_kwarg = true;
        } else if seen_kwarg {
            // Positional after keyword — skip or error.
            tracing::warn!("positional argument '{}' after keyword arguments", token);
        } else {
            positional.push(token.clone());
        }
    }

    Args { positional, kwargs }
}

fn args_to_json(args: &Args) -> Value {
    let mut map = serde_json::Map::new();
    for (k, v) in &args.kwargs {
        map.insert(k.clone(), Value::String(v.clone()));
    }
    // Positional args can be passed as a special array if needed.
    if !args.positional.is_empty() && map.is_empty() {
        // Single positional arg — heuristic: pass as first parameter.
        map.insert("value".to_owned(), Value::String(args.positional.join(" ")));
    }
    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Default system prompt
// ---------------------------------------------------------------------------

fn default_system_prompt() -> &'static str {
    "You are Eli, a helpful AI coding assistant.\n\
     \n\
     Output quality (priority: Clear > Coherent > Concise > Concrete): \
     Lead with result first, key evidence second, supporting detail only on demand. \
     Avoid emojis unless the user explicitly requests them.\n\
     \n\
     Execution: Always execute first and exhaust safe deterministic attempts before asking questions. \
     If intent is unclear, inspect context first (tape.search, then workspace files). \
     For explicit low-risk read-only asks (view/check/list/inspect files, branches, project state), \
     execute directly with tools and report findings — do not ask for reconfirmation. \
     Ask a question only when requirements are genuinely missing or contradictory after all viable attempts fail. \
     Treat explicit delegation signals (\"you decide\", \"anything works\", \"use your judgment\") \
     as authorization for low-risk reversible actions: choose a sensible default, execute, and report.\n\
     \n\
     Tools: Use tools to accomplish tasks rather than explaining how to do them. \
     When a tool fails, analyze the error and try an alternative approach before reporting failure. \
     Use web_fetch when you have a URL; use other tools for local operations. \
     Use /tmp as the default location for temporary files unless the user specifies another path.\n\
     \n\
     Response: Reply directly with your response text. \
     Your text output will be delivered to the user automatically — the framework handles channel routing. \
     Do NOT attempt to call channel-specific send functions or emit XML tool-call markup in your text output.\n\
     \n\
     Context: When context grows large, prefer concise responses. \
     You may use tape.info to check token usage and tape.handoff to trim older history. \
     Do not repeat information already visible in the conversation."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::store::{FileTapeStore, ForkTapeStore};
    use crate::builtin::tools::register_builtin_tools;
    use conduit::llm::ApiFormat;
    use serde_json::json;

    fn test_tape_service() -> (
        tempfile::TempDir,
        TapeService,
        String,
        HashMap<String, Value>,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let tapes_dir = tmp.path().join("tapes");
        let store = ForkTapeStore::from_sync(FileTapeStore::new(tapes_dir.clone()));
        let service = TapeService::new(tapes_dir, store);
        let tape_name = "workspace__session".to_owned();

        let mut tool_state = HashMap::new();
        tool_state.insert(
            "_runtime_workspace".to_owned(),
            json!(workspace.display().to_string()),
        );

        (tmp, service, tape_name, tool_state)
    }

    fn test_settings(home: &Path) -> AgentSettings {
        AgentSettings {
            home: home.to_path_buf(),
            model: "test-model".into(),
            fallback_models: None,
            api_key: ApiKeyConfig::None,
            api_base: ApiBaseConfig::None,
            api_format: ApiFormat::Auto,
            max_steps: 5,
            max_tokens: 256,
            model_timeout_seconds: None,
            verbose: 0,
        }
    }

    #[tokio::test]
    async fn test_run_command_passes_workspace_state_to_tools() {
        register_builtin_tools();

        let (tmp, service, tape_name, tool_state) = test_tape_service();
        let file_path = tmp.path().join("workspace").join("note.txt");
        std::fs::write(&file_path, "hello from workspace").unwrap();

        let output = run_command(&service, &tape_name, ",fs.read path=note.txt", &tool_state)
            .await
            .unwrap();

        assert_eq!(output, "hello from workspace");
    }

    #[tokio::test]
    async fn test_run_command_binds_tape_runtime_for_tape_tools() {
        register_builtin_tools();

        let (_tmp, service, tape_name, tool_state) = test_tape_service();
        service.ensure_bootstrap_anchor(&tape_name).await.unwrap();

        let output = run_command(&service, &tape_name, ",tape_info", &tool_state)
            .await
            .unwrap();

        assert!(output.contains("name: workspace__session"));
        assert!(output.contains("anchors: 1"));
    }

    #[test]
    fn test_build_system_prompt_ignores_workspace_agents_guidance() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(workspace.join(".agents")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(workspace.join(".agents").join("PROMPT.md"), "base prompt").unwrap();
        std::fs::write(workspace.join("AGENTS.md"), "workspace agents guidance").unwrap();

        let prompt = build_system_prompt(
            &test_settings(&home),
            "hello",
            &HashMap::new(),
            None,
            &workspace,
        );

        assert!(prompt.contains("base prompt"));
        assert!(!prompt.contains("workspace agents guidance"));
    }
}
