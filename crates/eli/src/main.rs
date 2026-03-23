//! Eli agent framework — Rust port.
//!
//! Entry point: parses CLI args with clap, initialises the framework, and
//! dispatches to the appropriate subcommand.

use clap::Parser;
use tracing_subscriber::EnvFilter;

use eli::builtin::cli::{CliCommand, execute};

/// Eli — a developer-first AI agent framework.
#[derive(Parser, Debug)]
#[command(name = "eli", version, about = "Eli agent framework")]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialise tracing (respects RUST_LOG env var).
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    execute(cli.command).await
}
