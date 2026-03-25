//! Profile switching and status display.

use crate::builtin::config::{EliConfig, load_auth_status, normalize_provider};

/// Switch the active provider profile.
pub(crate) fn use_command(profile: String) -> anyhow::Result<()> {
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
        println!(
            "  {name}{active_marker} (provider: {}, model: {})",
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
