//! Telegram channel — receives and sends messages via the Telegram Bot API
//! using the `teloxide` crate.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use teloxide::dispatching::{Dispatcher, UpdateFilterExt};
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{
    ChatKind, ChatMemberKind, ChatMemberUpdated, MediaKind, MessageKind as TgMessageKind,
    ParseMode, Update,
};
use teloxide::update_listeners::Polling;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::base::Channel;
use super::message::{ChannelMessage, DataFetcher, MediaItem, MediaType, MessageKind};

// ---------------------------------------------------------------------------
// TelegramSettings
// ---------------------------------------------------------------------------

/// Settings for the Telegram channel, loaded from env vars prefixed
/// `ELI_TELEGRAM_`.
#[derive(Clone)]
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

impl std::fmt::Debug for TelegramSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramSettings")
            .field("token", &"[REDACTED]")
            .field("allow_users", &self.allow_users)
            .field("allow_chats", &self.allow_chats)
            .field("proxy", &self.proxy)
            .finish()
    }
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

// ---------------------------------------------------------------------------
// EliMessageFilter logic
// ---------------------------------------------------------------------------

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

        if msg_type != "text" && extract_caption(msg).is_none() {
            return reply_to_bot;
        }

        return mentions_bot || reply_to_bot;
    }

    false
}

fn is_reply_to_bot(msg: &Message, bot_id: UserId) -> bool {
    msg.reply_to_message()
        .and_then(|reply| reply.from.as_ref())
        .is_some_and(|u| u.id == bot_id)
}

fn extract_text_content(msg: &Message) -> Option<&str> {
    msg.text().or_else(|| msg.caption())
}

fn extract_caption(msg: &Message) -> Option<&str> {
    msg.caption()
}

// ---------------------------------------------------------------------------
// Media extraction helpers
// ---------------------------------------------------------------------------

type MediaItemData = (MediaType, String, String, Option<String>);

fn common_media_kind(msg: &Message) -> Option<&MediaKind> {
    match &msg.kind {
        TgMessageKind::Common(common) => Some(&common.media_kind),
        _ => None,
    }
}

fn build_data_fetcher(bot: Bot, file_id: String) -> DataFetcher {
    Arc::new(move || {
        let bot = bot.clone();
        let file_id = file_id.clone();
        Box::pin(async move { download_file(&bot, &file_id).await })
    })
}

fn media_item_data(
    media_type: MediaType,
    file_id: String,
    mime_type: String,
    filename: Option<String>,
) -> MediaItemData {
    (media_type, file_id, mime_type, filename)
}

fn mime_or_default<T: ToString>(mime_type: Option<&T>, default: &str) -> String {
    mime_type
        .map(ToString::to_string)
        .unwrap_or_else(|| default.to_owned())
}

fn map_media_kind(media_kind: &MediaKind) -> Option<MediaItemData> {
    match media_kind {
        MediaKind::Photo(photo) => photo.photo.last().map(|size| {
            media_item_data(
                MediaType::Image,
                size.file.id.clone(),
                "image/jpeg".to_owned(),
                None,
            )
        }),
        MediaKind::Audio(audio) => Some(media_item_data(
            MediaType::Audio,
            audio.audio.file.id.clone(),
            mime_or_default(audio.audio.mime_type.as_ref(), "audio/mpeg"),
            audio.audio.file_name.clone(),
        )),
        MediaKind::Voice(voice) => Some(media_item_data(
            MediaType::Audio,
            voice.voice.file.id.clone(),
            mime_or_default(voice.voice.mime_type.as_ref(), "audio/ogg"),
            None,
        )),
        MediaKind::Video(video) => Some(media_item_data(
            MediaType::Video,
            video.video.file.id.clone(),
            mime_or_default(video.video.mime_type.as_ref(), "video/mp4"),
            video.video.file_name.clone(),
        )),
        MediaKind::VideoNote(video_note) => Some(media_item_data(
            MediaType::Video,
            video_note.video_note.file.id.clone(),
            "video/mp4".to_owned(),
            None,
        )),
        MediaKind::Document(document) => Some(media_item_data(
            MediaType::Document,
            document.document.file.id.clone(),
            mime_or_default(
                document.document.mime_type.as_ref(),
                "application/octet-stream",
            ),
            document.document.file_name.clone(),
        )),
        MediaKind::Sticker(sticker) => Some(media_item_data(
            MediaType::Image,
            sticker.sticker.file.id.clone(),
            if sticker.sticker.is_animated() {
                "video/webm"
            } else {
                "image/webp"
            }
            .to_owned(),
            None,
        )),
        _ => None,
    }
}

