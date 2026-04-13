//! CLI commands: run, chat, login, use, status, hooks, gateway, model, tape, decisions.

mod chat;
mod decisions;
mod evolution;
mod gateway;
mod login;
mod model;
mod profile;
mod run;
#[cfg(feature = "tape-viewer")]
mod tape;
mod task;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Subcommand, ValueEnum};

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
    /// Start message listeners (Telegram, Webhook/Sidecar).
    Gateway,
    /// Open the tape viewer web UI.
    #[cfg(feature = "tape-viewer")]
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
    /// Govern self-evolution candidates and promotions.
    Evolution {
        #[command(subcommand)]
        action: EvolutionAction,
    },
    /// Manage the task board.
    Task {
        #[command(subcommand)]
        action: TaskAction,
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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EvolutionStatusArg {
    Pending,
    Promoted,
    Rejected,
}

#[derive(Debug, Subcommand)]
pub enum EvolutionAction {
    /// List evolution candidates.
    List {
        /// Optional status filter.
        #[arg(long)]
        status: Option<EvolutionStatusArg>,
    },
    /// Show a candidate in full.
    Show {
        /// Candidate ID.
        id: String,
    },
    /// Capture a prompt-rule candidate.
    CaptureRule {
        /// Human-readable rule title.
        title: String,
        /// Short rationale or summary.
        #[arg(long)]
        summary: String,
        /// Final rule content.
        #[arg(long)]
        content: String,
    },
    /// Capture a skill candidate.
    CaptureSkill {
        /// Final skill name (directory name).
        skill_name: String,
        /// Optional display title.
        #[arg(long)]
        title: Option<String>,
        /// Skill description for frontmatter.
        #[arg(long)]
        description: String,
        /// Skill body markdown.
        #[arg(long)]
        content: String,
    },
    /// Promote a pending candidate.
    Promote {
        /// Candidate ID.
        id: String,
        /// Overwrite an existing promoted skill target.
        #[arg(long)]
        force: bool,
    },
    /// Reject a pending candidate.
    Reject {
        /// Candidate ID.
        id: String,
    },
}

/// Task board management actions.
#[derive(Debug, Subcommand)]
pub enum TaskAction {
    /// Add a new task to the board.
    Add {
        /// Task description.
        description: String,
        /// Task kind (e.g. explore, implement, review).
        #[arg(long, short)]
        kind: Option<String>,
        /// Priority: 0=low, 1=normal, 2=high, 3=urgent.
        #[arg(long, short, default_value_t = 1)]
        priority: u8,
        /// Parent task ID for sub-task decomposition.
        #[arg(long)]
        parent: Option<String>,
    },
    /// List tasks on the board.
    List {
        /// Filter by status (todo, running, done, failed, ...).
        #[arg(long, short)]
        status: Option<String>,
        /// Filter by task kind.
        #[arg(long, short)]
        kind: Option<String>,
        /// Max results.
        #[arg(long, short, default_value_t = 20)]
        limit: usize,
    },
    /// Show task details by ID.
    Show {
        /// Task ID (full UUID or first 8 chars).
        task_id: String,
    },
    /// Cancel a task.
    Cancel {
        /// Task ID.
        task_id: String,
        /// Cancellation reason.
        #[arg(long, short)]
        reason: Option<String>,
    },
    /// Show kanban-style board view.
    Board,
    /// Show task board statistics.
    Stats,
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
        CliCommand::Gateway => gateway::gateway_command().await,
        #[cfg(feature = "tape-viewer")]
        CliCommand::Tape { port, dir } => tape::tape_command(port, dir).await,
        CliCommand::Decisions { action } => match action {
            DecisionAction::List => decisions::list_command().await,
            DecisionAction::Remove { index } => decisions::remove_command(index).await,
            DecisionAction::Export => decisions::export_command().await,
        },
        CliCommand::Evolution { action } => match action {
            EvolutionAction::List { status } => {
                evolution::list_command(status.map(map_evolution_status)).await
            }
            EvolutionAction::Show { id } => evolution::show_command(id).await,
            EvolutionAction::CaptureRule {
                title,
                summary,
                content,
            } => evolution::capture_rule_command(title, summary, content).await,
            EvolutionAction::CaptureSkill {
                skill_name,
                title,
                description,
                content,
            } => evolution::capture_skill_command(skill_name, title, description, content).await,
            EvolutionAction::Promote { id, force } => evolution::promote_command(id, force).await,
            EvolutionAction::Reject { id } => evolution::reject_command(id).await,
        },
        CliCommand::Task { action } => {
            crate::taskboard::init_task_store(&crate::builtin::config::eli_home());
            match action {
                TaskAction::Add {
                    description,
                    kind,
                    priority,
                    parent,
                } => task::add_command(description, kind, priority, parent).await,
                TaskAction::List {
                    status,
                    kind,
                    limit,
                } => task::list_command(status, kind, limit).await,
                TaskAction::Show { task_id } => task::show_command(task_id).await,
                TaskAction::Cancel { task_id, reason } => {
                    task::cancel_command(task_id, reason).await
                }
                TaskAction::Board => task::board_command().await,
                TaskAction::Stats => task::stats_command().await,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Show registered hooks.
async fn hooks_command() {
    let (framework, _builtin) = builtin_framework().await;
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

/// Strip hallucinated `<function_calls>...</function_calls>` blocks from model output.
pub(crate) fn strip_fake_tool_calls(text: &str) -> String {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?s)<function_calls>.*?</function_calls>")
            .expect("SAFETY: regex is a static literal")
    });
    RE.replace_all(text, "").trim().to_owned()
}

async fn builtin_framework() -> (Arc<EliFramework>, Arc<BuiltinImpl>) {
    let builtin = Arc::new(BuiltinImpl::new());
    let framework = Arc::new(EliFramework::new());
    framework.register_plugin("builtin", builtin.clone()).await;
    (framework, builtin)
}

fn print_usage(usage: &crate::types::TurnUsageInfo) {
    if usage.total_tokens > 0 {
        eprintln!(
            "\x1b[2m[tokens: {} in + {} out = {}]\x1b[0m",
            usage.input_tokens, usage.output_tokens, usage.total_tokens,
        );
    }
}

fn map_evolution_status(status: EvolutionStatusArg) -> crate::evolution::CandidateStatus {
    match status {
        EvolutionStatusArg::Pending => crate::evolution::CandidateStatus::Pending,
        EvolutionStatusArg::Promoted => crate::evolution::CandidateStatus::Promoted,
        EvolutionStatusArg::Rejected => crate::evolution::CandidateStatus::Rejected,
    }
}
