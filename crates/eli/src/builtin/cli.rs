//! CLI commands: run, chat, login, use, status, hooks, gateway.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Subcommand;
use serde_json::Value;

use crate::builtin::BuiltinImpl;
use crate::builtin::config::{
    EliConfig, Profile, default_model_for_provider, load_auth_status, normalize_provider,
};
use crate::framework::EliFramework;

/// CLI subcommands for the `eli` binary.
#[derive(Debug, Subcommand)]
pub enum CliCommand {
    /// Run one inbound message through the framework pipeline.
    Run {
        /// Inbound message content.
        message: String,
        /// Message channel.
        #[arg(long, default_value = "cli")]
        channel: String,
        /// Chat id.
        #[arg(long, default_value = "local")]
        chat_id: String,
        /// Sender id.
        #[arg(long, default_value = "human")]
        sender_id: String,
        /// Optional session id.
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Start a REPL chat session.
    Chat {
        /// Chat id.
        #[arg(long, default_value = "local")]
        chat_id: String,
        /// Optional session id.
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Authenticate with a provider (openai, claude, github-copilot).
    Login {
        /// Authentication provider (openai, claude, github-copilot).
        provider: String,
        /// Directory to store credentials.
        #[arg(long)]
        codex_home: Option<PathBuf>,
        /// Open the OAuth URL in a browser.
        #[arg(long, default_value_t = true)]
        browser: bool,
        /// Paste the callback URL instead of using a local server.
        #[arg(long)]
        manual: bool,
        /// OAuth wait timeout in seconds.
        #[arg(long, default_value_t = 300.0)]
        timeout: f64,
        /// Paste an API key directly instead of using OAuth (for claude/anthropic).
        #[arg(long)]
        api_key: bool,
    },
    /// Switch active provider profile.
    Use {
        /// Profile name (e.g. "openai", "anthropic", "copilot").
        profile: String,
    },
    /// Show authentication and configuration status.
    Status,
    /// Show hook implementation mapping.
    #[command(hide = true)]
    Hooks,
    /// Manage model selection.
    Model {
        /// Model name to switch to, or "list" to show available models.
        /// Omit to show current model.
        name: Option<String>,
    },
    /// Start message listeners (like telegram).
    Gateway {
        /// Channels to enable (default: all).
        #[arg(long = "enable-channel")]
        enable_channels: Vec<String>,
    },
    /// Open the tape viewer web UI.
    Tape {
        /// HTTP port to bind to.
        #[arg(long, default_value_t = 7700)]
        port: u16,
        /// Path to tapes directory (defaults to ~/.eli/tapes).
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
    },
}

/// Execute a CLI command.
pub async fn execute(cmd: CliCommand) -> anyhow::Result<()> {
    match cmd {
        CliCommand::Run {
            message,
            channel,
            chat_id,
            sender_id,
            session_id,
        } => run_command(message, channel, chat_id, sender_id, session_id).await,
        CliCommand::Chat {
            chat_id,
            session_id,
        } => chat_command(chat_id, session_id).await,
        CliCommand::Login {
            provider,
            codex_home,
            browser,
            manual,
            timeout,
            api_key,
        } => login_command(provider, codex_home, browser, manual, timeout, api_key).await,
        CliCommand::Use { profile } => use_command(profile),
        CliCommand::Model { name } => model_command(name).await,
        CliCommand::Status => status_command(),
        CliCommand::Hooks => {
            hooks_command().await;
            Ok(())
        }
        CliCommand::Gateway { enable_channels } => gateway_command(enable_channels).await,
        CliCommand::Tape { port, dir } => tape_command(port, dir).await,
    }
}

/// Run a single message through the agent.
async fn run_command(
    message: String,
    channel: String,
    chat_id: String,
    sender_id: String,
    session_id: Option<String>,
) -> anyhow::Result<()> {
    let session = session_id.unwrap_or_else(|| format!("{channel}:{chat_id}"));
    let framework = builtin_framework().await;
    let inbound = serde_json::json!({
        "session_id": session,
        "channel": channel,
        "chat_id": chat_id,
        "sender_id": sender_id,
        "content": message,
        "output_channel": "cli",
    });

    match framework.process_inbound(inbound).await {
        Ok(result) => {
            println!("[{}]", result.session_id);
            print_cli_outbounds(&result.outbounds);
        }
        Err(e) => {
            eprintln!("Error: {e}");
        }
    }

    Ok(())
}

/// Start an interactive REPL chat session.
async fn chat_command(chat_id: String, session_id: Option<String>) -> anyhow::Result<()> {
    let session = session_id.unwrap_or_else(|| format!("cli:{chat_id}"));
    let framework = builtin_framework().await;

    println!("Eli chat session (session: {session}). Type ,quit to exit.");

    let stdin = tokio::io::stdin();
    let reader = tokio::io::BufReader::new(stdin);
    use tokio::io::AsyncBufReadExt;
    let mut lines = reader.lines();

    loop {
        eprint!("> ");
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => break,
            Err(_) => break,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed == ",quit" || trimmed == "quit" {
            println!("Goodbye.");
            break;
        }

        let inbound = serde_json::json!({
            "session_id": session,
            "channel": "cli",
            "chat_id": chat_id,
            "content": trimmed,
            "output_channel": "cli",
        });

        match framework.process_inbound(inbound).await {
            Ok(result) => print_cli_outbounds(&result.outbounds),
            Err(e) => eprintln!("Error: {e}"),
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

/// Login to a provider.
async fn login_command(
    provider: String,
    codex_home: Option<PathBuf>,
    browser: bool,
    manual: bool,
    timeout: f64,
    api_key_mode: bool,
) -> anyhow::Result<()> {
    match provider.as_str() {
        "openai" => login_openai(codex_home, browser, manual, timeout).await,
        "claude" | "anthropic" => {
            if api_key_mode {
                login_claude_api_key().await
            } else {
                login_claude_oauth(browser).await
            }
        }
        "github-copilot" | "copilot" => login_github_copilot(browser, timeout).await,
        _ => anyhow::bail!(
            "Unsupported auth provider: {provider}\n\
             Supported providers: openai, claude, github-copilot"
        ),
    }
}

/// Save a profile after login and print a summary.
fn post_login_save_profile(provider_raw: &str) -> anyhow::Result<()> {
    let provider = normalize_provider(provider_raw);
    let model = default_model_for_provider(provider);
    let profile_name = provider.to_string();

    let mut config = EliConfig::load();
    let had_active = config.active_profile.is_some();

    config.add_profile(
        &profile_name,
        Profile {
            provider: provider.to_string(),
            model: model.to_string(),
        },
    );

    if !had_active {
        config.active_profile = Some(profile_name.clone());
    }

    config.save()?;

    println!();
    println!("  Profile:  {profile_name}");
    println!("  Provider: {provider}");
    println!("  Model:    {model}");

    if had_active {
        let current = config.active_profile.as_deref().unwrap_or("(none)");
        if current != profile_name {
            println!();
            println!("  Tip: run `eli use {profile_name}` to switch to this profile");
            println!("  (current active profile: {current})");
        }
    } else {
        println!("  Active:   yes (auto-selected as first profile)");
    }

    Ok(())
}

/// OpenAI Codex OAuth login flow.
async fn login_openai(
    codex_home: Option<PathBuf>,
    open_browser: bool,
    _manual: bool,
    timeout: f64,
) -> anyhow::Result<()> {
    use conduit::auth::openai_codex::{
        self, DEFAULT_CODEX_OAUTH_AUTHORIZE_URL, DEFAULT_CODEX_OAUTH_CLIENT_ID,
        DEFAULT_CODEX_OAUTH_ORIGINATOR, DEFAULT_CODEX_OAUTH_SCOPE, DEFAULT_CODEX_OAUTH_TOKEN_URL,
    };

    let home = codex_home.unwrap_or_else(|| {
        std::env::var("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".codex")
            })
    });

    let redirect_uri = "http://localhost:1455/auth/callback";

    println!("Starting OpenAI Codex OAuth login...");
    if !open_browser {
        println!("(browser auto-open disabled; copy the URL from above)");
    }

    let tokens = openai_codex::login_openai_codex_oauth(
        Some(home.as_path()),
        redirect_uri,
        timeout,
        DEFAULT_CODEX_OAUTH_CLIENT_ID,
        DEFAULT_CODEX_OAUTH_AUTHORIZE_URL,
        DEFAULT_CODEX_OAUTH_TOKEN_URL,
        DEFAULT_CODEX_OAUTH_SCOPE,
        DEFAULT_CODEX_OAUTH_ORIGINATOR,
        open_browser,
    )
    .await
    .map_err(|e| anyhow::anyhow!("OpenAI OAuth login failed: {e}"))?;

    let account_info = tokens.account_id.as_deref().unwrap_or("(unknown account)");
    println!("Login successful! Account: {account_info}");
    println!("Tokens saved to: {}", home.join("auth.json").display());

    post_login_save_profile("openai")?;

    Ok(())
}

/// Claude / Anthropic OAuth login — user runs `claude setup-token` separately and pastes the token.
async fn login_claude_oauth(_open_browser: bool) -> anyhow::Result<()> {
    use std::io::{self, Write};

    println!("Claude subscription login");
    println!();
    println!("  1. Run this in another terminal:  claude setup-token");
    println!("  2. Complete the browser login");
    println!("  3. Copy the token (starts with sk-ant-) and paste it below");
    println!();
    print!("Paste your token: ");
    io::stdout().flush()?;

    let mut token = String::new();
    io::stdin().read_line(&mut token)?;
    let token = token.trim().to_string();

    if token.is_empty() {
        anyhow::bail!("Token cannot be empty");
    }

    if !token.starts_with("sk-ant-") {
        anyhow::bail!(
            "Invalid token format (expected sk-ant-...).\n\
             Make sure you copied the full token from `claude setup-token`."
        );
    }

    // Save with long expiry (1 year).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let expires_at = now + 365 * 24 * 3600;

    crate::builtin::config::save_anthropic_oauth_tokens(&token, "", expires_at)?;

    let auth_path = crate::builtin::config::eli_home().join("auth.json");
    println!("Token saved to: {}", auth_path.display());

    post_login_save_profile("anthropic")?;

    Ok(())
}

/// Claude / Anthropic API key login flow (legacy, via --api-key flag).
async fn login_claude_api_key() -> anyhow::Result<()> {
    use std::io::{self, Write};

    println!("Anthropic API key login");
    println!("Get your API key from: https://console.anthropic.com/settings/keys");
    print!("Enter your Anthropic API key: ");
    io::stdout().flush()?;

    let mut api_key = String::new();
    io::stdin().read_line(&mut api_key)?;
    let api_key = api_key.trim().to_string();

    if api_key.is_empty() {
        anyhow::bail!("API key cannot be empty");
    }

    // Save to ~/.eli/auth.json under "anthropic" key.
    let home = crate::builtin::config::eli_home();
    std::fs::create_dir_all(&home)?;

    let auth_path = home.join("auth.json");
    let mut auth_data: serde_json::Map<String, Value> = if auth_path.exists() {
        let contents = std::fs::read_to_string(&auth_path)?;
        serde_json::from_str(&contents).unwrap_or_default()
    } else {
        serde_json::Map::new()
    };

    auth_data.insert(
        "anthropic".to_string(),
        serde_json::json!({ "api_key": api_key }),
    );

    let json_str = serde_json::to_string_pretty(&Value::Object(auth_data))? + "\n";
    std::fs::write(&auth_path, &json_str)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&auth_path, std::fs::Permissions::from_mode(0o600));
    }

    println!("API key saved to: {}", auth_path.display());

    post_login_save_profile("anthropic")?;

    Ok(())
}

/// GitHub Copilot device-flow login.
async fn login_github_copilot(open_browser: bool, timeout: f64) -> anyhow::Result<()> {
    use conduit::auth::github_copilot::{
        self, DEFAULT_GITHUB_COPILOT_ACCESS_TOKEN_URL, DEFAULT_GITHUB_COPILOT_DEVICE_CODE_URL,
        DEFAULT_GITHUB_COPILOT_OAUTH_CLIENT_ID, DEFAULT_GITHUB_COPILOT_SCOPE,
    };

    println!("Starting GitHub Copilot device-flow login...");

    let tokens = github_copilot::login_github_copilot_oauth(
        None, // config_home (uses default)
        open_browser,
        DEFAULT_GITHUB_COPILOT_SCOPE,
        timeout,
        DEFAULT_GITHUB_COPILOT_OAUTH_CLIENT_ID,
        DEFAULT_GITHUB_COPILOT_DEVICE_CODE_URL,
        DEFAULT_GITHUB_COPILOT_ACCESS_TOKEN_URL,
        None, // device_code_notifier (uses default eprintln)
    )
    .await
    .map_err(|e| anyhow::anyhow!("GitHub Copilot login failed: {e}"))?;

    let login_info = tokens.login.as_deref().unwrap_or("(unknown user)");
    println!("Login successful! GitHub user: {login_info}");

    post_login_save_profile("github-copilot")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Use
// ---------------------------------------------------------------------------

/// Switch the active provider profile.
fn use_command(profile: String) -> anyhow::Result<()> {
    let mut config = EliConfig::load();

    if !config.set_active(&profile) {
        // Try normalized name.
        let normalized = normalize_provider(&profile).to_string();
        if !config.set_active(&normalized) {
            let available: Vec<&str> = config.profiles.keys().map(|k| k.as_str()).collect();
            if available.is_empty() {
                anyhow::bail!(
                    "No profiles configured. Run `eli login <provider>` first.\n\
                     Supported providers: openai, claude, github-copilot"
                );
            } else {
                anyhow::bail!(
                    "Profile '{}' not found.\nAvailable profiles: {}",
                    profile,
                    available.join(", ")
                );
            }
        }
    }

    config.save()?;

    let active = config.active_profile().unwrap();
    println!(
        "Switched to profile: {}",
        config.active_profile.as_deref().unwrap_or("")
    );
    println!("  Provider: {}", active.provider);
    println!("  Model:    {}", active.model);

    Ok(())
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Show authentication and configuration status.
fn status_command() -> anyhow::Result<()> {
    let config = EliConfig::load();
    let auth = load_auth_status();

    println!("Eli configuration status");
    println!("========================");
    println!();

    // Active profile.
    println!("Active profile:");
    match config.active_profile() {
        Some(profile) => {
            println!(
                "  {} (provider: {}, model: {})",
                config.active_profile.as_deref().unwrap_or("(none)"),
                profile.provider,
                profile.model
            );
        }
        None => {
            println!("  (none) -- run `eli login <provider>` to get started");
        }
    }
    println!();

    // All profiles.
    println!("Profiles:");
    if config.profiles.is_empty() {
        println!("  (none)");
    } else {
        let mut names: Vec<&String> = config.profiles.keys().collect();
        names.sort();
        for name in names {
            let p = &config.profiles[name];
            let active_marker = if config.active_profile.as_deref() == Some(name.as_str()) {
                " *"
            } else {
                ""
            };
            println!(
                "  {name}{active_marker} (provider: {}, model: {})",
                p.provider, p.model
            );
        }
    }
    println!();

    // Credentials.
    println!("Stored credentials:");
    if auth.is_empty() {
        println!("  (none)");
    } else {
        let mut providers: Vec<&String> = auth.keys().collect();
        providers.sort();
        for provider in providers {
            println!("  {}: {}", provider, auth[provider]);
        }
    }
    println!();

    // Environment variable overrides.
    println!("Environment overrides:");
    let env_vars = [
        "ELI_MODEL",
        "ELI_API_KEY",
        "ELI_API_BASE",
        "ELI_API_FORMAT",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
    ];
    let mut has_override = false;
    for var in &env_vars {
        if let Ok(val) = std::env::var(var) {
            let display = if var.contains("KEY") {
                if val.len() > 12 {
                    format!("{}...{}", &val[..7], &val[val.len() - 4..])
                } else {
                    "****".to_string()
                }
            } else {
                val
            };
            println!("  {var}={display}");
            has_override = true;
        }
    }
    if !has_override {
        println!("  (none)");
    }

    println!();
    println!("Config file: {}", EliConfig::config_path().display());

    Ok(())
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

/// Show registered hooks.
async fn hooks_command() {
    let framework = builtin_framework().await;
    let mut report: Vec<_> = framework.hook_report().await.into_iter().collect();
    report.sort_by(|a, b| a.0.cmp(&b.0));
    println!("Hook implementations:");
    for (name, mut plugins) in report {
        plugins.sort();
        println!("  {name}:");
        if plugins.is_empty() {
            println!("    - (none)");
            continue;
        }
        for plugin in plugins {
            println!("    - {plugin}");
        }
    }
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Manage model selection: show, list, or switch.
async fn model_command(name: Option<String>) -> anyhow::Result<()> {
    match name.as_deref() {
        None => model_show(),
        Some("list") => model_list().await,
        Some(model_name) => model_switch(model_name),
    }
}

/// Show the current model.
fn model_show() -> anyhow::Result<()> {
    let config = EliConfig::load();

    match config.active_profile() {
        Some(profile) => {
            let profile_name = config.active_profile.as_deref().unwrap_or("(none)");
            println!("Current model: {}", profile.model);
            println!("  Profile:  {profile_name}");
            println!("  Provider: {}", profile.provider);
        }
        None => {
            println!("No active profile configured.");
            println!("Run `eli login <provider>` to get started.");
        }
    }

    // Show env var override if set.
    if let Ok(env_model) = std::env::var("ELI_MODEL") {
        println!();
        println!("Note: ELI_MODEL environment variable is set to: {env_model}");
        println!("  This overrides the configured model at runtime.");
    }

    Ok(())
}

/// Resolve an API key for the given provider for model listing purposes.
fn resolve_api_key_for_provider(provider: &str) -> anyhow::Result<String> {
    // 1. Check ELI_API_KEY env var.
    if let Ok(key) = std::env::var("ELI_API_KEY")
        && !key.is_empty()
    {
        return Ok(key);
    }

    match provider {
        "anthropic" => {
            // Check ANTHROPIC_API_KEY env var.
            if let Ok(key) = std::env::var("ANTHROPIC_API_KEY")
                && !key.is_empty()
            {
                return Ok(key);
            }
            // Check auth.json (handles both OAuth tokens and legacy API keys).
            if let Some(key) = crate::builtin::config::load_anthropic_api_key() {
                return Ok(key);
            }
            anyhow::bail!(
                "No API key found for Anthropic.\n\
                 Run `eli login claude` or set ANTHROPIC_API_KEY."
            );
        }
        "openai" => {
            // Check OPENAI_API_KEY env var.
            if let Ok(key) = std::env::var("OPENAI_API_KEY")
                && !key.is_empty()
            {
                return Ok(key);
            }
            // Check Codex OAuth tokens.
            let codex_home = std::env::var("CODEX_HOME")
                .ok()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    dirs::home_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join(".codex")
                });
            let resolver =
                conduit::auth::openai_codex::codex_cli_api_key_resolver(Some(codex_home));
            if let Some(key) = resolver("openai") {
                return Ok(key);
            }
            anyhow::bail!(
                "No API key found for OpenAI.\n\
                 Run `eli login openai` or set OPENAI_API_KEY."
            );
        }
        "github-copilot" => {
            // Use the github copilot resolver which checks stored tokens, env vars, gh CLI.
            let resolver =
                conduit::auth::github_copilot::github_copilot_oauth_resolver(None, None, None);
            if let Some(key) = resolver("github-copilot") {
                return Ok(key);
            }
            anyhow::bail!(
                "No GitHub token found for Copilot.\n\
                 Run `eli login github-copilot` or set GITHUB_TOKEN."
            );
        }
        "openrouter" => {
            if let Ok(key) = std::env::var("OPENROUTER_API_KEY")
                && !key.is_empty()
            {
                return Ok(key);
            }
            anyhow::bail!(
                "No API key found for OpenRouter.\n\
                 Set OPENROUTER_API_KEY environment variable."
            );
        }
        _ => {
            anyhow::bail!(
                "Cannot resolve API key for unknown provider: {provider}\n\
                 Set ELI_API_KEY environment variable."
            );
        }
    }
}

/// Default API base URL for a provider.
fn default_api_base(provider: &str) -> &str {
    match provider {
        "openai" => "https://api.openai.com/v1",
        "anthropic" => "https://api.anthropic.com/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        "github-copilot" => "https://api.githubcopilot.com",
        _ => "https://api.openai.com/v1",
    }
}

/// Fetch available models from a provider's API.
async fn fetch_models(provider: &str, api_key: &str) -> anyhow::Result<Vec<String>> {
    let client = reqwest::Client::new();
    match provider {
        "openai" => fetch_models_openai_codex(&client, api_key).await,
        // Anthropic's model list is curated — no dynamic API needed.
        "anthropic" => Ok(known_models("anthropic")),
        "github-copilot" => fetch_models_github_copilot(&client, api_key).await,
        _ => fetch_models_openai_compatible(&client, api_key, default_api_base(provider)).await,
    }
}

/// Fetch models from OpenAI Codex backend (chatgpt.com/backend-api/codex/models).
/// Codex OAuth tokens work against this endpoint, not api.openai.com/v1/models.
async fn fetch_models_openai_codex(
    client: &reqwest::Client,
    api_key: &str,
) -> anyhow::Result<Vec<String>> {
    let resp = client
        .get("https://chatgpt.com/backend-api/codex/models?client_version=0.114.0")
        .bearer_auth(api_key)
        .send()
        .await?;

    if !resp.status().is_success() {
        // Fallback to standard OpenAI API (works with API keys, not OAuth).
        return fetch_models_openai_compatible(client, api_key, "https://api.openai.com/v1").await;
    }

    let body: serde_json::Value = resp.json().await?;

    // Codex returns { "models": [ { "slug": "...", "visibility": "..." }, ... ] }
    let models = body["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|m| m["visibility"].as_str() != Some("hidden"))
                .filter_map(|m| m["slug"].as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(models)
}

/// Fetch models from an OpenAI-compatible API (OpenAI, OpenRouter, etc.).
async fn fetch_models_openai_compatible(
    client: &reqwest::Client,
    api_key: &str,
    api_base: &str,
) -> anyhow::Result<Vec<String>> {
    let url = format!("{}/models", api_base.trim_end_matches('/'));
    let resp = client.get(&url).bearer_auth(api_key).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("API request failed (HTTP {status}): {text}");
    }

    let body: serde_json::Value = resp.json().await?;
    let models = body["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(models)
}

/// Fetch models from the GitHub Copilot API.
///
/// The GitHub token must first be exchanged for a Copilot session token.
async fn fetch_models_github_copilot(
    client: &reqwest::Client,
    github_token: &str,
) -> anyhow::Result<Vec<String>> {
    // Exchange GitHub token for Copilot session token.
    let token_resp = client
        .get("https://api.github.com/copilot_internal/v2/token")
        .header("Authorization", format!("Bearer {github_token}"))
        .header("User-Agent", "conduit-eli/0")
        .header("Accept", "application/json")
        .send()
        .await?;

    if !token_resp.status().is_success() {
        let status = token_resp.status();
        let text = token_resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Failed to get Copilot session token (HTTP {status}): {text}\n\
             Make sure you have an active GitHub Copilot subscription."
        );
    }

    let token_body: serde_json::Value = token_resp.json().await?;
    let session_token = token_body["token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Copilot token response missing 'token' field"))?;

    // Fetch models from the Copilot API.
    let resp = client
        .get("https://api.githubcopilot.com/models")
        .header("Authorization", format!("Bearer {session_token}"))
        .header("Copilot-Integration-Id", "vscode-chat")
        .header("User-Agent", "conduit-eli/0")
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Copilot models API request failed (HTTP {status}): {text}");
    }

    let body: serde_json::Value = resp.json().await?;

    // Copilot returns { "data": [...] } or a flat array of model objects.
    let models = if let Some(arr) = body["data"].as_array() {
        arr.iter()
            .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
            .collect()
    } else if let Some(arr) = body.as_array() {
        arr.iter()
            .filter_map(|m| {
                m["id"]
                    .as_str()
                    .or_else(|| m["model"].as_str())
                    .map(|s| s.to_string())
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(models)
}

/// Filter out non-chat models for OpenAI provider.
fn filter_chat_models(provider: &str, models: &[String]) -> Vec<String> {
    if provider != "openai" {
        return models.to_vec();
    }

    // Skip embedding, tts, whisper, dall-e, and moderation models.
    let skip_prefixes = [
        "text-embedding",
        "tts-",
        "whisper-",
        "dall-e",
        "davinci",
        "babbage",
        "curie",
        "ada",
    ];
    let skip_contains = [
        "embedding",
        "moderation",
        "realtime",
        "transcribe",
        "search",
    ];

    models
        .iter()
        .filter(|m| {
            let lower = m.to_lowercase();
            !skip_prefixes.iter().any(|p| lower.starts_with(p))
                && !skip_contains.iter().any(|c| lower.contains(c))
        })
        .cloned()
        .collect()
}

/// Known models per provider (fallback when API is unavailable).
fn known_models(provider: &str) -> Vec<String> {
    let list: &[&str] = match provider {
        "openai" => &[
            // GPT-5.4 (latest frontier, March 2026)
            "gpt-5.4",
            "gpt-5.4-pro",
            "gpt-5.4-mini",
            "gpt-5.4-nano",
            // GPT-5.x
            "gpt-5.3-codex",
            "gpt-5.3-codex-spark",
            "gpt-5.2",
            "gpt-5.2-pro",
            "gpt-5.2-codex",
            "gpt-5.1",
            "gpt-5.1-codex-max",
            "gpt-5.1-codex",
            "gpt-5.1-codex-mini",
            "gpt-5",
            "gpt-5-pro",
            "gpt-5-mini",
            "gpt-5-nano",
            "gpt-5-codex",
            "gpt-5-codex-mini",
            // O-series reasoning
            "o3",
            "o3-pro",
            "o3-mini",
            "o4-mini",
            // GPT-4.x
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4.1-nano",
            "gpt-4o",
            "gpt-4o-mini",
            // Open-weight
            "gpt-oss-120b",
            "gpt-oss-20b",
        ],
        "anthropic" => &[
            // Claude 4.6 (latest, March 2026)
            "claude-opus-4-6",
            "claude-sonnet-4-6",
            // Claude 4.5
            "claude-opus-4-5-20251101",
            "claude-sonnet-4-5-20250929",
            "claude-haiku-4-5-20251001",
            // Claude 4.0
            "claude-opus-4-20250514",
            "claude-sonnet-4-20250514",
            // Claude 4.1
            "claude-opus-4-1-20250805",
        ],
        "github-copilot" => &[
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5.3-codex",
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4.1",
            "claude-sonnet-4",
            "o3-mini",
            "o4-mini",
        ],
        "openrouter" => &[
            "openai/gpt-5.4",
            "openai/gpt-5.4-mini",
            "openai/gpt-4o",
            "openai/gpt-4o-mini",
            "anthropic/claude-opus-4-6",
            "anthropic/claude-sonnet-4-6",
            "anthropic/claude-sonnet-4-20250514",
            "google/gemini-2.5-pro",
        ],
        _ => &[],
    };
    list.iter().map(|s| s.to_string()).collect()
}

/// List available models from the active provider.
async fn model_list() -> anyhow::Result<()> {
    let config = EliConfig::load();
    let profile = config.active_profile().ok_or_else(|| {
        anyhow::anyhow!("No active profile configured.\nRun `eli login <provider>` to get started.")
    })?;

    let provider = profile.provider.as_str();
    println!("Fetching models from {provider}...");
    println!();

    let api_key = resolve_api_key_for_provider(provider);
    let mut models = match api_key {
        Ok(key) => match fetch_models(provider, &key).await {
            Ok(m) => {
                let filtered = filter_chat_models(provider, &m);
                if filtered.is_empty() { m } else { filtered }
            }
            Err(e) => {
                eprintln!("  (API fetch failed: {e})");
                eprintln!("  Showing known models instead.");
                eprintln!();
                known_models(provider)
            }
        },
        Err(_) => {
            eprintln!("  (No API key available, showing known models)");
            eprintln!();
            known_models(provider)
        }
    };
    models.sort();
    models.dedup();

    if models.is_empty() {
        println!("No models found.");
    } else {
        println!("Available models ({provider}):");
        for model in &models {
            let marker = if config
                .active_profile()
                .is_some_and(|p| p.model == *model || p.model == format!("{provider}:{model}"))
            {
                " *"
            } else {
                ""
            };
            println!("  {model}{marker}");
        }
    }

    Ok(())
}

/// Switch the active profile's model.
fn model_switch(model_name: &str) -> anyhow::Result<()> {
    let mut config = EliConfig::load();

    let profile_name = config.active_profile.clone().ok_or_else(|| {
        anyhow::anyhow!("No active profile configured.\nRun `eli login <provider>` to get started.")
    })?;

    let profile = config
        .profiles
        .get_mut(&profile_name)
        .ok_or_else(|| anyhow::anyhow!("Active profile '{profile_name}' not found in config"))?;

    let old_model = profile.model.clone();
    // Ensure model is stored in provider:model format.
    let new_model = if model_name.contains(':') {
        model_name.to_string()
    } else {
        format!("{}:{}", profile.provider, model_name)
    };
    profile.model = new_model.clone();
    config.save()?;

    println!("Model updated for profile '{profile_name}':");
    println!("  {old_model} -> {new_model}");

    Ok(())
}

// ---------------------------------------------------------------------------
// Gateway
// ---------------------------------------------------------------------------

/// Resolve the sidecar directory. Search order:
///   1. `ELI_SIDECAR_DIR` env var
///   2. `sidecar/` next to the current executable
///   3. `sidecar/` in the current working directory
fn find_sidecar_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    let candidates: Vec<PathBuf> = [
        std::env::var("ELI_SIDECAR_DIR").ok().map(PathBuf::from),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("sidecar"))),
        std::env::current_dir().ok().map(|d| d.join("sidecar")),
    ]
    .into_iter()
    .flatten()
    .collect();

    candidates
        .into_iter()
        .find(|d| d.join("start.cjs").exists())
}

/// Prompt for a line of input with the given label.
fn prompt_line(label: &str) -> String {
    use std::io::Write;
    print!("{label}");
    std::io::stdout().flush().unwrap();
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf).unwrap();
    buf.trim().to_owned()
}

/// Ensure sidecar.json exists. If not, interactively prompt for channel
/// credentials and write it.
fn ensure_sidecar_config(sidecar_dir: &std::path::Path) {
    let config_path = sidecar_dir.join("sidecar.json");
    if config_path.exists() {
        return;
    }

    println!("\n  No sidecar.json found — let's set up your channel.\n");
    println!("  Which channel plugin? (default: @larksuite/openclaw-lark)");
    let plugin = prompt_line("  Plugin: ");
    let plugin = if plugin.is_empty() {
        "@larksuite/openclaw-lark".to_owned()
    } else {
        plugin
    };

    // Determine channel id from plugin name.
    let channel_id = if plugin.contains("lark") || plugin.contains("feishu") {
        "feishu"
    } else if plugin.contains("dingtalk") {
        "dingtalk"
    } else if plugin.contains("discord") {
        "discord"
    } else if plugin.contains("slack") {
        "slack"
    } else {
        &*prompt_line("  Channel ID (e.g. feishu, slack): ")
            .to_owned()
            .leak()
    };

    println!("\n  Enter credentials for {channel_id}:");
    let app_id = prompt_line("  App ID: ");
    let app_secret = prompt_line("  App Secret: ");

    // For feishu, ask domain (feishu vs lark).
    let domain = if channel_id == "feishu" {
        let d = prompt_line("  Domain (feishu/lark) [feishu]: ");
        if d.is_empty() { "feishu".to_owned() } else { d }
    } else {
        String::new()
    };

    // Build config JSON.
    let mut channel_config = serde_json::json!({
        "enabled": true,
        "appId": app_id,
        "appSecret": app_secret,
        "accounts": {
            "default": {
                "appId": app_id,
                "appSecret": app_secret,
            }
        }
    });
    if !domain.is_empty() {
        channel_config["domain"] = serde_json::json!(domain);
        channel_config["accounts"]["default"]["domain"] = serde_json::json!(domain);
    }

    let config = serde_json::json!({
        "eli_url": "http://127.0.0.1:3100",
        "port": 3101,
        "plugins": [plugin],
        "channels": {
            channel_id: channel_config,
        }
    });

    let json = serde_json::to_string_pretty(&config).unwrap();
    std::fs::write(&config_path, &json).unwrap();
    println!("\n  Saved {}\n", config_path.display());
}

/// Find and start the Node sidecar process.
/// Returns `Some(Child)` if spawned, `None` if not found or failed.
fn start_sidecar(wh: &crate::channels::webhook::WebhookSettings) -> Option<std::process::Child> {
    let sidecar_dir = match find_sidecar_dir() {
        Some(d) => d,
        None => {
            println!("Sidecar directory not found, skipping");
            return None;
        }
    };

    // Check that node is available.
    if std::process::Command::new("node")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("Warning: `node` not found in PATH, cannot start sidecar");
        return None;
    }

    // Check node_modules exists.
    if !sidecar_dir.join("node_modules").exists() {
        println!("Installing sidecar dependencies...");
        let install = std::process::Command::new("npm")
            .arg("install")
            .current_dir(&sidecar_dir)
            .status();
        if install.is_err() || !install.unwrap().success() {
            eprintln!("Warning: `npm install` failed in {}", sidecar_dir.display());
            return None;
        }
    }

    // Ensure sidecar.json exists (prompt if missing).
    ensure_sidecar_config(&sidecar_dir);

    println!("Starting sidecar from {}...", sidecar_dir.display());

    let eli_url = format!("http://127.0.0.1:{}", wh.listen_port);
    // Pass workspace path so sidecar writes SKILL.md files to the project root,
    // where discover_skills() can find them.
    let workspace = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    match std::process::Command::new("node")
        .arg("start.cjs")
        .current_dir(&sidecar_dir)
        .env("SIDECAR_ELI_URL", &eli_url)
        .env("SIDECAR_SKILLS_DIR", &workspace)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(child) => {
            println!("Sidecar started (pid={})", child.id());
            Some(child)
        }
        Err(e) => {
            eprintln!("Failed to start sidecar: {e}");
            None
        }
    }
}