fn extract_media_item(msg: &Message, bot: Bot) -> Option<MediaItem> {
    let (media_type, file_id, mime_type, filename) = map_media_kind(common_media_kind(msg)?)?;
    Some(MediaItem {
        media_type,
        mime_type,
        filename,
        data_fetcher: Some(build_data_fetcher(bot, file_id)),
    })
}

async fn download_file(bot: &Bot, file_id: &str) -> Vec<u8> {
    match try_download_file(bot, file_id).await {
        Ok(buf) => buf,
        Err(e) => {
            error!(error = %e, file_id = %file_id, "failed to download telegram file");
            Vec::new()
        }
    }
}

async fn try_download_file(
    bot: &Bot,
    file_id: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use futures::StreamExt as _;
    let file = bot.get_file(file_id).await?;
    let stream = bot.download_file_stream(&file.path);
    let mut buf = Vec::new();
    let mut stream = std::pin::pin!(stream);
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk?);
    }
    Ok(buf)
}

fn format_captioned_content(base: String, caption: &str) -> String {
    if caption.is_empty() {
        base
    } else {
        format!("{base} Caption: {caption}")
    }
}

fn format_audio_content(msg: &Message) -> String {
    let Some(MediaKind::Audio(audio)) = common_media_kind(msg) else {
        return "[Audio]".to_owned();
    };
    let title = audio.audio.title.as_deref().unwrap_or("Unknown");
    let performer = audio.audio.performer.as_deref().unwrap_or("");
    let duration = audio.audio.duration.seconds();
    if performer.is_empty() {
        format!("[Audio: {title} ({duration}s)]")
    } else {
        format!("[Audio: {performer} - {title} ({duration}s)]")
    }
}

fn format_sticker_content(msg: &Message) -> String {
    let Some(MediaKind::Sticker(sticker)) = common_media_kind(msg) else {
        return "[Sticker]".to_owned();
    };
    let emoji = sticker.sticker.emoji.as_deref().unwrap_or("");
    let set_name = sticker.sticker.set_name.as_deref().unwrap_or("");
    if emoji.is_empty() {
        format!("[Sticker from {set_name}]")
    } else {
        format!("[Sticker: {emoji} from {set_name}]")
    }
}

fn format_document_content(msg: &Message, caption: &str) -> String {
    let Some(MediaKind::Document(document)) = common_media_kind(msg) else {
        return "[Document: unknown (application/octet-stream)]".to_owned();
    };
    let file_name = document.document.file_name.as_deref().unwrap_or("unknown");
    let mime_type = mime_or_default(
        document.document.mime_type.as_ref(),
        "application/octet-stream",
    );
    let base = format!("[Document: {file_name} ({mime_type})]");
    format_captioned_content(base, caption)
}

fn video_duration_seconds(msg: &Message) -> u32 {
    match common_media_kind(msg) {
        Some(MediaKind::Video(video)) => video.video.duration.seconds(),
        _ => 0,
    }
}

fn voice_duration_seconds(msg: &Message) -> u32 {
    match common_media_kind(msg) {
        Some(MediaKind::Voice(voice)) => voice.voice.duration.seconds(),
        _ => 0,
    }
}

fn video_note_duration_seconds(msg: &Message) -> u32 {
    match common_media_kind(msg) {
        Some(MediaKind::VideoNote(video_note)) => video_note.video_note.duration.seconds(),
        _ => 0,
    }
}

fn format_message_content(msg: &Message) -> String {
    let caption = extract_caption(msg).unwrap_or("");

    match detect_message_type(msg) {
        "text" => msg.text().unwrap_or("").to_owned(),
        "photo" => format_captioned_content("[Photo message]".to_owned(), caption),
        "audio" => format_audio_content(msg),
        "sticker" => format_sticker_content(msg),
        "video" => format_captioned_content(
            format!("[Video: {}s]", video_duration_seconds(msg)),
            caption,
        ),
        "voice" => format!("[Voice message: {}s]", voice_duration_seconds(msg)),
        "document" => format_document_content(msg, caption),
        "video_note" => format!("[Video note: {}s]", video_note_duration_seconds(msg)),
        msg_type => format!("[Unsupported message type: {msg_type}]"),
    }
}

