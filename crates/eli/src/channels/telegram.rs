//! Telegram channel — receives and sends messages via the Telegram Bot API
//! using the `teloxide` crate.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use teloxide::dispatching::{Dispatcher, UpdateFilterExt};
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{ChatKind, MediaKind, MessageKind as TgMessageKind, ParseMode, Update};
use teloxide::update_listeners;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::base::Channel;
use super::message::{ChannelMessage, DataFetcher, MediaItem, MediaType};

// ---------------------------------------------------------------------------
// TelegramSettings
// ---------------------------------------------------------------------------

/// Settings for the Telegram channel, loaded from env vars prefixed
/// `ELI_TELEGRAM_`.
#[derive(Debug, Clone)]
pub struct TelegramSettings {
    /// Bot token.
    pub token: String,
    /// Comma-separated allowed user IDs (empty = no restriction).
    pub allow_users: HashSet<String>,
    /// Comma-separated allowed chat IDs (empty = no restriction).
    pub allow_chats: HashSet<String>,
    /// Optional HTTP/SOCKS5 proxy URL.
    pub proxy: Option<String>,
}

impl TelegramSettings {
    /// Load from environment variables.
    pub fn from_env() -> Self {
        let token = std::env::var("ELI_TELEGRAM_TOKEN").unwrap_or_default();
        let allow_users = std::env::var("ELI_TELEGRAM_ALLOW_USERS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        let allow_chats = std::env::var("ELI_TELEGRAM_ALLOW_CHATS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        let proxy = std::env::var("ELI_TELEGRAM_PROXY")
            .ok()
            .filter(|s| !s.is_empty());

        Self {
            token,
            allow_users,
            allow_chats,
            proxy,
        }
    }
}

/// Message returned when a user has no access.
const NO_ACCESS_MESSAGE: &str =
    "You are not allowed to chat with me. Please deploy your own instance of Eli.";

// ---------------------------------------------------------------------------
// Message type detection
// ---------------------------------------------------------------------------

/// Detect the high-level message type from a teloxide `Message`.
fn detect_message_type(msg: &Message) -> &'static str {
    match &msg.kind {
        TgMessageKind::Common(common) => match &common.media_kind {
            MediaKind::Text(_) => "text",
            MediaKind::Photo(_) => "photo",
            MediaKind::Audio(_) => "audio",
            MediaKind::Video(_) => "video",
            MediaKind::Voice(_) => "voice",
            MediaKind::Document(_) => "document",
            MediaKind::Sticker(_) => "sticker",
            MediaKind::VideoNote(_) => "video_note",
            _ => "unknown",
        },
        _ => "unknown",
    }
}

/// Map telegram message types to our [`MediaType`].
fn msg_type_to_media_type(msg_type: &str) -> MediaType {
    match msg_type {
        "photo" | "sticker" => MediaType::Image,
        "audio" | "voice" => MediaType::Audio,
        "video" | "video_note" => MediaType::Video,
        "document" => MediaType::Document,
        _ => MediaType::Document,
    }
}

// ---------------------------------------------------------------------------
// EliMessageFilter logic
// ---------------------------------------------------------------------------

/// Determine whether a message should be processed (mirrors the Python
/// `EliMessageFilter.filter` logic).
fn should_process_message(msg: &Message, bot_id: UserId, bot_username: &str) -> bool {
    let msg_type = detect_message_type(msg);
    if msg_type == "unknown" {
        return false;
    }

    let chat_type = &msg.chat.kind;
    let is_private = matches!(chat_type, ChatKind::Private(_));
    let is_group = matches!(chat_type, ChatKind::Public(_));

    if is_private {
        return true;
    }

    if is_group {
        let content = extract_text_content(msg).unwrap_or_default().to_lowercase();
        let bot_user_lower = bot_username.to_lowercase();

        let mentions_bot = content.contains("eli")
            || (!bot_user_lower.is_empty() && content.contains(&format!("@{bot_user_lower}")));

        let reply_to_bot = is_reply_to_bot(msg, bot_id);

        // Non-text media without caption: only process if replying to bot.
        if msg_type != "text" && extract_caption(msg).is_none() {
            return reply_to_bot;
        }

        return mentions_bot || reply_to_bot;
    }

    false
}

/// Check whether the message is a reply to the bot.
fn is_reply_to_bot(msg: &Message, bot_id: UserId) -> bool {
    match msg.reply_to_message() {
        Some(reply) => reply.from.as_ref().is_some_and(|u| u.id == bot_id),
        None => false,
    }
}

