//! Webhook channel — receives inbound messages via HTTP POST and sends
//! outbound messages by calling back to a configured URL.
//!
//! This channel acts as a generic HTTP bridge, allowing external services
//! (such as a Node.js sidecar hosting OpenClaw plugins) to send and receive
//! messages through eli's turn pipeline.

use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use super::base::Channel;
use super::message::ChannelMessage;

// ---------------------------------------------------------------------------
// WebhookSettings
// ---------------------------------------------------------------------------

/// Settings for the webhook channel, loaded from env vars prefixed
/// `ELI_WEBHOOK_`.
#[derive(Debug, Clone)]
pub struct WebhookSettings {
    /// Port to listen on for inbound messages.
    pub listen_port: u16,
    /// URL to POST outbound messages to (the sidecar's `/outbound` endpoint).
    pub callback_url: String,
}

impl WebhookSettings {
    /// Load from environment variables.
    pub fn from_env() -> Self {
        let listen_port = std::env::var("ELI_WEBHOOK_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3100);
        let callback_url = std::env::var("ELI_WEBHOOK_CALLBACK_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3101/outbound".to_owned());

        Self {
            listen_port,
            callback_url,
        }
    }

    /// Returns `true` if the webhook channel has been explicitly configured.
    pub fn is_configured(&self) -> bool {
        std::env::var("ELI_WEBHOOK_PORT").is_ok()
            || std::env::var("ELI_WEBHOOK_CALLBACK_URL").is_ok()
    }
}

// ---------------------------------------------------------------------------
// WebhookChannel
// ---------------------------------------------------------------------------

/// A channel that receives messages via HTTP POST and sends responses by
/// calling back to a configured URL.
pub struct WebhookChannel {
    settings: WebhookSettings,
    on_receive_tx: mpsc::UnboundedSender<ChannelMessage>,
    server_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    http_client: reqwest::Client,
}

impl WebhookChannel {
    pub fn new(
        on_receive_tx: mpsc::UnboundedSender<ChannelMessage>,
        settings: WebhookSettings,
    ) -> Self {
        Self {
            settings,
            on_receive_tx,
            server_handle: Mutex::new(None),
            http_client: reqwest::Client::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Inbound handler
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AppState {
    tx: mpsc::UnboundedSender<ChannelMessage>,
}

async fn handle_inbound(
    State(state): State<AppState>,
    Json(mut msg): Json<ChannelMessage>,
) -> StatusCode {
    if msg.channel.is_empty() {
        msg.channel = "webhook".to_owned();
    }
    if msg.output_channel.is_empty() {
        msg.output_channel = "webhook".to_owned();
    }
    if !msg.is_active {
        msg.is_active = true;
    }

    match state.tx.send(msg) {
        Ok(()) => StatusCode::OK,
        Err(e) => {
            error!(error = %e, "webhook: failed to enqueue inbound message");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

// ---------------------------------------------------------------------------
// Channel impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Channel for WebhookChannel {
    fn name(&self) -> &str {
        "webhook"
    }

    fn needs_debounce(&self) -> bool {
        false
    }

    async fn start(&self, cancel: CancellationToken) -> anyhow::Result<()> {
        let port = self.settings.listen_port;

        info!(port = port, callback = %self.settings.callback_url, "webhook.start");

        let state = AppState {
            tx: self.on_receive_tx.clone(),
        };

        let app = Router::new()
            .route("/inbound", post(handle_inbound))
            .with_state(state);

        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;

        let handle = tokio::spawn(async move {
            let server =
                axum::serve(listener, app).with_graceful_shutdown(cancel.cancelled_owned());

            if let Err(e) = server.await {
                error!(error = %e, "webhook server error");
            }
        });

        *self.server_handle.lock().await = Some(handle);

        info!(port = port, "webhook.start listening");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        if let Some(handle) = self.server_handle.lock().await.take() {
            handle.abort();
            let _ = handle.await;
        }
        info!("webhook.stopped");
        Ok(())
    }

    async fn send(&self, message: ChannelMessage) -> anyhow::Result<()> {
        let mut req = self
            .http_client
            .post(&self.settings.callback_url)
            .json(&message);
        if let Ok(token) = std::env::var("ELI_SIDECAR_TOKEN") {
            req = req.bearer_auth(&token);
        }
        let resp = req.send().await;

        match resp {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                error!(status = %status, body = %body, "webhook callback failed");
                anyhow::bail!("webhook callback returned {status}")
            }
            Err(e) => {
                error!(error = %e, "webhook callback request failed");
                anyhow::bail!("webhook callback error: {e}")
            }
        }
    }
}