fn extract_links(msg: &Message) -> Vec<String> {
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
        return Vec::new();
    };

    let mut seen = HashSet::new();
    entities
        .iter()
        .filter_map(|entity| extract_link_from_entity(entity, &source_text))
        .filter(|url| seen.insert(url.clone()))
        .collect()
}

fn extract_link_from_entity(
    entity: &teloxide::types::MessageEntity,
    source_text: &str,
) -> Option<String> {
    match &entity.kind {
        teloxide::types::MessageEntityKind::TextLink { url } => Some(url.as_str().to_owned()),
        teloxide::types::MessageEntityKind::Url => {
            let candidate: String = source_text
                .chars()
                .skip(entity.offset)
                .take(entity.length)
                .collect::<String>()
                .trim()
                .to_owned();
            if candidate.is_empty() {
                None
            } else {
                Some(candidate)
            }
        }
        _ => None,
    }
}

fn strip_eli_prefix(content: String) -> String {
    content
        .strip_prefix("/eli ")
        .map(|rest| rest.to_owned())
        .unwrap_or(content)
}

enum AccessResult {
    Allowed,
    DeniedChat,
    DeniedUser,
    StartCommand,
}

fn check_access(
    msg: &Message,
    allow_chats: &HashSet<String>,
    allow_users: &HashSet<String>,
) -> AccessResult {
    let chat_id = msg.chat.id.to_string();

    if !allow_chats.is_empty() && !allow_chats.contains(&chat_id) {
        if msg.text().is_some_and(|t| t.starts_with("/start")) {
            return AccessResult::DeniedChat;
        }
        return AccessResult::DeniedChat;
    }

    if let Some(user) = msg.from.as_ref()
        && !allow_users.is_empty()
    {
        let uid = user.id.0.to_string();
        let uname = user.username.clone().unwrap_or_default();
        if !allow_users.contains(&uid) && !allow_users.contains(&uname) {
            return AccessResult::DeniedUser;
        }
    }

    if msg.text().is_some_and(|t| t.starts_with("/start")) {
        return AccessResult::StartCommand;
    }

    AccessResult::Allowed
}

fn build_channel_message(
    msg: &Message,
    bot: &Bot,
    bot_id: UserId,
    bot_username: &str,
) -> ChannelMessage {
    let chat_id = msg.chat.id.to_string();
    let session_id = format!("telegram:{chat_id}");
    let content = strip_eli_prefix(format_message_content(msg));

    if content.trim().starts_with('/') {
        return ChannelMessage::new(&session_id, "telegram", content.trim())
            .with_chat_id(&chat_id)
            .finalize();
    }

    let media_items = collect_media_items(msg, bot);
    let json_content = build_message_metadata(msg, &content);
    let is_active = should_process_message(msg, bot_id, bot_username);

    ChannelMessage::new(&session_id, "telegram", &json_content)
        .with_chat_id(&chat_id)
        .with_is_active(is_active)
        .with_media(media_items)
        .finalize()
}

fn collect_media_items(msg: &Message, bot: &Bot) -> Vec<MediaItem> {
    [Some(msg), msg.reply_to_message()]
        .into_iter()
        .flatten()
        .filter_map(|m| extract_media_item(m, bot.clone()))
        .collect()
}

fn build_message_metadata(msg: &Message, content: &str) -> String {
    let links = extract_links(msg);
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

    let mut metadata = serde_json::json!({
        "message_id": msg.id.0,
        "type": detect_message_type(msg),
        "username": sender_username,
        "full_name": sender_name,
        "sender_id": sender_id,
        "message": content,
    });
    if !links.is_empty() {
        metadata["links"] = json!(links);
    }
    metadata.to_string()
}

