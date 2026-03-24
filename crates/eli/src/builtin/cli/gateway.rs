//! Gateway command: channel listeners (Telegram, Webhook) and sidecar management.

use std::sync::Arc;

use crate::channels::message::ChannelMessage;

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

    // Use process_group(0) so the sidecar and all its children share a
    // process group that we can kill atomically on shutdown.
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new("node");
    cmd.arg("start.cjs")
        .current_dir(&sidecar_dir)
        .env("SIDECAR_ELI_URL", &eli_url)
        .env("SIDECAR_SKILLS_DIR", &workspace)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .process_group(0);

    match cmd.spawn() {
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

/// Start channel listeners (Telegram, Webhook/Sidecar).
pub(crate) async fn gateway_command() -> anyhow::Result<()> {
    use std::collections::HashMap;

    use crate::channels::base::Channel;
    use crate::channels::telegram::{TelegramChannel, TelegramSettings};
    use crate::channels::webhook::{WebhookChannel, WebhookSettings};
    use tokio_util::sync::CancellationToken;

    // Load .env so ELI_TELEGRAM_TOKEN (and others) are available.
    let _ = dotenvy::dotenv();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let mut channels: HashMap<String, Arc<dyn Channel>> = HashMap::new();
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // -- Telegram --
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

    // -- Webhook + Sidecar (enabled when sidecar directory exists) --
    let mut sidecar_child: Option<std::process::Child> = None;
    let wh_settings = WebhookSettings::from_env();
    if find_sidecar_dir().is_some() || wh_settings.is_configured() {
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

    if channels.is_empty() {
        anyhow::bail!(
            "No channels configured.\n\
             Set ELI_TELEGRAM_TOKEN for Telegram, or add a sidecar/ directory."
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

    let framework = super::builtin_framework().await;
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

                            let content = super::outbound_string_field(outbound, "content");
                            let cleanup_only = outbound
                                .get("context")
                                .and_then(|v| v.as_object())
                                .and_then(|ctx| ctx.get(crate::builtin::CLEANUP_ONLY_CONTEXT_KEY))
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if content.trim().is_empty() && !cleanup_only {
                                continue;
                            }

                            let chat_id = super::outbound_string_field(outbound, "chat_id");
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

    // Clean up — kill the entire sidecar process group so child processes
    // (jiti workers, etc.) don't leak.
    if let Some(mut child) = sidecar_child {
        let pid = child.id();
        println!("Stopping sidecar (pgid={pid})...");
        // Kill the process group (negative pid) with SIGTERM.
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &format!("-{pid}")])
            .status();
        // Give it a moment to exit gracefully, then force-kill.
        let waited = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let _ = child.kill(); // SIGKILL if still alive
            child.wait()
        });
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