/// Extract text content from a message.
fn extract_text_content(msg: &Message) -> Option<&str> {
    msg.text().or_else(|| msg.caption())
}

/// Extract caption from a message (photos, videos, documents, etc.).
fn extract_caption(msg: &Message) -> Option<&str> {
    msg.caption()
}

// ---------------------------------------------------------------------------
// Media extraction helpers
// ---------------------------------------------------------------------------

/// Build a [`MediaItem`] from a teloxide `Message`, if it contains media.
fn extract_media_item(msg: &Message, bot: Bot) -> Option<MediaItem> {
    let msg_type = detect_message_type(msg);
    let _media_type = msg_type_to_media_type(msg_type);

    match &msg.kind {
        TgMessageKind::Common(common) => match &common.media_kind {
            MediaKind::Photo(photo) => {
                let largest = photo.photo.last()?;
                let file_id = largest.file.id.clone();
                let bot_clone = bot.clone();
                let fetcher: DataFetcher = Arc::new(move || {
                    let bot = bot_clone.clone();
                    let fid = file_id.clone();
                    Box::pin(async move { download_file(&bot, &fid).await })
                });
                Some(MediaItem {
                    media_type: MediaType::Image,
                    mime_type: "image/jpeg".to_owned(),
                    filename: None,
                    data_fetcher: Some(fetcher),
                })
            }
            MediaKind::Audio(audio) => {
                let file_id = audio.audio.file.id.clone();
                let mime = audio
                    .audio
                    .mime_type
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "audio/mpeg".to_owned());
                let fname = audio.audio.file_name.clone();
                let bot_clone = bot.clone();
                let fetcher: DataFetcher = Arc::new(move || {
                    let bot = bot_clone.clone();
                    let fid = file_id.clone();
                    Box::pin(async move { download_file(&bot, &fid).await })
                });
                Some(MediaItem {
                    media_type: MediaType::Audio,
                    mime_type: mime,
                    filename: fname,
                    data_fetcher: Some(fetcher),
                })
            }
            MediaKind::Voice(voice) => {
                let file_id = voice.voice.file.id.clone();
                let mime = voice
                    .voice
                    .mime_type
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "audio/ogg".to_owned());
                let bot_clone = bot.clone();
                let fetcher: DataFetcher = Arc::new(move || {
                    let bot = bot_clone.clone();
                    let fid = file_id.clone();
                    Box::pin(async move { download_file(&bot, &fid).await })
                });
                Some(MediaItem {
                    media_type: MediaType::Audio,
                    mime_type: mime,
                    filename: None,
                    data_fetcher: Some(fetcher),
                })
            }
            MediaKind::Video(video) => {
                let file_id = video.video.file.id.clone();
                let mime = video
                    .video
                    .mime_type
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "video/mp4".to_owned());
                let fname = video.video.file_name.clone();
                let bot_clone = bot.clone();
                let fetcher: DataFetcher = Arc::new(move || {
                    let bot = bot_clone.clone();
                    let fid = file_id.clone();
                    Box::pin(async move { download_file(&bot, &fid).await })
                });
                Some(MediaItem {
                    media_type: MediaType::Video,
                    mime_type: mime,
                    filename: fname,
                    data_fetcher: Some(fetcher),
                })
            }
            MediaKind::VideoNote(vn) => {
                let file_id = vn.video_note.file.id.clone();
                let bot_clone = bot.clone();
                let fetcher: DataFetcher = Arc::new(move || {
                    let bot = bot_clone.clone();
                    let fid = file_id.clone();
                    Box::pin(async move { download_file(&bot, &fid).await })
                });
                Some(MediaItem {
                    media_type: MediaType::Video,
                    mime_type: "video/mp4".to_owned(),
                    filename: None,
                    data_fetcher: Some(fetcher),
                })
            }
            MediaKind::Document(doc) => {
                let file_id = doc.document.file.id.clone();
                let mime = doc
                    .document
                    .mime_type
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "application/octet-stream".to_owned());
                let fname = doc.document.file_name.clone();
                let bot_clone = bot.clone();
                let fetcher: DataFetcher = Arc::new(move || {
                    let bot = bot_clone.clone();
                    let fid = file_id.clone();
                    Box::pin(async move { download_file(&bot, &fid).await })
                });
                Some(MediaItem {
                    media_type: MediaType::Document,
                    mime_type: mime,
                    filename: fname,
                    data_fetcher: Some(fetcher),
                })
            }
            MediaKind::Sticker(sticker) => {
                let file_id = sticker.sticker.file.id.clone();
                let mime = if sticker.sticker.is_animated() {
                    "video/webm"
                } else {
                    "image/webp"
                }
                .to_owned();
                let bot_clone = bot.clone();
                let fetcher: DataFetcher = Arc::new(move || {
                    let bot = bot_clone.clone();
                    let fid = file_id.clone();
                    Box::pin(async move { download_file(&bot, &fid).await })
                });
                Some(MediaItem {
                    media_type: MediaType::Image,
                    mime_type: mime,
                    filename: None,
                    data_fetcher: Some(fetcher),
                })
            }
            _ => None,
        },
        _ => None,
    }
}

