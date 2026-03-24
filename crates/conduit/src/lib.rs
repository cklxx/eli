//! Conduit: a developer-first LLM toolkit.

pub(crate) mod adapter;
pub mod auth;
pub mod clients;
pub mod core;
pub mod llm;
pub(crate) mod providers;
pub mod tape;
pub mod tools;

// Re-export top-level public types for convenience.
pub use crate::auth::{
    APIKeyResolver, CodexOAuthLoginError, GitHubCopilotOAuthLoginError, GitHubCopilotOAuthTokens,
    OpenAICodexOAuthTokens, codex_cli_api_key_resolver, github_copilot_oauth_resolver,
    load_openai_codex_oauth_tokens, login_github_copilot_oauth, login_openai_codex_oauth,
    multi_api_key_resolver, openai_codex_oauth_resolver,
};
pub use crate::clients::InternalOps;
pub use crate::core::errors::{ConduitError, ErrorKind};
pub use crate::core::results::{
    AsyncStreamEvents, AsyncTextStream, ErrorPayload, StreamEvent, StreamEvents, StreamState,
    TextStream, ToolAutoResult, ToolExecution,
};
pub use crate::llm::{ApiFormat, ChatRequest, EmbedInput, LLM, LLMBuilder, StreamEventFilter};
pub use crate::tape::{
    AnchorSelector, TapeContext, TapeEntry, TapeManager, TapeQuery, TapeSession,
};
pub use crate::tools::{
    Tool, ToolCallResponse, ToolContext, ToolExecutor, ToolSet, normalize_tools, tool_from_fn,
    tool_from_schema,
};
