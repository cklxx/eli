//! CLI commands: run, chat, login, use, status, hooks, gateway, model, tape, decisions.

mod chat;
mod decisions;
mod gateway;
mod login;
mod model;
mod profile;
mod run;
mod tape;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Subcommand;
use serde_json::Value;

use crate::builtin::BuiltinImpl;
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
    /// Manage persistent decisions.
    Decisions {
        #[command(subcommand)]
        action: DecisionAction,
    },
}

/// Decision management actions.
#[derive(Debug, Subcommand)]
pub enum DecisionAction {
    /// List active decisions.
    List,
    /// Remove a decision by number.
    Remove {
        /// Decision number (1-based, from `eli decisions list`).
        index: usize,
    },
    /// Export decisions as markdown.
    Export,
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
        } => run::run_command(message, channel, chat_id, sender_id, session_id).await,
        CliCommand::Chat {
            chat_id,
            session_id,
        } => chat::chat_command(chat_id, session_id).await,
        CliCommand::Login {
            provider,
            codex_home,
            browser,
            manual,
            timeout,
            api_key,
        } => login::login_command(provider, codex_home, browser, manual, timeout, api_key).await,
        CliCommand::Use { profile } => profile::use_command(profile),
        CliCommand::Model { name } => model::model_command(name).await,
        CliCommand::Status => profile::status_command(),
        CliCommand::Hooks => {
            hooks_command().await;
            Ok(())
        }
        CliCommand::Gateway { enable_channels } => gateway::gateway_command(enable_channels).await,
        CliCommand::Tape { port, dir } => tape::tape_command(port, dir).await,
        CliCommand::Decisions { action } => match action {
            DecisionAction::List => decisions::list_command().await,
            DecisionAction::Remove { index } => decisions::remove_command(index).await,
            DecisionAction::Export => decisions::export_command().await,
        },
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
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