/// Wait for the sidecar to be ready and register its URL for the bridge tool.
/// Skills are discovered from .agents/skills/ SKILL.md files (standard protocol)
/// — the sidecar writes them to disk on startup.
async fn wait_for_sidecar(sidecar_url: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();

    for attempt in 0..15 {
        match client.get(format!("{sidecar_url}/health")).send().await {
            Ok(resp) if resp.status().is_success() => {
                *crate::tools::SIDECAR_URL.lock().unwrap() = Some(sidecar_url.to_owned());
                println!("Sidecar ready at {sidecar_url} (skills via .agents/skills/)");
                return Ok(());
            }
            _ => {
                if attempt < 14 {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }
    anyhow::bail!("sidecar not reachable at {sidecar_url}");
}

/// Start channel listeners (Telegram, Webhook, etc.).
async fn gateway_command(enable_channels: Vec<String>) -> anyhow::Result<()> {
    use std::collections::HashMap;

    use crate::channels::base::Channel;
    use crate::channels::message::ChannelMessage;
    use crate::channels::telegram::{TelegramChannel, TelegramSettings};
    use crate::channels::webhook::{WebhookChannel, WebhookSettings};
    use tokio_util::sync::CancellationToken;

    // Load .env so ELI_TELEGRAM_TOKEN (and others) are available.
    let _ = dotenvy::dotenv();

    let should_enable = |name: &str| -> bool {
        enable_channels.is_empty() || enable_channels.iter().any(|c| c == name || c == "all")
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let mut channels: HashMap<String, Arc<dyn Channel>> = HashMap::new();
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // -- Telegram --
    if should_enable("telegram") {
        let tg_settings = TelegramSettings::from_env();
        if !tg_settings.token.is_empty() {
            let tg = Arc::new(TelegramChannel::new(tx.clone(), tg_settings));
            println!("Starting Telegram channel...");
            let ch = tg.clone();
            let c = cancel.clone();
            tasks.push(tokio::spawn(async move {
                if let Err(e) = Channel::start(&*ch, c).await {
                    eprintln!("Telegram channel error: {e}");
                }
            }));
            channels.insert("telegram".to_owned(), tg);
        }
    }

    // -- Webhook + Sidecar --
    let mut sidecar_child: Option<std::process::Child> = None;
    if should_enable("webhook") {
        let wh_settings = WebhookSettings::from_env();
        if wh_settings.is_configured() || should_enable("webhook") {
            // Auto-start the Node sidecar if a sidecar directory is found.
            sidecar_child = start_sidecar(&wh_settings);

            let wh = Arc::new(WebhookChannel::new(tx.clone(), wh_settings));
            println!("Starting Webhook channel...");
            let ch = wh.clone();
            let c = cancel.clone();
            tasks.push(tokio::spawn(async move {
                if let Err(e) = Channel::start(&*ch, c).await {
                    eprintln!("Webhook channel error: {e}");
                }
            }));
            channels.insert("webhook".to_owned(), wh);
        }
    }

    if channels.is_empty() {
        anyhow::bail!(
            "No channels configured.\n\
             Set ELI_TELEGRAM_TOKEN for Telegram, or ELI_WEBHOOK_PORT for Webhook."
        );
    }

    // -- Sidecar --
    // Wait for sidecar to be ready. Skills are on disk (.agents/skills/).
    if sidecar_child.is_some()
        && let Err(e) = wait_for_sidecar("http://127.0.0.1:3101").await
    {
        eprintln!("Warning: sidecar not ready: {e}");
    }

    // Handle Ctrl-C.
    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!("\nShutting down...");
        cancel_for_signal.cancel();
    });

    let framework = builtin_framework().await;
    loop {
        tokio::select! {
            Some(msg) = rx.recv() => {
                let source_channel = msg.channel.clone();
                let output_channel = if msg.output_channel.is_empty() {
                    source_channel.clone()
                } else {
                    msg.output_channel.clone()
                };

                let inbound_context = msg.context.clone();
                let inbound = serde_json::json!({
                    "session_id": msg.session_id,
                    "channel": msg.channel,
                    "chat_id": msg.chat_id,
                    "content": msg.content,
                    "context": msg.context,
                    "kind": msg.kind,
                    "output_channel": output_channel,
                });

                match framework.process_inbound(inbound).await {
                    Ok(result) => {
                        tracing::info!(session = %result.session_id, "framework run completed");
                        for outbound in &result.outbounds {
                            let out_ch = outbound
                                .get("output_channel")
                                .and_then(|v| v.as_str())
                                .or_else(|| outbound.get("channel").and_then(|v| v.as_str()))
                                .unwrap_or("");

                            let channel = match channels.get(out_ch) {
                                Some(ch) => ch.clone(),
                                None => continue,
                            };

                            let content = outbound_string_field(outbound, "content");
                            if content.trim().is_empty() {
                                continue;
                            }

                            let chat_id = outbound_string_field(outbound, "chat_id");
                            if chat_id.is_empty() {
                                continue;
                            }

                            let session_id = outbound
                                .get("session_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&result.session_id);
                            // Carry over context from the outbound or inbound so
                            // the webhook sidecar can route replies correctly.
                            let reply_context = outbound
                                .get("context")
                                .and_then(|v| v.as_object())
                                .cloned()
                                .unwrap_or_else(|| inbound_context.clone());
                            let reply = ChannelMessage::new(session_id, out_ch, &content)
                                .with_chat_id(chat_id)
                                .with_context(reply_context)
                                .finalize();
                            if let Err(e) = channel.send(reply).await {
                                eprintln!("Failed to send reply via {out_ch}: {e}");
                            }
                        }
                    }
                    Err(e) => eprintln!("Framework error: {e}"),
                }
            }
            () = cancel.cancelled() => {
                break;
            }
        }
    }

    // Clean up.
    if let Some(mut child) = sidecar_child {
        println!("Stopping sidecar (pid={})...", child.id());
        let _ = child.kill();
        // Non-blocking wait with timeout.
        let waited = std::thread::spawn(move || child.wait());
        match waited.join() {
            Ok(Ok(_)) => println!("Sidecar stopped."),
            _ => println!("Sidecar force-killed."),
        }
    }
    for (name, ch) in &channels {
        if let Err(e) = ch.stop().await {
            eprintln!("Error stopping {name}: {e}");
        }
    }
    for task in tasks {
        let _ = task.await;
    }
    println!("Gateway stopped.");
    Ok(())
}

/// Remove hallucinated `<function_calls>...</function_calls>` blocks and
/// surrounding narration like "I'll respond..." from model output.
pub(crate) fn strip_fake_tool_calls(text: &str) -> String {
    // Remove <function_calls>...</function_calls> blocks (greedy, may span multiple lines)
    let re = regex::Regex::new(r"(?s)<function_calls>.*?</function_calls>").unwrap();
    let cleaned = re.replace_all(text, "");
    cleaned.trim().to_owned()
}

async fn builtin_framework() -> Arc<EliFramework> {
    let framework = Arc::new(EliFramework::new());
    framework
        .register_plugin("builtin", Arc::new(BuiltinImpl::new()))
        .await;
    framework
}

fn print_cli_outbounds(outbounds: &[Value]) {
    for outbound in outbounds {
        let content = outbound_string_field(outbound, "content");
        if !content.trim().is_empty() {
            println!("{content}");
        }
    }
}

fn outbound_string_field(outbound: &Value, key: &str) -> String {
    match outbound.get(key) {
        Some(Value::String(value)) => value.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

async fn tape_command(port: u16, dir: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let tapes_dir = dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".eli")
            .join("tapes")
    });
    crate::builtin::tape_viewer::serve(tapes_dir, port).await
}
