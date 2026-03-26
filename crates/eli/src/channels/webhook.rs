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
use base64::Engine;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::base::Channel;
use super::message::{ChannelMessage, DataFetcher, MediaItem, MediaType};

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

#[derive(Debug, Deserialize)]
struct InboundWebhookMessage {
    #[serde(flatten)]
    message: ChannelMessage,
    #[serde(default)]
    media: Vec<InboundMediaPayload>,
}

#[derive(Debug, Deserialize)]
struct InboundMediaPayload {
    #[serde(default)]
    media_type: String,
    #[serde(default)]
    mime_type: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    data_base64: Option<String>,
}

impl InboundWebhookMessage {
    fn into_message(self) -> ChannelMessage {
        let mut message = self.message;
        message.media = restore_inbound_media(self.media);
        apply_inbound_defaults(&mut message);
        message
    }
}

fn restore_inbound_media(media: Vec<InboundMediaPayload>) -> Vec<MediaItem> {
    media
        .into_iter()
        .filter_map(restore_inbound_media_item)
        .collect()
}

fn restore_inbound_media_item(item: InboundMediaPayload) -> Option<MediaItem> {
    let media_type = parse_media_type(&item.media_type)?;
    let data_fetcher = build_inbound_fetcher(&item)?;
    Some(MediaItem {
        media_type,
        mime_type: inbound_mime_type(&item, media_type),
        filename: item.filename.or(item.path.clone()),
        data_fetcher: Some(data_fetcher),
    })
}

fn build_inbound_fetcher(item: &InboundMediaPayload) -> Option<DataFetcher> {
    item.data_base64
        .as_deref()
        .filter(|data| !data.is_empty())
        .and_then(base64_data_fetcher)
        .or_else(|| {
            item.path
                .as_deref()
                .filter(|path| !path.is_empty())
                .map(path_data_fetcher)
        })
}

fn base64_data_fetcher(data: &str) -> Option<DataFetcher> {
    let bytes = decode_base64_data(data)?;
    Some(Arc::new(move || {
        let bytes = bytes.clone();
        Box::pin(async move { bytes })
    }))
}

fn decode_base64_data(data: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| warn!(%error, "webhook: invalid inbound media base64"))
        .ok()
}

fn path_data_fetcher(path: &str) -> DataFetcher {
    let path = path.to_owned();
    Arc::new(move || {
        let path = path.clone();
        Box::pin(async move { tokio::fs::read(path).await.unwrap_or_default() })
    })
}

fn parse_media_type(value: &str) -> Option<MediaType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "image" | "img" | "photo" | "picture" | "sticker" => Some(MediaType::Image),
        "audio" | "voice" => Some(MediaType::Audio),
        "video" => Some(MediaType::Video),
        "document" | "doc" | "file" => Some(MediaType::Document),
        other => unsupported_media_type(other),
    }
}

fn unsupported_media_type(value: &str) -> Option<MediaType> {
    if !value.is_empty() {
        warn!(
            media_type = value,
            "webhook: unsupported inbound media type"
        );
    }
    None
}

fn inbound_mime_type(item: &InboundMediaPayload, media_type: MediaType) -> String {
    if !item.mime_type.is_empty() {
        return item.mime_type.clone();
    }
    default_mime_type(media_type).to_owned()
}

fn default_mime_type(media_type: MediaType) -> &'static str {
    match media_type {
        MediaType::Image => "image/jpeg",
        MediaType::Audio => "audio/mpeg",
        MediaType::Video => "video/mp4",
        MediaType::Document => "application/octet-stream",
    }
}

fn apply_inbound_defaults(message: &mut ChannelMessage) {
    if message.channel.is_empty() {
        message.channel = "webhook".to_owned();
    }
    if message.output_channel.is_empty() {
        message.output_channel = "webhook".to_owned();
    }
    if !message.is_active {
        message.is_active = true;
    }
}

fn enqueue_inbound(
    tx: &mpsc::UnboundedSender<ChannelMessage>,
    message: ChannelMessage,
) -> StatusCode {
    match tx.send(message) {
        Ok(()) => StatusCode::OK,
        Err(error) => {
            error!(%error, "webhook: failed to enqueue inbound message");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn handle_inbound(
    State(state): State<AppState>,
    Json(payload): Json<InboundWebhookMessage>,
) -> StatusCode {
    enqueue_inbound(&state.tx, payload.into_message())
}

// ---------------------------------------------------------------------------
// Channel impl
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) fn build_webhook_payload(message: &ChannelMessage) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "session_id": message.session_id,
        "channel": message.channel,
        "content": message.content,
        "chat_id": message.chat_id,
        "is_active": message.is_active,
        "kind": message.kind,
        "context": message.context,
        "output_channel": message.output_channel,
    });

    let mut media_json = Vec::new();

    for item in &message.media {
        if let Some(path) = item.filename.as_ref() {
            media_json.push(serde_json::json!({
                "path": path,
                "media_type": item.media_type,
                "mime_type": item.mime_type,
            }));
        }
    }

    if media_json.is_empty()
        && let Some(outbound) = message
            .context
            .get("outbound_media")
            .and_then(|v| v.as_array())
    {
        media_json = outbound.clone();
    }

    if !media_json.is_empty() {
        payload["media"] = serde_json::Value::Array(media_json);
    }

    payload
}

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
        let media = webhook_media_payload(&message);
        let mut payload = serde_json::to_value(&message)?;
        if let Some(media) = media {
            payload["media"] = media;
        }

        let mut req = self
            .http_client
            .post(&self.settings.callback_url)
            .json(&payload);
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

