//! Eli agent framework — Rust port.
//!
//! Entry point: parses CLI args with clap, initialises the framework, and
//! dispatches to the appropriate subcommand.

use clap::Parser;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use eli::builtin::cli::{CliCommand, execute};

/// Eli — a developer-first AI agent framework.
#[derive(Parser, Debug)]
#[command(name = "eli", version, about = "Eli agent framework")]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

fn eli_home() -> PathBuf {
    std::env::var("ELI_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".eli")))
        .unwrap_or_else(|| PathBuf::from(".eli"))
}

fn init_tracing() -> anyhow::Result<()> {
    let console_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("eli_trace=off".parse()?);

    let trace_enabled = std::env::var("ELI_TRACE")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

    if trace_enabled {
        let trace_log_dir = eli_home().join("logs");
        fs::create_dir_all(&trace_log_dir)?;
        let trace_log_path = trace_log_dir.join("eli-trace.log");
        let trace_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&trace_log_path)?;

        let console_layer = tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_filter(console_filter);

        let trace_file_for_writer = std::sync::Arc::new(std::sync::Mutex::new(trace_file));
        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_target(false)
            .with_writer(move || ArcMutexWriter(trace_file_for_writer.clone()))
            .with_filter(filter_fn(|metadata| metadata.target() == "eli_trace"));

        tracing_subscriber::registry()
            .with(console_layer)
            .with(file_layer)
            .init();

        tracing::info!(trace_log = %trace_log_path.display(), "eli trace log enabled");
    } else {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        // Always suppress eli_trace on console — it contains full LLM payloads.
        let filter = filter.add_directive("eli_trace=off".parse().expect("valid directive"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .init();
    }

    Ok(())
}

/// Thread-safe writer that wraps an `Arc<Mutex<File>>`, avoiding
/// `File::try_clone` (which can panic under fd pressure).
struct ArcMutexWriter(std::sync::Arc<std::sync::Mutex<std::fs::File>>);

impl std::io::Write for ArcMutexWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut f = self.0.lock().unwrap();
        f.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let mut f = self.0.lock().unwrap();
        f.flush()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing()?;

    let cli = Cli::parse();
    execute(cli.command).await
}
