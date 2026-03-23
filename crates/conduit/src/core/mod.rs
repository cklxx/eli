//! Core utilities for Conduit.

pub(crate) mod anthropic_messages;
pub mod api_format;
pub mod client_registry;
pub mod errors;
pub mod execution;
pub mod provider_policies;
pub mod provider_runtime;
pub mod request_adapters;
pub mod results;
pub(crate) mod tool_calls;

pub use api_format::ApiFormat;
pub use errors::{ConduitError, ErrorKind};
pub use execution::{
    ApiBaseConfig, ApiKeyConfig, AttemptDecision, AttemptOutcome, LLMCore, TransportCallRequest,
    TransportResponse, classify_by_text_signature,
};
pub use results::{
    AsyncStreamEvents, AsyncTextStream, ErrorPayload, StreamEvent, StreamEventKind, StreamEvents,
    StreamState, TextStream, ToolAutoResult, ToolAutoResultKind, ToolExecution,
};