async fn resolve_bot_identity(bot: &Bot) -> (UserId, String) {
    match bot.get_me().await {
        Ok(me) => (me.id, me.username.clone().unwrap_or_default()),
        Err(_) => (UserId(0), String::new()),
    }
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
            let tx_for_join = tx.clone();

            let message_handler =
                Update::filter_message().endpoint(move |bot: Bot, msg: Message| {
                    let tx = tx.clone();
                    let allow_users = allow_users.clone();
                    let allow_chats = allow_chats.clone();

                    async move {
                        match check_access(&msg, &allow_chats, &allow_users) {
                            AccessResult::DeniedChat => {
                                if msg.text().is_some_and(|t| t.starts_with("/start")) {
                                    let _ = bot.send_message(msg.chat.id, NO_ACCESS_MESSAGE).await;
                                }
                                return respond(());
                            }
                            AccessResult::DeniedUser => {
                                let _ = bot.send_message(msg.chat.id, "Access denied.").await;
                                return Ok(());
                            }
                            AccessResult::StartCommand => {
                                let _ = bot
                                    .send_message(msg.chat.id, "Eli is online. Send text to start.")
                                    .await;
                                return Ok(());
                            }
                            AccessResult::Allowed => {}
                        }

                        let (bot_id, bot_uname) = resolve_bot_identity(&bot).await;
                        let channel_msg = build_channel_message(&msg, &bot, bot_id, &bot_uname);
                        let _ = tx.send(channel_msg);
                        Ok(())
                    }
                });

            let join_handler = Update::filter_my_chat_member().endpoint(
                move |_bot: Bot, update: ChatMemberUpdated| {
                    let tx = tx_for_join.clone();
                    async move {
                        let was_absent = matches!(
                            update.old_chat_member.kind,
                            ChatMemberKind::Left | ChatMemberKind::Banned(_)
                        );
                        let is_present = matches!(
                            update.new_chat_member.kind,
                            ChatMemberKind::Member
                                | ChatMemberKind::Administrator(_)
                                | ChatMemberKind::Owner(_)
                        );
                        if was_absent && is_present {
                            let chat_id = update.chat.id.to_string();
                            let session_id = format!("telegram:{chat_id}");
                            let msg = ChannelMessage::new(&session_id, "telegram", "")
                                .with_chat_id(&chat_id)
                                .with_kind(MessageKind::Join)
                                .with_is_active(true)
                                .finalize();
                            let _ = tx.send(msg);
                        }
                        respond(())
                    }
                },
            );

            let handler = dptree::entry().branch(message_handler).branch(join_handler);

            let listener = Polling::builder(bot_for_handler.clone())
                .timeout(std::time::Duration::from_secs(5))
                .allowed_updates(vec![
                    teloxide::types::AllowedUpdate::Message,
                    teloxide::types::AllowedUpdate::MyChatMember,
                ])
                .delete_webhook()
                .await
                .build();
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

        // Send text if present.
        let text = serde_json::from_str::<serde_json::Value>(&message.content)
            .ok()
            .and_then(|val| {
                val.get("message")
                    .or_else(|| val.get("content"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned())
            })
            .unwrap_or_else(|| message.content.clone());

        if !text.trim().is_empty() {
            let md_result = bot
                .send_message(ChatId(chat_id), &text)
                .parse_mode(ParseMode::MarkdownV2)
                .await;
            if md_result.is_err() {
                bot.send_message(ChatId(chat_id), &text).await?;
            }
        }

        // Send media if present.
        if let Some(media) = message
            .context
            .get("outbound_media")
            .and_then(|v| v.as_array())
        {
            for item in media {
                let Some(path) = item.get("path").and_then(|v| v.as_str()) else {
                    continue;
                };
                // Reject paths with traversal components to prevent sending
                // arbitrary system files via Telegram.
                if std::path::Path::new(path)
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    warn!(path, "outbound_media: path traversal rejected");
                    continue;
                }
                if !std::path::Path::new(path).exists() {
                    warn!(path, "outbound_media: file not found, skipping");
                    continue;
                }
                let media_type = item
                    .get("media_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("document");
                let input_file = teloxide::types::InputFile::file(path);
                let result = match media_type {
                    "image" => bot
                        .send_photo(ChatId(chat_id), input_file)
                        .await
                        .map(|_| ()),
                    "video" => bot
                        .send_video(ChatId(chat_id), input_file)
                        .await
                        .map(|_| ()),
                    "audio" => bot
                        .send_audio(ChatId(chat_id), input_file)
                        .await
                        .map(|_| ()),
                    _ => bot
                        .send_document(ChatId(chat_id), input_file)
                        .await
                        .map(|_| ()),
                };
                if let Err(e) = result {
                    error!(path, error = %e, "outbound_media: telegram send failed");
                }
            }
        }

        Ok(())
    }
}
