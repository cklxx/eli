//! Profile switching and status display.

use std::io::{self, IsTerminal, Write};

use crate::builtin::config::{EliConfig, load_auth_status, normalize_provider};

/// Switch the active provider profile.
///
/// With no argument, launches a numeric picker listing all configured
/// profiles (active one marked with `*`).
pub(crate) fn use_command(profile: Option<String>) -> anyhow::Result<()> {
    let mut config = EliConfig::load();

    let target = match profile {
        Some(name) => name,
        None => pick_profile_interactively(&config)?,
    };

    if !config.set_active(&target) {
        let normalized = normalize_provider(&target);
        if !config.set_active(&normalized) {
            let available: Vec<&str> = config.profiles.keys().map(|k| k.as_str()).collect();
            if available.is_empty() {
                anyhow::bail!(
                    "No profiles configured. Run `eli login <provider>` first.\n\
                     Supported providers: openai, claude, github-copilot, agent-infer"
                );
            } else {
                anyhow::bail!(
                    "Profile '{}' not found.\nAvailable profiles: {}",
                    target,
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
    if let Some(base) = active.api_base.as_deref() {
        println!("  Endpoint: {base}");
    }

    Ok(())
}

/// Interactive numeric picker over configured profiles.
///
/// The active profile is the default (empty input selects it). In non-TTY
/// environments, an explicit selection is still required — stdin is read
/// regardless, so this also works when piped.
fn pick_profile_interactively(config: &EliConfig) -> anyhow::Result<String> {
    if config.profiles.is_empty() {
        anyhow::bail!(
            "No profiles configured. Run `eli login <provider>` first.\n\
             Supported providers: openai, claude, github-copilot, agent-infer"
        );
    }

    let mut names: Vec<&String> = config.profiles.keys().collect();
    names.sort();

    println!("Select a profile:");
    for (idx, name) in names.iter().enumerate() {
        let p = &config.profiles[*name];
        let marker = if config.active_profile.as_deref() == Some(name.as_str()) {
            " *"
        } else {
            "  "
        };
        let endpoint = p
            .api_base
            .as_deref()
            .map(|b| format!(" @ {b}"))
            .unwrap_or_default();
        println!(
            "{marker}[{n}] {name}  ({provider} · {model}{endpoint})",
            n = idx + 1,
            provider = p.provider,
            model = p.model,
        );
    }

    // If the picker is invoked without a TTY and no input, fall back to the
    // active profile (if any) so scripts don't deadlock on stdin.
    let default_idx = config
        .active_profile
        .as_deref()
        .and_then(|active| names.iter().position(|n| n.as_str() == active));

    let prompt = match default_idx {
        Some(i) => format!("Enter number [1-{}] (default {}): ", names.len(), i + 1),
        None => format!("Enter number [1-{}]: ", names.len()),
    };
    print!("{prompt}");
    io::stdout().flush()?;

    let mut input = String::new();
    if !io::stdin().is_terminal()
        && let Some(i) = default_idx
    {
        // Non-interactive: keep the current active profile.
        return Ok(names[i].clone());
    }
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    let choice_idx = if trimmed.is_empty() {
        default_idx.ok_or_else(|| anyhow::anyhow!("no default profile; please enter a number"))?
    } else {
        let n: usize = trimmed
            .parse()
            .map_err(|_| anyhow::anyhow!("not a number: {trimmed}"))?;
        if n == 0 || n > names.len() {
            anyhow::bail!("out of range: {n} (expected 1..={})", names.len());
        }
        n - 1
    };

    Ok(names[choice_idx].clone())
}

/// Show authentication and configuration status.
pub(crate) fn status_command() -> anyhow::Result<()> {
    let config = EliConfig::load();
    let auth = load_auth_status();

    println!("Eli configuration status");
    println!("========================");
    println!();
    print_active_profile(&config);
    print_profiles(&config);
    print_credentials(&auth);
    print_env_overrides();
    println!();
    println!("Config file: {}", EliConfig::config_path().display());
    Ok(())
}

fn print_active_profile(config: &EliConfig) {
    println!("Active profile:");
    match config.active_profile() {
        Some(profile) => println!(
            "  {} (provider: {}, model: {})",
            config.active_profile.as_deref().unwrap_or("(none)"),
            profile.provider,
            profile.model
        ),
        None => println!("  (none) -- run `eli login <provider>` to get started"),
    }
}

fn print_profiles(config: &EliConfig) {
    println!();
    println!("Profiles:");
    if config.profiles.is_empty() {
        println!("  (none)");
        return;
    }
    let mut names: Vec<&String> = config.profiles.keys().collect();
    names.sort();
    for name in names {
        let p = &config.profiles[name];
        let active_marker = if config.active_profile.as_deref() == Some(name.as_str()) {
            " *"
        } else {
            ""
        };
        let endpoint = p
            .api_base
            .as_deref()
            .map(|b| format!(", endpoint: {b}"))
            .unwrap_or_default();
        println!(
            "  {name}{active_marker} (provider: {}, model: {}{endpoint})",
            p.provider, p.model
        );
    }
}

fn print_credentials(auth: &std::collections::HashMap<String, String>) {
    println!();
    println!("Stored credentials:");
    if auth.is_empty() {
        println!("  (none)");
        return;
    }
    let mut providers: Vec<&String> = auth.keys().collect();
    providers.sort();
    for provider in providers {
        println!("  {}: {}", provider, auth[provider]);
    }
}

fn print_env_overrides() {
    const ENV_VARS: &[&str] = &[
        "ELI_MODEL",
        "ELI_API_KEY",
        "ELI_API_BASE",
        "ELI_API_FORMAT",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "ELI_VOLCANO_API_KEY",
        "VOLCANO_API_KEY",
        "ARK_API_KEY",
    ];
    println!();
    println!("Environment overrides:");
    let overrides: Vec<_> = ENV_VARS
        .iter()
        .filter_map(|var| std::env::var(var).ok().map(|val| (*var, val)))
        .collect();
    if overrides.is_empty() {
        println!("  (none)");
        return;
    }
    for (var, val) in overrides {
        let display = if var.contains("KEY") {
            redact_env_key(&val)
        } else {
            val
        };
        println!("  {var}={display}");
    }
}

fn redact_env_key(val: &str) -> String {
    if val.len() > 12 {
        format!("{}...{}", &val[..7], &val[val.len() - 4..])
    } else {
        "****".to_string()
    }
}
