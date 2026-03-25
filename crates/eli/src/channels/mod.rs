//! Channel subsystem — pluggable transports for user interaction.
//!
//! Each channel implements the [`Channel`] trait and is managed by a
//! [`ChannelManager`] that routes inbound messages to the framework and
//! dispatches outbound envelopes to the correct channel.

pub mod base;
pub mod cli;
pub mod handler;
pub mod manager;
pub mod message;
#[cfg(feature = "telegram")]
pub mod telegram;
#[cfg(feature = "gateway")]
pub mod webhook;

pub use base::Channel;
pub use cli::{CliChannel, CliRenderer};
pub use handler::BufferedMessageHandler;
pub use manager::{ChannelManager, ChannelSettings, InboundProcessor, OutboundRouter};
pub use message::{ChannelMessage, DataFetcher, MediaItem, MediaType, MessageKind};
#[cfg(feature = "telegram")]
pub use telegram::{TelegramChannel, TelegramSettings};
#[cfg(feature = "gateway")]
pub use webhook::{WebhookChannel, WebhookSettings};
