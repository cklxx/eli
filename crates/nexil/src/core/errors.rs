//! Error definitions for Conduit.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable error kinds for caller decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidInput,
    Config,
    Provider,
    Tool,
    Temporary,
    NotFound,
    Unknown,
}

impl ErrorKind {
    /// Return the snake_case string value, matching the Python StrEnum values.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::InvalidInput => "invalid_input",
            ErrorKind::Config => "config",
            ErrorKind::Provider => "provider",
            ErrorKind::Tool => "tool",
            ErrorKind::Temporary => "temporary",
            ErrorKind::NotFound => "not_found",
            ErrorKind::Unknown => "unknown",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Public error type for Conduit.
#[derive(Debug, Clone, thiserror::Error)]
#[error("[{kind}] {message}")]
pub struct ConduitError {
    pub kind: ErrorKind,
    pub message: String,
    #[source]
    pub cause: Option<Box<ConduitError>>,
}

impl ConduitError {
    /// Create a new `ConduitError` without a cause.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            cause: None,
        }
    }

    /// Return a new error that is identical but with the given cause attached.
    pub fn with_cause(self, cause: ConduitError) -> Self {
        Self {
            kind: self.kind,
            message: self.message,
            cause: Some(Box::new(cause)),
        }
    }
}