fn webhook_media_payload(message: &ChannelMessage) -> Option<serde_json::Value> {
    let mut items = Vec::new();

    if let Some(outbound) = message
        .context
        .get("outbound_media")
        .and_then(|v| v.as_array())
    {
        for item in outbound {
            let Some(path) = item.get("path").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(filename) = std::path::Path::new(path)
                .file_name()
                .and_then(|v| v.to_str())
            else {
                continue;
            };
            let media_type = item
                .get("media_type")
                .and_then(|v| v.as_str())
                .unwrap_or("document");
            let mime_type = item
                .get("mime_type")
                .and_then(|v| v.as_str())
                .unwrap_or("application/octet-stream");
            items.push(serde_json::json!({
                "path": path,
                "filename": filename,
                "media_type": media_type,
                "mime_type": mime_type,
            }));
        }
    }

    if items.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(items))
    }
}

#[cfg(test)]
mod webhook_payload_tests {
    use base64::Engine;
    use serde_json::json;

    use super::{InboundWebhookMessage, build_webhook_payload};
    use crate::channels::message::{ChannelMessage, MediaItem, MediaType};

    #[test]
    fn webhook_payload_includes_context_outbound_media_as_media() {
        let mut msg = ChannelMessage::new("s1", "webhook", "hello").with_chat_id("42");
        msg.context.insert(
            "outbound_media".into(),
            json!([{
                "path": "/tmp/a.png",
                "media_type": "image",
                "mime_type": "image/png"
            }]),
        );

        let payload = build_webhook_payload(&msg);
        assert_eq!(payload["media"][0]["path"], "/tmp/a.png");
        assert_eq!(payload["media"][0]["media_type"], "image");
        assert_eq!(payload["media"][0]["mime_type"], "image/png");
    }

    #[test]
    fn webhook_payload_includes_structured_message_media() {
        let mut msg = ChannelMessage::new("s1", "webhook", "hello").with_chat_id("42");
        msg = msg.with_media(vec![MediaItem {
            media_type: MediaType::Image,
            mime_type: "image/jpeg".into(),
            filename: Some("/tmp/b.jpg".into()),
            data_fetcher: None,
        }]);

        let payload = build_webhook_payload(&msg);
        assert_eq!(payload["media"][0]["path"], "/tmp/b.jpg");
        assert_eq!(payload["media"][0]["media_type"], "image");
        assert_eq!(payload["media"][0]["mime_type"], "image/jpeg");
    }

    fn parse_inbound_payload(payload: serde_json::Value) -> ChannelMessage {
        serde_json::from_value::<InboundWebhookMessage>(payload)
            .unwrap()
            .into_message()
    }

    fn inbound_message_payload(media: serde_json::Value) -> serde_json::Value {
        json!({
            "session_id": "s1",
            "channel": "webhook",
            "content": "hello",
            "chat_id": "42",
            "is_active": true,
            "kind": "normal",
            "context": {},
            "output_channel": "webhook",
            "media": media
        })
    }

    fn defaulted_message_payload(media: serde_json::Value) -> serde_json::Value {
        json!({
            "session_id": "s1",
            "channel": "",
            "content": "hello",
            "chat_id": "42",
            "is_active": false,
            "kind": "normal",
            "context": {},
            "output_channel": "",
            "media": media
        })
    }

    fn base64_image_payload(data_base64: String) -> serde_json::Value {
        json!({
            "media_type": "image",
            "mime_type": "image/png",
            "filename": "inline.png",
            "data_base64": data_base64
        })
    }

    fn path_image_payload(path: &str) -> serde_json::Value {
        json!({
            "media_type": "image",
            "mime_type": "image/png",
            "path": path
        })
    }

    async fn fetch_media_bytes(message: &ChannelMessage, index: usize) -> Vec<u8> {
        message.media[index].data_fetcher.as_ref().unwrap()().await
    }

    #[tokio::test]
    async fn test_restore_inbound_media_builds_base64_and_path_fetchers() {
        let tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
        let path = tmp.path().to_string_lossy().to_string();
        let path_bytes = vec![9_u8, 8, 7];
        std::fs::write(tmp.path(), &path_bytes).unwrap();
        let inline_bytes = vec![1_u8, 2, 3, 4];
        let inline_b64 = base64::engine::general_purpose::STANDARD.encode(&inline_bytes);
        let media = json!([base64_image_payload(inline_b64), path_image_payload(&path)]);
        let message = parse_inbound_payload(inbound_message_payload(media));
        let inline = fetch_media_bytes(&message, 0).await;
        let from_path = fetch_media_bytes(&message, 1).await;
        assert_eq!(message.media.len(), 2);
        assert_eq!(message.media[0].media_type, MediaType::Image);
        assert_eq!(message.media[0].filename.as_deref(), Some("inline.png"));
        assert_eq!(inline, inline_bytes);
        assert_eq!(from_path, path_bytes);
    }

    #[test]
    fn test_restore_inbound_media_applies_defaults_and_ignores_unsupported_types() {
        let media = json!([
            { "media_type": "binary", "path": "/tmp/ignored.bin" },
            { "media_type": "file", "path": "/tmp/doc.pdf" }
        ]);
        let message = parse_inbound_payload(defaulted_message_payload(media));
        assert_eq!(message.channel, "webhook");
        assert_eq!(message.output_channel, "webhook");
        assert!(message.is_active);
        assert_eq!(message.media.len(), 1);
        assert_eq!(message.media[0].media_type, MediaType::Document);
    }
}
