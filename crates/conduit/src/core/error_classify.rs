//! Error classification heuristics, HTTP status mapping, and retry decision logic.

use serde_json::Value;

use super::errors::{ConduitError, ErrorKind};
use super::execution::LLMCore;
use super::results::ErrorPayload;

/// What to do after one failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptDecision {
    RetrySameModel,
    TryNextModel,
}

/// Result of classifying and deciding how to handle one exception.
#[derive(Debug, Clone)]
pub struct AttemptOutcome {
    pub error: ConduitError,
    pub decision: AttemptDecision,
}

/// Check for an HTTP status code in contextual patterns to avoid false
/// positives on bare numbers (e.g. "Expected 404 items").
fn has_http_status_pattern(lower: &str, code: &str) -> bool {
    lower.contains(&format!("status {code}"))
        || lower.contains(&format!("status: {code}"))
        || lower.contains(&format!("http {code}"))
        || lower.contains(&format!("http/{code}"))
        || lower.contains(&format!("code {code}"))
        || lower.contains(&format!("code: {code}"))
        || lower.contains(&format!("error {code}"))
}

/// Classify an error by scanning the message text for common patterns.
///
/// Returns `None` when no pattern matches, allowing the caller to fall
/// through to other classification strategies.
pub fn classify_by_text_signature(message: &str) -> Option<ErrorKind> {
    let lower = message.to_lowercase();

    // Authentication / configuration errors
    if lower.contains("auth")
        || lower.contains("unauthorized")
        || lower.contains("api key")
        || lower.contains("invalid key")
    {
        return Some(ErrorKind::Config);
    }

    // Rate-limit / quota errors
    if lower.contains("rate limit")
        || has_http_status_pattern(&lower, "429")
        || lower.contains("quota")
    {
        return Some(ErrorKind::Temporary);
    }

    // Not-found errors
    if lower.contains("not found") || has_http_status_pattern(&lower, "404") {
        return Some(ErrorKind::NotFound);
    }

    // Timeout errors
    if lower.contains("timeout") || lower.contains("timed out") {
        return Some(ErrorKind::Temporary);
    }

    // Server errors
    if lower.contains("server error")
        || has_http_status_pattern(&lower, "500")
        || has_http_status_pattern(&lower, "502")
        || has_http_status_pattern(&lower, "503")
    {
        return Some(ErrorKind::Temporary);
    }

    None
}

impl LLMCore {
    /// Log an error at the warning level if verbose mode is enabled.
    pub fn log_error(&self, error: &ConduitError, provider: &str, model: &str, attempt: u32) {
        if self.verbose() == 0 {
            return;
        }
        let prefix = format!(
            "[{}:{}] attempt {}/{}",
            provider,
            model,
            attempt + 1,
            self.max_attempts()
        );
        if let Some(ref cause) = error.cause {
            tracing::warn!("{} failed: {} (cause={:?})", prefix, error, cause);
        } else {
            tracing::warn!("{} failed: {}", prefix, error);
        }
    }

    /// Classify an error into an `ErrorKind`.
    ///
    /// Resolution order:
    /// 1. Custom classifier (if set)
    /// 2. Text-signature heuristic on the error message
    /// 3. The error's own `kind` field
    pub fn classify_error(&self, error: &ConduitError) -> ErrorKind {
        if let Some(kind) = self.custom_classify(error) {
            return kind;
        }
        if let Some(kind) = classify_by_text_signature(&error.message) {
            return kind;
        }
        error.kind
    }

    /// Classify an HTTP status code into an `ErrorKind`.
    pub fn classify_http_status(status: u16) -> Option<ErrorKind> {
        match status {
            401 | 403 => Some(ErrorKind::Config),
            400 | 404 | 413 | 422 => Some(ErrorKind::InvalidInput),
            408 | 409 | 425 | 429 => Some(ErrorKind::Temporary),
            s if (500..600).contains(&s) => Some(ErrorKind::Provider),
            _ => None,
        }
    }

    /// Whether the error kind should trigger a retry.
    pub fn should_retry(kind: ErrorKind) -> bool {
        matches!(kind, ErrorKind::Temporary | ErrorKind::Provider)
    }

    /// Wrap a raw error message into a `ConduitError` with provider/model context.
    pub fn wrap_error(
        &self,
        kind: ErrorKind,
        provider: &str,
        model: &str,
        message: &str,
    ) -> ConduitError {
        ConduitError::new(kind, format!("{}:{}: {}", provider, model, message))
    }

    /// Handle a single failed attempt and decide what to do next.
    pub fn handle_attempt_error(
        &self,
        error: ConduitError,
        provider_name: &str,
        model_id: &str,
        attempt: u32,
    ) -> AttemptOutcome {
        let kind = self.classify_error(&error);
        self.log_error(&error, provider_name, model_id, attempt);
        let can_retry = Self::should_retry(kind) && (attempt + 1) < self.max_attempts();
        let decision = if can_retry {
            AttemptDecision::RetrySameModel
        } else {
            AttemptDecision::TryNextModel
        };
        AttemptOutcome { error, decision }
    }

    /// Build an `ErrorPayload` with populated details from retry context.
    ///
    /// The `details` object includes `provider`, `model`, `attempt`,
    /// `max_attempts`, and optionally `http_status`.
    pub fn build_error_payload(
        &self,
        error: &ConduitError,
        provider_name: &str,
        model_id: &str,
        attempt: u32,
        http_status: Option<u16>,
    ) -> ErrorPayload {
        let mut details = serde_json::json!({
            "provider": provider_name,
            "model": model_id,
            "attempt": attempt + 1,
            "max_attempts": self.max_attempts(),
        });
        if let Some(status) = http_status {
            details
                .as_object_mut()
                .unwrap()
                .insert("http_status".to_owned(), Value::Number(status.into()));
        }
        ErrorPayload::new(error.kind, &error.message).with_details(details)
    }
}
