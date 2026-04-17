//! Authentication / login flows for all providers.

use std::path::PathBuf;

use super::detect;
use crate::builtin::config::{EliConfig, Profile, default_model_for_provider, normalize_provider};

/// Mask an account ID for display: show first 3 and last 3 chars only.
fn mask_account_id(id: Option<&str>) -> String {
    let s = id.unwrap_or("unknown");
    if s.len() > 6 {
        format!("{}…{}", &s[..3], &s[s.len() - 3..])
    } else {
        s.to_string()
    }
}

/// Login to a provider.
pub(crate) async fn login_command(
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
        // All keyless local OpenAI-compatible servers share one detection
        // + save flow. The brand only affects the saved profile name and
        // the "how to start" hint shown when no server responds.
        "local" => login_local(None).await,
        "agent-infer" | "agent_infer" | "agentinfer" => login_local(Some("agent-infer")).await,
        "ollama" => login_local(Some("ollama")).await,
        "vllm" => login_local(Some("vllm")).await,
        "lmstudio" => login_local(Some("lmstudio")).await,
        "llama-cpp" | "llamacpp" | "llama.cpp" => login_local(Some("llama-cpp")).await,
        _ => anyhow::bail!(
            "Unsupported auth provider: {provider}\n\
             Supported providers: openai, claude, github-copilot, \
             local (or any of: agent-infer, ollama, vllm, lmstudio, llama-cpp)"
        ),
    }
}

/// Save a profile after login and print a summary.
fn post_login_save_profile(provider_raw: &str) -> anyhow::Result<()> {
    let provider = normalize_provider(provider_raw);
    let model = default_model_for_provider(&provider);
    save_profile_with_overrides(&provider, None, model, None, false)
}

