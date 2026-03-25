//! Parsing helpers for provider response payloads.

pub mod common;
pub mod completion;
pub mod messages;
pub mod responses;
pub mod sse;
pub mod types;

pub use completion::CompletionTransportParser;
pub use messages::MessagesTransportParser;
pub use responses::ResponseTransportParser;
pub use types::{BaseTransportParser, ToolCallDelta, TransportKind};

use std::sync::LazyLock;

static COMPLETION_PARSER: LazyLock<CompletionTransportParser> =
    LazyLock::new(|| CompletionTransportParser);
static RESPONSE_PARSER: LazyLock<ResponseTransportParser> =
    LazyLock::new(|| ResponseTransportParser);
static MESSAGES_PARSER: LazyLock<MessagesTransportParser> =
    LazyLock::new(|| MessagesTransportParser);

/// Return the appropriate parser for a given transport kind.
pub fn parser_for_transport(transport: TransportKind) -> &'static dyn BaseTransportParser {
    match transport {
        TransportKind::Completion => &*COMPLETION_PARSER,
        TransportKind::Responses => &*RESPONSE_PARSER,
        TransportKind::Messages => &*MESSAGES_PARSER,
    }
}