/// Download a file from Telegram by file ID. Returns an empty Vec on failure.
async fn download_file(bot: &Bot, file_id: &str) -> Vec<u8> {
    match bot.get_file(file_id).await {
        Ok(file) => {
            use futures::StreamExt as _;
            let stream = bot.download_file_stream(&file.path);
            let mut buf = Vec::new();
            let mut stream = std::pin::pin!(stream);
            let mut download_err = None;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => buf.extend_from_slice(&bytes),
                    Err(e) => {
                        download_err = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = download_err {
                error!(error = %e, file_id = %file_id, "failed to download telegram file");
                Vec::new()
            } else {
                buf
            }
        }
        Err(e) => {
            error!(error = %e, file_id = %file_id, "failed to get telegram file info");
            Vec::new()
        }
    }
}

/// Build a human-readable content string from the message.
fn format_message_content(msg: &Message) -> String {
    let msg_type = detect_message_type(msg);
    let caption = extract_caption(msg).unwrap_or("");

    match msg_type {
        "text" => msg.text().unwrap_or("").to_owned(),
        "photo" => {
            if caption.is_empty() {
                "[Photo message]".to_owned()
            } else {
                format!("[Photo message] Caption: {caption}")
            }
        }
        "audio" => {
            if let TgMessageKind::Common(common) = &msg.kind
                && let MediaKind::Audio(audio) = &common.media_kind
            {
                let title = audio.audio.title.as_deref().unwrap_or("Unknown");
                let performer = audio.audio.performer.as_deref().unwrap_or("");
                let duration = audio.audio.duration.seconds();
                if performer.is_empty() {
                    return format!("[Audio: {title} ({duration}s)]");
                }
                return format!("[Audio: {performer} - {title} ({duration}s)]");
            }
            "[Audio]".to_owned()
        }
        "sticker" => {
            if let TgMessageKind::Common(common) = &msg.kind
                && let MediaKind::Sticker(sticker) = &common.media_kind
            {
                let emoji = sticker.sticker.emoji.as_deref().unwrap_or("");
                let set_name = sticker.sticker.set_name.as_deref().unwrap_or("");
                if !emoji.is_empty() {
                    return format!("[Sticker: {emoji} from {set_name}]");
                }
                return format!("[Sticker from {set_name}]");
            }
            "[Sticker]".to_owned()
        }
        "video" => {
            let duration = if let TgMessageKind::Common(common) = &msg.kind {
                if let MediaKind::Video(v) = &common.media_kind {
                    v.video.duration.seconds()
                } else {
                    0
                }
            } else {
                0
            };
            let base = format!("[Video: {duration}s]");
            if caption.is_empty() {
                base
            } else {
                format!("{base} Caption: {caption}")
            }
        }
        "voice" => {
            let duration = if let TgMessageKind::Common(common) = &msg.kind {
                if let MediaKind::Voice(v) = &common.media_kind {
                    v.voice.duration.seconds()
                } else {
                    0
                }
            } else {
                0
            };
            format!("[Voice message: {duration}s]")
        }
        "document" => {
            let (file_name, mime_type) = if let TgMessageKind::Common(common) = &msg.kind {
                if let MediaKind::Document(d) = &common.media_kind {
                    (
                        d.document.file_name.as_deref().unwrap_or("unknown"),
                        d.document
                            .mime_type
                            .as_ref()
                            .map(|m| m.to_string())
                            .unwrap_or_else(|| "application/octet-stream".to_owned()),
                    )
                } else {
                    ("unknown", "application/octet-stream".to_owned())
                }
            } else {
                ("unknown", "application/octet-stream".to_owned())
            };
            let base = format!("[Document: {file_name} ({mime_type})]");
            if caption.is_empty() {
                base
            } else {
                format!("{base} Caption: {caption}")
            }
        }
        "video_note" => {
            let duration = if let TgMessageKind::Common(common) = &msg.kind {
                if let MediaKind::VideoNote(vn) = &common.media_kind {
                    vn.video_note.duration.seconds()
                } else {
                    0
                }
            } else {
                0
            };
            format!("[Video note: {duration}s]")
        }
        _ => format!("[Unsupported message type: {msg_type}]"),
    }
}

/// Extract links from message entities.
fn extract_links(msg: &Message) -> Vec<String> {
    let mut links = Vec::new();

    let (entities, source_text) = if let Some(text) = msg.text() {
        (
            msg.entities().map(|e| e.to_vec()).unwrap_or_default(),
            text.to_owned(),
        )
    } else if let Some(caption) = msg.caption() {
        (
            msg.caption_entities()
                .map(|e| e.to_vec())
                .unwrap_or_default(),
            caption.to_owned(),
        )
    } else {
        return links;
    };

    for entity in &entities {
        match entity.kind {
            teloxide::types::MessageEntityKind::TextLink { ref url } => {
                let url_str = url.as_str().to_owned();
                if !links.contains(&url_str) {
                    links.push(url_str);
                }
            }
            teloxide::types::MessageEntityKind::Url => {
                let offset = entity.offset;
                let length = entity.length;
                let candidate: String = source_text
                    .chars()
                    .skip(offset)
                    .take(length)
                    .collect::<String>()
                    .trim()
                    .to_owned();
                if !candidate.is_empty() && !links.contains(&candidate) {
                    links.push(candidate);
                }
            }
            _ => {}
        }
    }
    links
}

// ---------------------------------------------------------------------------
// TelegramChannel
// ---------------------------------------------------------------------------

/// A channel that connects to Telegram via a bot token and the teloxide
/// polling dispatcher.
pub struct TelegramChannel {
    settings: TelegramSettings,
    on_receive_tx: mpsc::UnboundedSender<ChannelMessage>,
    bot: RwLock<Option<Bot>>,
    dispatcher_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl TelegramChannel {
    pub fn new(
        on_receive_tx: mpsc::UnboundedSender<ChannelMessage>,
        settings: TelegramSettings,
    ) -> Self {
        Self {
            settings,
            on_receive_tx,
            bot: RwLock::new(None),
            dispatcher_handle: Mutex::new(None),
        }
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    fn needs_debounce(&self) -> bool {
        true
    }

    async fn start(&self, cancel: CancellationToken) -> anyhow::Result<()> {
        let token = &self.settings.token;
        if token.is_empty() {
            anyhow::bail!("ELI_TELEGRAM_TOKEN is not set");
        }

        info!(
            allow_users = self.settings.allow_users.len(),
            allow_chats = self.settings.allow_chats.len(),
            proxy = ?self.settings.proxy,
            "telegram.start"
        );

        let bot = Bot::new(token);
        *self.bot.write().await = Some(bot.clone());

        let tx = self.on_receive_tx.clone();
        let allow_users = self.settings.allow_users.clone();
        let allow_chats = self.settings.allow_chats.clone();

        let bot_for_handler = bot.clone();

        let handle = tokio::spawn(async move {
            let handler = Update::filter_message().endpoint(move |bot: Bot, msg: Message| {
                let tx = tx.clone();
                let allow_users = allow_users.clone();
                let allow_chats = allow_chats.clone();

                async move {
                    let chat_id = msg.chat.id.to_string();

                    // Access control.
                    if !allow_chats.is_empty() && !allow_chats.contains(&chat_id) {
                        if let Some(text) = msg.text()
                            && text.starts_with("/start")
                        {
                            let _ = bot.send_message(msg.chat.id, NO_ACCESS_MESSAGE).await;
                        }
                        return respond(());
                    }

                    if let Some(user) = msg.from.as_ref()
                        && !allow_users.is_empty()
                    {
                        let uid = user.id.0.to_string();
                        let uname = user.username.clone().unwrap_or_default();
                        if !allow_users.contains(&uid) && !allow_users.contains(&uname) {
                            let _ = bot.send_message(msg.chat.id, "Access denied.").await;
                            return Ok(());
                        }
                    }

                    // Handle /start command.
                    if let Some(text) = msg.text()
                        && text.starts_with("/start")
                    {
                        let _ = bot
                            .send_message(msg.chat.id, "Eli is online. Send text to start.")
                            .await;
                        return Ok(());
                    }

                    // Build and send the channel message.
                    let session_id = format!("telegram:{chat_id}");
                    let content = format_message_content(&msg);

                    // Strip /eli prefix.
                    let content = if let Some(rest) = content.strip_prefix("/eli ") {
                        rest.to_owned()
                    } else {
                        content
                    };

                    // Comma commands pass through directly.
                    if content.trim().starts_with(',') {
                        let channel_msg =
                            ChannelMessage::new(&session_id, "telegram", content.trim())
                                .with_chat_id(&chat_id)
                                .finalize();
                        let _ = tx.send(channel_msg);
                        return Ok(());
                    }

                    let mut media_items = Vec::new();
                    if let Some(item) = extract_media_item(&msg, bot.clone()) {
                        media_items.push(item);
                    }
                    if let Some(reply) = msg.reply_to_message()
                        && let Some(item) = extract_media_item(reply, bot.clone())
                    {
                        media_items.push(item);
                    }

                    let links = extract_links(&msg);
                    let msg_type = detect_message_type(&msg);
                    let sender_name = msg.from.as_ref().map(|u| u.full_name()).unwrap_or_default();
                    let sender_username = msg
                        .from
                        .as_ref()
                        .and_then(|u| u.username.clone())
                        .unwrap_or_default();
                    let sender_id = msg
                        .from
                        .as_ref()
                        .map(|u| u.id.0.to_string())
                        .unwrap_or_default();

                    let mut metadata = serde_json::Map::new();
                    metadata.insert("message_id".to_owned(), json!(msg.id.0));
                    metadata.insert("type".to_owned(), json!(msg_type));
                    metadata.insert("username".to_owned(), json!(sender_username));
                    metadata.insert("full_name".to_owned(), json!(sender_name));
                    metadata.insert("sender_id".to_owned(), json!(sender_id));
                    if !links.is_empty() {
                        metadata.insert("links".to_owned(), json!(links));
                    }
                    metadata.insert("message".to_owned(), json!(content));

                    let json_content = serde_json::Value::Object(metadata).to_string();

                    let bot_me = bot.get_me().await;
                    let (bot_id, bot_uname) = match bot_me {
                        Ok(me) => (me.id, me.username.clone().unwrap_or_default()),
                        Err(_) => (UserId(0), String::new()),
                    };
                    let is_active = should_process_message(&msg, bot_id, &bot_uname);

                    let channel_msg = ChannelMessage::new(&session_id, "telegram", &json_content)
                        .with_chat_id(&chat_id)
                        .with_is_active(is_active)
                        .with_media(media_items)
                        .with_output_channel("null")
                        .finalize();

                    let _ = tx.send(channel_msg);
                    Ok(())
                }
            });

            let listener = update_listeners::polling_default(bot_for_handler.clone()).await;
            let error_handler = Arc::new(|error: teloxide::RequestError| {
                warn!("telegram update listener: {error}");
                async {}
            });

            let mut dispatcher = Dispatcher::builder(bot_for_handler, handler).build();

            tokio::select! {
                () = dispatcher.dispatch_with_listener(listener, error_handler) => {}
                () = cancel.cancelled() => {}
            }
        });

        *self.dispatcher_handle.lock().await = Some(handle);

        info!("telegram.start polling");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        // Abort the dispatcher task so it exits immediately even if stuck
        // on a long-poll GetUpdates request.
        if let Some(handle) = self.dispatcher_handle.lock().await.take() {
            handle.abort();
            let _ = handle.await;
        }

        *self.bot.write().await = None;

        info!("telegram.stopped");
        Ok(())
    }

    async fn send(&self, message: ChannelMessage) -> anyhow::Result<()> {
        let bot_guard = self.bot.read().await;
        let bot = match bot_guard.as_ref() {
            Some(b) => b,
            None => anyhow::bail!("telegram bot not initialized"),
        };

        let chat_id: i64 = message.chat_id.parse().unwrap_or(0);
        if chat_id == 0 {
            anyhow::bail!("invalid chat_id: {}", message.chat_id);
        }

        let text = match serde_json::from_str::<serde_json::Value>(&message.content) {
            Ok(val) => {
                // Try "message" first, then "content" field
                val.get("message")
                    .or_else(|| val.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned()
            }
            Err(_) => message.content.clone(),
        };

        if text.trim().is_empty() {
            return Ok(());
        }

        // Try MarkdownV2 first, fallback to plain text if formatting is invalid
        let md_result = bot
            .send_message(ChatId(chat_id), &text)
            .parse_mode(ParseMode::MarkdownV2)
            .await;
        if md_result.is_err() {
            bot.send_message(ChatId(chat_id), &text).await?;
        }
        Ok(())
    }
}
