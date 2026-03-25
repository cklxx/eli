//! Model management: show, list, switch.

use crate::builtin::config::EliConfig;
use crate::builtin::settings::EnvConfig;

/// Manage model selection: show, list, or switch.
pub(crate) async fn model_command(name: Option<String>) -> anyhow::Result<()> {
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
    if let Some(env_model) = EnvConfig::model_override() {
        println!();
        println!("Note: ELI_MODEL environment variable is set to: {env_model}");
        println!("  This overrides the configured model at runtime.");
    }

    Ok(())
}

/// Resolve an API key for the given provider for model listing purposes.
fn resolve_api_key_for_provider(provider: &str) -> anyhow::Result<String> {
    // 1. Check ELI_API_KEY env var.
    if let Some(key) = EnvConfig::api_key(None)
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
            let resolver = nexil::auth::openai_codex::codex_cli_api_key_resolver(Some(codex_home));
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
                nexil::auth::github_copilot::github_copilot_oauth_resolver(None, None, None);
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