/// Persist a profile with explicit model and optional api_base override.
///
/// Used by login flows that discover the model/endpoint at runtime
/// (e.g. `eli login local` probing `/v1/models`). `profile_name_override`
/// lets brand-named entries (`agent-infer`, `ollama`, …) save under a
/// distinct user-facing label while sharing one underlying provider type.
fn save_profile_with_overrides(
    provider_raw: &str,
    profile_name_override: Option<&str>,
    model: &str,
    api_base: Option<String>,
    set_active: bool,
) -> anyhow::Result<()> {
    let provider = normalize_provider(provider_raw);
    let profile_name = profile_name_override
        .map(|s| s.to_owned())
        .unwrap_or_else(|| provider.clone());

    let mut config = EliConfig::load();
    let had_active = config.active_profile.is_some();

    config.add_profile(
        &profile_name,
        Profile {
            provider: provider.clone(),
            model: model.to_string(),
            api_base: api_base.clone(),
        },
    );

    let became_active = if set_active || !had_active {
        config.active_profile = Some(profile_name.clone());
        true
    } else {
        false
    };

    config.save()?;

    println!();
    println!("  Profile:  {profile_name}");
    println!("  Provider: {provider}");
    println!("  Model:    {model}");
    if let Some(base) = api_base {
        println!("  Endpoint: {base}");
    }

    if became_active {
        if had_active {
            println!("  Active:   yes");
        } else {
            println!("  Active:   yes (auto-selected as first profile)");
        }
    } else {
        let current = config.active_profile.as_deref().unwrap_or("(none)");
        if current != profile_name {
            println!();
            println!("  Tip: run `eli use {profile_name}` to switch to this profile");
            println!("  (current active profile: {current})");
        }
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
    use nexil::auth::openai_codex::{
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

    println!(
        "Login successful! Account: {}",
        mask_account_id(tokens.account_id.as_deref())
    );
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

    crate::builtin::config::save_anthropic_api_key_entry(&api_key)?;

    let auth_path = crate::builtin::config::eli_home().join("auth.json");
    println!("API key saved to: {}", auth_path.display());

    post_login_save_profile("anthropic")?;

    Ok(())
}

/// Auto-detect a running local OpenAI-compatible server and persist a profile.
///
/// Probes candidate endpoints (see `detect::local_candidates`: `ELI_LOCAL_URL`
/// override, then default ports for agent-infer/vllm/llama.cpp/ollama/lmstudio,
/// plus any `ELI_LOCAL_PORTS` extras). Reads the served model from
/// `/v1/models`, prompts the user to confirm, then writes a profile with both
/// the discovered model and the `api_base` override.
///
/// `brand_hint` is purely cosmetic: it picks the saved profile name and the
/// "how to start" message shown when no server responds. `None` saves under
/// the generic name `local`. The profile's `provider` field is always `local`
/// so all brands share one runtime path.
async fn login_local(brand_hint: Option<&str>) -> anyhow::Result<()> {
    use std::io::{self, Write};

    let label = brand_hint.unwrap_or("local");
    println!("🔍 Probing {label}...");

    let hit = match detect::detect_local().await {
        Some(h) => h,
        None => {
            eprintln!();
            eprintln!("No local server responded on any candidate endpoint.");
            eprintln!();
            eprintln!("Tried:");
            for candidate in detect::local_candidates() {
                eprintln!("  - {candidate}");
            }
            eprintln!();
            print_start_hint(brand_hint);
            eprintln!();
            eprintln!("Or override the endpoint:");
            eprintln!("  ELI_LOCAL_URL=http://host:port eli login {label}");
            eprintln!("  ELI_LOCAL_PORTS=8090,9000 eli login {label}   # extra ports");
            anyhow::bail!("{label} not reachable");
        }
    };

    println!("  Endpoint: {}", hit.api_base);
    println!("  Model:    {}", hit.model_id);
    println!();
    print!("Save as profile '{label}' and set active? [y/N] ");
    io::stdout().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_ascii_lowercase();
    if answer != "y" && answer != "yes" {
        println!("Aborted.");
        return Ok(());
    }

    // Always prefix with `local:` unless the model id already starts with a
    // known provider prefix. Ollama-style tags like `llama3.2:3b` contain a
    // colon but the prefix is the model name, not a provider — so plain
    // `contains(':')` would mis-strip the prefix and break runtime lookup.
    let model_id = match hit.model_id.split_once(':') {
        Some((prefix, _)) if nexil::core::provider_policies::is_known_provider(prefix) => {
            hit.model_id.clone()
        }
        _ => format!("local:{}", hit.model_id),
    };

    save_profile_with_overrides("local", Some(label), &model_id, Some(hit.api_base), true)?;

    println!();
    println!("Done. Try: eli chat");
    Ok(())
}

/// Print a brand-specific "how to start the server" hint on the miss path.
fn print_start_hint(brand: Option<&str>) {
    eprintln!("Start a local server first:");
    match brand {
        Some("agent-infer") => {
            eprintln!("  cd ~/code/agent-infer && ./scripts/start_infer.sh");
            eprintln!("    # optional: ./scripts/start_infer.sh models/Qwen3-4B 8000");
            eprintln!("  # Docker (Linux + CUDA)");
            eprintln!(
                "  docker run --gpus all -v /path/to/model:/model \\\n    \
                 ghcr.io/cklxx/agent-infer:latest --model-path /model --port 8000"
            );
        }
        Some("ollama") => {
            eprintln!("  ollama serve                           # default port 11434");
            eprintln!("  ollama pull llama3.2                   # in another shell");
        }
        Some("vllm") => {
            eprintln!("  python -m vllm.entrypoints.openai.api_server \\");
            eprintln!("    --model <hf-id-or-path> --port 8000");
        }
        Some("lmstudio") => {
            eprintln!("  Open LM Studio → Local Server tab → Start Server (default :1234)");
        }
        Some("llama-cpp") => {
            eprintln!("  llama-server -m <model.gguf> --port 8080");
        }
        _ => {
            eprintln!("  Any OpenAI-compatible server on one of the probed ports above.");
            eprintln!("  Common defaults: agent-infer/vllm:8000, llama.cpp:8080,");
            eprintln!("                   ollama:11434, lmstudio:1234.");
        }
    }
}

/// GitHub Copilot device-flow login.
async fn login_github_copilot(open_browser: bool, timeout: f64) -> anyhow::Result<()> {
    use nexil::auth::github_copilot::{
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
