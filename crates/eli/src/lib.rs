//! Eli: a hook-first AI agent framework.
//!
//! This crate provides the core framework runtime, hook specifications,
//! envelope utilities, and shared types for building Eli-based applications.

pub mod builtin;
pub mod channels;
pub mod envelope;
pub mod framework;
pub mod hooks;
pub mod prompt_builder;
pub mod skill_matcher;
pub mod skills;
pub mod smart_router;
pub mod tool_middleware;
pub mod tools;
pub mod types;
pub mod utils;

// Re-export key types at the crate root for convenience.
pub use framework::EliFramework;
pub use hooks::{ChannelHook, EliHookSpec, HookError, HookRuntime, TapeStoreKind};
pub use types::{
    Envelope, MessageHandler, OutboundChannelRouter, OutboundDispatcher, PromptValue, State,
    TurnResult,
};
