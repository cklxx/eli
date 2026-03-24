//! CLI channel — interactive REPL with colored terminal output.

use std::io::{self, BufRead, Write as IoWrite};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use crossterm::execute;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::base::Channel;
use super::message::{ChannelMessage, MessageKind};

// ---------------------------------------------------------------------------
// CliRenderer
// ---------------------------------------------------------------------------

/// Rich-like terminal renderer using crossterm for colored output.
pub struct CliRenderer;

impl CliRenderer {
    pub fn new() -> Self {
        Self
    }

    /// Print the welcome banner.
    pub fn welcome(&self, model: &str, workspace: &str) {
        let mut out = io::stdout();
        let border = "─".repeat(60);
        let _ = execute!(
            out,
            SetForegroundColor(Color::Cyan),
            Print(format!("┌{border}┐\n")),
            Print(format!("│{:^60}│\n", "Eli")),
            Print(format!("├{border}┤\n")),
            ResetColor,
            Print(format!("│ workspace: {:<47}│\n", workspace)),
            Print(format!("│ model: {:<51}│\n", model)),
            Print(format!("│ internal command prefix: '/'{:<31}│\n", "")),
            Print(format!(
                "│ shell command prefix: '/' at line start{:<18}│\n",
                ""
            )),
            Print(format!("│ type '/help' for command list{:<30}│\n", "")),
            SetForegroundColor(Color::Cyan),
            Print(format!("└{border}┘\n")),
            ResetColor,
        );
    }

    /// Print informational text in dim grey.
    pub fn info(&self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        let mut out = io::stdout();
        let _ = execute!(
            out,
            SetForegroundColor(Color::DarkGrey),
            Print(text),
            Print("\n"),
            ResetColor,
        );
    }

    /// Print command output inside a green bordered panel.
    pub fn command_output(&self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        self.panel(text, "Command", Color::Green);
    }

    /// Print assistant output inside a blue bordered panel.
    pub fn assistant_output(&self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        self.panel(text, "Assistant", Color::Blue);
    }

    /// Print error output inside a red bordered panel.
    pub fn error(&self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        self.panel(text, "Error", Color::Red);
    }

    fn panel(&self, text: &str, title: &str, color: Color) {
        let mut out = io::stdout();
        let width = 60usize;
        let border = "─".repeat(width);
        let title_line = format!("─ {title} ");
        let padding = if title_line.len() < width {
            "─".repeat(width - title_line.len())
        } else {
            String::new()
        };

        let _ = execute!(out, SetForegroundColor(color), Print("┌"),);
        let _ = execute!(out, Print(&title_line), Print(&padding), Print("┐\n"),);
        let _ = execute!(out, ResetColor,);

        for line in text.lines() {
            let _ = execute!(
                out,
                SetForegroundColor(color),
                Print("│ "),
                ResetColor,
                Print(line),
            );
            // Pad to the panel width.
            let visible_len = line.len();
            if visible_len + 2 < width {
                let pad = " ".repeat(width - 2 - visible_len);
                let _ = execute!(out, Print(&pad),);
            }
            let _ = execute!(out, SetForegroundColor(color), Print(" │\n"), ResetColor,);
        }
        let _ = execute!(
            out,
            SetForegroundColor(color),
            Print(format!("└{border}┘\n")),
            ResetColor,
        );
    }
}

impl Default for CliRenderer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// CliChannel
// ---------------------------------------------------------------------------

/// An interactive CLI channel that runs a REPL, reads user input, and
/// forwards it to the framework as [`ChannelMessage`]s.
pub struct CliChannel {
    /// Sender half passed in by the framework (typically `ChannelManager::on_receive`).
    on_receive_tx: mpsc::UnboundedSender<ChannelMessage>,
    renderer: CliRenderer,
    mode: Mutex<CliMode>,
    model: String,
    workspace: PathBuf,
    /// Handle for the spawned REPL task.
    repl_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Signaled after the framework finishes processing a request so the REPL
    /// can prompt again.
    request_done: Arc<Notify>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliMode {
    Agent,
    Shell,
}

impl CliChannel {
    pub fn new(
        on_receive_tx: mpsc::UnboundedSender<ChannelMessage>,
        model: impl Into<String>,
        workspace: impl Into<PathBuf>,
    ) -> Self {
        Self {
            on_receive_tx,
            renderer: CliRenderer::new(),
            mode: Mutex::new(CliMode::Agent),
            model: model.into(),
            workspace: workspace.into(),
            repl_handle: Mutex::new(None),
            request_done: Arc::new(Notify::new()),
        }
    }

