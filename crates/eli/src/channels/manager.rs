//! Channel manager — orchestrates all enabled channels, routes inbound and
//! outbound messages, and owns the lifecycle of in-flight processing tasks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::base::Channel;
use super::handler::BufferedMessageHandler;
use super::message::{ChannelMessage, MessageKind};
use crate::envelope::OutboundMessage;
use crate::types::Envelope;

// ---------------------------------------------------------------------------
// ChannelSettings
// ---------------------------------------------------------------------------

/// Runtime settings for the channel manager, typically loaded from environment
/// variables (prefix `ELI_`).
#[derive(Debug, Clone)]
pub struct ChannelSettings {
    /// Comma-separated list of enabled channels, or `"all"`.
    pub enabled_channels: String,
    /// Debounce seconds for buffered channels.
    pub debounce_seconds: f64,
    /// Maximum wait seconds before flushing follow-up messages.
    pub max_wait_seconds: f64,
    /// Time window (seconds) to consider a channel active.
    pub active_time_window: f64,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self {
            enabled_channels: "all".to_owned(),
            debounce_seconds: 1.0,
            max_wait_seconds: 10.0,
            active_time_window: 60.0,
        }
    }
}

impl ChannelSettings {
    /// Load settings from environment variables with the `ELI_` prefix.
    pub fn from_env() -> Self {
        Self {
            enabled_channels: std::env::var("ELI_ENABLED_CHANNELS")
                .unwrap_or_else(|_| "all".to_owned()),
            debounce_seconds: std::env::var("ELI_DEBOUNCE_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0),
            max_wait_seconds: std::env::var("ELI_MAX_WAIT_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10.0),
            active_time_window: std::env::var("ELI_ACTIVE_TIME_WINDOW")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60.0),
        }
    }

    fn enabled_set(&self) -> Vec<String> {
        self.enabled_channels
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// InboundProcessor  (trait object that the framework supplies)
// ---------------------------------------------------------------------------

/// Trait for the framework's inbound processing pipeline.
///
/// The channel manager calls `process_inbound` for every message that has been
/// through debounce / batching.
#[async_trait::async_trait]
pub trait InboundProcessor: Send + Sync {
    async fn process_inbound(&self, message: ChannelMessage) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------------------
// OutboundRouter  (the manager itself acts as one)
// ---------------------------------------------------------------------------

/// Trait mirroring the Python `OutboundChannelRouter` protocol.
#[async_trait::async_trait]
pub trait OutboundRouter: Send + Sync {
    /// Dispatch an outbound envelope to the appropriate channel.
    /// Returns `true` if a channel handled the message.
    async fn dispatch(&self, message: &Envelope) -> bool;

    /// Cancel all tasks for a given session.
    async fn quit(&self, session_id: &str);
}

// ---------------------------------------------------------------------------
// ChannelManager
// ---------------------------------------------------------------------------

/// Owns every registered [`Channel`], routes inbound messages through optional
/// debounce buffering, and dispatches outbound envelopes to the correct
/// channel.
pub struct ChannelManager {
    channels: HashMap<String, Arc<dyn Channel>>,
    settings: ChannelSettings,
    enabled_list: Vec<String>,

    /// Unbounded sender/receiver pair for the internal message queue.
    tx: mpsc::UnboundedSender<ChannelMessage>,
    rx: Mutex<mpsc::UnboundedReceiver<ChannelMessage>>,

    /// Per-session debounced handlers (only for channels with `needs_debounce`).
    session_handlers: Mutex<HashMap<String, Arc<BufferedMessageHandler>>>,

    /// Per-session in-flight task handles, so we can cancel on quit/shutdown.
    ongoing_tasks: Mutex<HashMap<String, HashSet<tokio::task::Id>>>,
    /// We also keep `JoinHandle`s keyed by task id for awaiting cancellation.
    task_handles: Mutex<HashMap<tokio::task::Id, tokio::task::JoinHandle<()>>>,
}

impl ChannelManager {
    /// Create a new manager from a map of channels.
    pub fn new(
        channels: HashMap<String, Arc<dyn Channel>>,
        settings: ChannelSettings,
        enabled_channels: Option<Vec<String>>,
    ) -> Arc<Self> {
        let enabled_list = enabled_channels.unwrap_or_else(|| settings.enabled_set());
        let (tx, rx) = mpsc::unbounded_channel();

        Arc::new(Self {
            channels,
            settings,
            enabled_list,
            tx,
            rx: Mutex::new(rx),
            session_handlers: Mutex::new(HashMap::new()),
            ongoing_tasks: Mutex::new(HashMap::new()),
            task_handles: Mutex::new(HashMap::new()),
        })
    }

    // ----- inbound ----------------------------------------------------------

    /// Called by channels when they receive a message from the user.
    pub async fn on_receive(&self, message: ChannelMessage) {
        let channel_name = message.channel.clone();
        let session_id = message.session_id.clone();

        if !self.channels.contains_key(&channel_name) {
            warn!(
                channel = %channel_name,
                "received message from unknown channel, ignoring"
            );
            return;
        }

        let needs_debounce = self
            .channels
            .get(&channel_name)
            .map(|c| c.needs_debounce())
            .unwrap_or(false);

        if needs_debounce {
            let handler = {
                let mut handlers = self.session_handlers.lock().await;
                handlers
                    .entry(session_id.clone())
                    .or_insert_with(|| {
                        Arc::new(BufferedMessageHandler::new(
                            self.tx.clone(),
                            self.settings.active_time_window,
                            self.settings.max_wait_seconds,
                            self.settings.debounce_seconds,
                        ))
                    })
                    .clone()
            };
            handler.handle(message).await;
        } else {
            let _ = self.tx.send(message);
        }
    }

    /// Get a reference to a channel by name.
    pub fn get_channel(&self, name: &str) -> Option<&Arc<dyn Channel>> {
        self.channels.get(name)
    }

    // ----- outbound ---------------------------------------------------------

    /// Dispatch an outbound envelope to the correct channel.
    pub async fn dispatch(&self, message: &Envelope) -> bool {
        // Use OutboundMessage for validated field extraction with logging.
        // We pass empty defaults; if neither output_channel nor channel is
        // present, the validated message's channel field will be empty and
        // the lookup below will fail gracefully (returning false).
        let validated = OutboundMessage::from_envelope(message, "", "");

        if validated.channel.is_empty() {
            return false;
        }

        let channel = match self.channels.get(&validated.channel) {
            Some(c) => Arc::clone(c),
            None => return false,
        };

        // Resolve session_id default that depends on the channel name.
        let session_id = if validated.session_id.is_empty() {
            format!("{}:default", validated.channel)
        } else {
            validated.session_id
        };

        let kind_str = message
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("normal");

        let kind = match kind_str {
            "error" => MessageKind::Error,
            "command" => MessageKind::Command,
            _ => MessageKind::Normal,
        };

        let outbound = ChannelMessage {
            session_id,
            channel: validated.channel,
            chat_id: validated.chat_id,
            content: validated.content,
            is_active: false,
            kind,
            context: validated.context,
            media: Vec::new(),
            output_channel: String::new(),
        };

        if let Err(e) = channel.send(outbound).await {
            error!(error = %e, "failed to send outbound message");
            return false;
        }
        true
    }

    /// Cancel all in-flight tasks for a session.
    pub async fn quit(&self, session_id: &str) {
        let task_ids = {
            let mut ongoing = self.ongoing_tasks.lock().await;
            ongoing.remove(session_id).unwrap_or_default()
        };

        let mut handles_guard = self.task_handles.lock().await;
        let mut cancelled = 0usize;
        for id in &task_ids {
            if let Some(handle) = handles_guard.remove(id) {
                handle.abort();
                let _ = handle.await;
                cancelled += 1;
            }
        }
        drop(handles_guard);

        info!(
            session_id = %session_id,
            cancelled = cancelled,
            "channel.manager quit session"
        );
    }

    // ----- enabled channels -------------------------------------------------

    /// Return the list of channels that should be started.
    pub fn enabled_channels(&self) -> Vec<Arc<dyn Channel>> {
        if self.enabled_list.iter().any(|s| s == "all") {
            // "all" excludes CLI to prevent interference.
            self.channels
                .iter()
                .filter(|(name, _)| name.as_str() != "cli")
                .map(|(_, ch)| Arc::clone(ch))
                .collect()
        } else {
            self.channels
                .iter()
                .filter(|(name, _)| self.enabled_list.contains(name))
                .map(|(_, ch)| Arc::clone(ch))
                .collect()
        }
    }

    // ----- main loop --------------------------------------------------------

    /// Start all enabled channels and process the message queue until the
    /// cancellation token fires.
    pub async fn listen_and_run(
        self: &Arc<Self>,
        processor: Arc<dyn InboundProcessor>,
        cancel: CancellationToken,
    ) -> anyhow::Result<()> {
        let enabled = self.enabled_channels();
        for ch in &enabled {
            ch.start(cancel.clone()).await?;
        }
        info!("channel.manager started listening");

        let result = self.run_loop(processor, cancel.clone()).await;

        self.shutdown().await;
        info!("channel.manager stopped");
        result
    }

    async fn run_loop(
        self: &Arc<Self>,
        processor: Arc<dyn InboundProcessor>,
        cancel: CancellationToken,
    ) -> anyhow::Result<()> {
        let mut rx = self.rx.lock().await;
        loop {
            let message = tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(m) => m,
                        None => break,
                    }
                }
                () = cancel.cancelled() => {
                    info!("channel.manager received shutdown signal");
                    break;
                }
            };

            let session_id = message.session_id.clone();
            let proc = Arc::clone(&processor);
            let mgr = Arc::clone(self);

            let handle = tokio::spawn(async move {
                if let Err(e) = proc.process_inbound(message).await {
                    error!(error = %e, "channel.manager process_inbound error");
                }
            });

            let task_id = handle.id();
            {
                let mut ongoing = self.ongoing_tasks.lock().await;
                ongoing
                    .entry(session_id.clone())
                    .or_default()
                    .insert(task_id);
            }
            {
                let mut handles = self.task_handles.lock().await;
                handles.insert(task_id, handle);
            }

            // Spawn a cleanup task that removes the handle once done.
            let task_id_clean = task_id;
            let session_id_clean = session_id;
            tokio::spawn(async move {
                // Wait for the handle to be inserted, then poll until it completes.
                loop {
                    let maybe_handle = {
                        let handles = mgr.task_handles.lock().await;
                        handles.contains_key(&task_id_clean)
                    };
                    if !maybe_handle {
                        break;
                    }
                    // Check if the task is finished by trying to await it.
                    tokio::task::yield_now().await;
                    let finished = {
                        let handles = mgr.task_handles.lock().await;
                        match handles.get(&task_id_clean) {
                            Some(h) => h.is_finished(),
                            None => true,
                        }
                    };
                    if finished {
                        let handle = {
                            let mut handles = mgr.task_handles.lock().await;
                            handles.remove(&task_id_clean)
                        };
                        if let Some(h) = handle {
                            let _ = h.await;
                        }
                        let mut ongoing = mgr.ongoing_tasks.lock().await;
                        if let Some(set) = ongoing.get_mut(&session_id_clean) {
                            set.remove(&task_id_clean);
                            if set.is_empty() {
                                ongoing.remove(&session_id_clean);
                            }
                        }
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            });
        }

        Ok(())
    }

    // ----- shutdown ---------------------------------------------------------

    /// Cancel all in-flight tasks and stop every enabled channel.
    pub async fn shutdown(&self) {
        let mut count = 0usize;

        let all_sessions: Vec<String> = {
            let ongoing = self.ongoing_tasks.lock().await;
            ongoing.keys().cloned().collect()
        };

        for session_id in all_sessions {
            let task_ids = {
                let mut ongoing = self.ongoing_tasks.lock().await;
                ongoing.remove(&session_id).unwrap_or_default()
            };
            let mut handles = self.task_handles.lock().await;
            for id in task_ids {
                if let Some(handle) = handles.remove(&id) {
                    handle.abort();
                    let _ = handle.await;
                    count += 1;
                }
            }
        }

        info!(
            cancelled = count,
            "channel.manager cancelled in-flight tasks"
        );

        for ch in self.enabled_channels() {
            if let Err(e) = ch.stop().await {
                error!(channel = %ch.name(), error = %e, "error stopping channel");
            }
        }
    }
}