    /// Notify the REPL that the latest request has been processed so it can
    /// prompt for the next input.
    pub fn notify_request_done(&self) {
        self.request_done.notify_one();
    }

    #[allow(dead_code)]
    fn session_id(&self) -> String {
        "cli_session".to_owned()
    }

    #[allow(dead_code)]
    fn chat_id(&self) -> String {
        "cli_chat".to_owned()
    }

    fn prompt_symbol(mode: CliMode) -> &'static str {
        match mode {
            CliMode::Agent => "> ",
            CliMode::Shell => "/ ",
        }
    }

    /// The directory name used in the prompt.
    fn cwd_name() -> String {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "?".to_owned())
    }

    /// Normalize user input based on the current mode.
    fn normalize_input(raw: &str, mode: CliMode) -> String {
        if mode != CliMode::Shell {
            return raw.to_owned();
        }
        if raw.starts_with('/') {
            return raw.to_owned();
        }
        format!("/{raw}")
    }
}

#[async_trait]
impl Channel for CliChannel {
    fn name(&self) -> &str {
        "cli"
    }

    async fn start(&self, cancel: CancellationToken) -> anyhow::Result<()> {
        let tx = self.on_receive_tx.clone();
        let renderer = CliRenderer::new();
        let model = self.model.clone();
        let workspace = self.workspace.clone();
        let mode_mutex = &self.mode;
        // We cannot move the mutex into the task, so we read the initial mode
        // and the spawned task manages its own copy.
        let initial_mode = *mode_mutex.lock().await;
        let request_done = Arc::clone(&self.request_done);

        renderer.welcome(&model, &workspace.display().to_string());

        let handle = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            let stdin = io::stdin();
            let mut reader = stdin.lock();
            let mut mode = initial_mode;

            loop {
                if cancel.is_cancelled() {
                    break;
                }

                let cwd = Self::cwd_name();
                let symbol = Self::prompt_symbol(mode);
                print!("{cwd} {symbol}");
                let _ = io::stdout().flush();

                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {}
                    Err(_) => break,
                }

                let raw = line.trim().to_owned();
                if raw.is_empty() {
                    continue;
                }

                // Built-in commands.
                match raw.as_str() {
                    "/quit" | "/exit" => break,
                    "/help" => {
                        let r = CliRenderer::new();
                        r.info("Commands:");
                        r.info("  /quit / /exit   — exit the REPL");
                        r.info("  /help           — show this help");
                        r.info("  Ctrl-X          — toggle agent/shell mode");
                        r.info("");
                        continue;
                    }
                    "/mode" => {
                        mode = match mode {
                            CliMode::Agent => CliMode::Shell,
                            CliMode::Shell => CliMode::Agent,
                        };
                        let r = CliRenderer::new();
                        r.info(&format!(
                            "Mode switched to {}",
                            match mode {
                                CliMode::Agent => "agent",
                                CliMode::Shell => "shell",
                            }
                        ));
                        continue;
                    }
                    _ => {}
                }

                let request = Self::normalize_input(&raw, mode);
                let message = ChannelMessage::new("cli_session", "cli", &request)
                    .with_chat_id("cli_chat")
                    .finalize();

                if tx.send(message).is_err() {
                    break;
                }

                // Wait for the framework to finish processing before prompting
                // again, unless cancelled.
                rt.block_on(async {
                    tokio::select! {
                        () = request_done.notified() => {}
                        () = cancel.cancelled() => {}
                    }
                });
            }

            let r = CliRenderer::new();
            r.info("Bye.");
            cancel.cancel();
        });

        *self.repl_handle.lock().await = Some(handle);
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        if let Some(handle) = self.repl_handle.lock().await.take() {
            handle.abort();
            let _ = handle.await;
        }
        info!("cli channel stopped");
        Ok(())
    }

    async fn send(&self, message: ChannelMessage) -> anyhow::Result<()> {
        match message.kind {
            MessageKind::Error => self.renderer.error(&message.content),
            MessageKind::Command => self.renderer.command_output(&message.content),
            MessageKind::Normal => self.renderer.assistant_output(&message.content),
        }
        // Signal the REPL that the response has been delivered.
        self.request_done.notify_one();
        Ok(())
    }

    fn needs_debounce(&self) -> bool {
        false
    }
}
