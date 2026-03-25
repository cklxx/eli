//! Error classification heuristics, HTTP status mapping, and retry decision logic.

use serde_json::Value;

use super::errors::{ConduitError, ErrorKind};
use super::execution::LLMCore;
use super::results::ErrorPayload;

/// Post-failure action for a single attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptDecision {
    RetrySameModel,
    TryNextModel,
}

/// Classified error paired with the retry/fallback decision.
#[derive(Debug, Clone)]
pub struct AttemptOutcome {
    pub error: ConduitError,
    pub decision: AttemptDecision,
}

const HTTP_STATUS_PREFIXES: &[&str] = &[
    "status ", "status: ", "http ", "http/", "code ", "code: ", "error ",
];

fn has_http_status_pattern(lower: &str, code: &str) -> bool {
    HTTP_STATUS_PREFIXES
        .iter()
        .any(|prefix| lower.contains(&format!("{prefix}{code}")))
}

/// Classify an error by scanning the message text for common patterns.
///
/// Returns `None` when no pattern matches, allowing the caller to fall
/// through to other classification strategies.
pub fn classify_by_text_signature(message: &str) -> Option<ErrorKind> {
    let lower = message.to_lowercase();

    if is_auth_error(&lower) {
        return Some(ErrorKind::Config);
    }
    if is_rate_limit_error(&lower) || is_timeout_error(&lower) || is_server_error(&lower) {
        return Some(ErrorKind::Temporary);
    }
    if is_not_found_error(&lower) {
        return Some(ErrorKind::NotFound);
    }
    None
}

fn is_auth_error(lower: &str) -> bool {
    lower.contains("auth")
        || lower.contains("unauthorized")
        || lower.contains("api key")
        || lower.contains("invalid key")
}

fn is_rate_limit_error(lower: &str) -> bool {
    lower.contains("rate limit") || has_http_status_pattern(lower, "429") || lower.contains("quota")
}

fn is_not_found_error(lower: &str) -> bool {
    lower.contains("not found") || has_http_status_pattern(lower, "404")
}

fn is_timeout_error(lower: &str) -> bool {
    lower.contains("timeout") || lower.contains("timed out")
}

fn is_server_error(lower: &str) -> bool {
    lower.contains("server error")
        || has_http_status_pattern(lower, "500")
        || has_http_status_pattern(lower, "502")
        || has_http_status_pattern(lower, "503")
}

fn mask_sensitive(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        if let Some(advance) = try_mask_bearer(&text[i..], &mut out) {
            i += advance;
        } else if let Some(advance) = try_mask_key_prefix(&text[i..], bytes, i, &mut out) {
            i += advance;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn try_mask_bearer(slice: &str, out: &mut String) -> Option<usize> {
    if !slice.starts_with("Bearer ") {
        return None;
    }
    out.push_str("Bearer [MASKED]");
    let token_len = slice[7..]
        .bytes()
        .take_while(|b| !b.is_ascii_whitespace())
        .count();
    Some(7 + token_len)
}

const KEY_PREFIXES: &[&str] = &["sk-", "key-", "token-"];

fn try_mask_key_prefix(
    slice: &str,
    bytes: &[u8],
    offset: usize,
    out: &mut String,
) -> Option<usize> {
    for prefix in KEY_PREFIXES {
        if !slice.starts_with(prefix) {
            continue;
        }
        let start = offset + prefix.len();
        let suffix_len = bytes[start..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_' || **b == b'-')
            .count();
        if suffix_len >= 20 {
            out.push_str("[MASKED_KEY]");
            return Some(prefix.len() + suffix_len);
        }
    }
    None
}

impl LLMCore {
    /// Log a sanitized error at the warning level when verbose mode is enabled.
    pub fn log_error(&self, error: &ConduitError, provider: &str, model: &str, attempt: u32) {
        if self.verbose() == 0 {
            return;
        }
        let prefix = format!(
            "[{provider}:{model}] attempt {}/{}",
            attempt + 1,
            self.max_attempts()
        );
        let sanitized = mask_sensitive(&error.to_string());
        let cause_suffix = error
            .cause
            .as_ref()
            .map(|c| format!(" (cause={})", mask_sensitive(&format!("{c:?}"))))
            .unwrap_or_default();
        tracing::warn!("{prefix} failed: {sanitized}{cause_suffix}");
    }

    /// Classify an error: custom classifier, then text heuristic, then error's own kind.
    pub fn classify_error(&self, error: &ConduitError) -> ErrorKind {
        self.custom_classify(error)
            .or_else(|| classify_by_text_signature(&error.message))
            .unwrap_or(error.kind)
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
        ConduitError::new(kind, format!("{provider}:{model}: {message}"))
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
        let decision = if Self::should_retry(kind) && (attempt + 1) < self.max_attempts() {
            AttemptDecision::RetrySameModel
        } else {
            AttemptDecision::TryNextModel
        };
        AttemptOutcome { error, decision }
    }

    /// Build an `ErrorPayload` with retry context details.
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
        if let Some(status) = http_status
            && let Value::Object(obj) = &mut details
        {
            obj.insert("http_status".to_owned(), Value::Number(status.into()));
        }
        ErrorPayload::new(error.kind, &error.message).with_details(details)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_bearer_token() {
        let input = "Authorization: Bearer sk-abc123def456ghi789jkl0 failed";
        let masked = mask_sensitive(input);
        assert!(masked.contains("Bearer [MASKED]"));
        assert!(!masked.contains("sk-abc123"));
    }

    #[test]
    fn mask_sk_key() {
        let input = "invalid api key: sk-proj-abcdefghijklmnopqrstuvwx";
        let masked = mask_sensitive(input);
        assert!(masked.contains("[MASKED_KEY]"));
        assert!(!masked.contains("sk-proj-abcdefgh"));
    }

    #[test]
    fn no_mask_short_prefix() {
        // Short strings after prefix should NOT be masked (not a real key).
        let input = "sk-short is fine";
        let masked = mask_sensitive(input);
        assert_eq!(masked, input);
    }

    #[test]
    fn no_mask_normal_text() {
        let input = "rate limit exceeded, please retry";
        let masked = mask_sensitive(input);
        assert_eq!(masked, input);
    }

    #[test]
    fn mask_multiple_keys() {
        let input = "key-aaaaaaaaaaaaaaaaaaaaaaaaa and token-bbbbbbbbbbbbbbbbbbbbbbbbb";
        let masked = mask_sensitive(input);
        assert_eq!(
            masked.matches("[MASKED_KEY]").count(),
            2,
            "should mask both keys: {masked}"
        );
    }

    #[test]
    fn classify_auth_error() {
        assert_eq!(
            classify_by_text_signature("unauthorized access"),
            Some(ErrorKind::Config)
        );
    }

    #[test]
    fn classify_rate_limit() {
        assert_eq!(
            classify_by_text_signature("rate limit exceeded"),
            Some(ErrorKind::Temporary)
        );
    }

    #[test]
    fn classify_no_match() {
        assert_eq!(
            classify_by_text_signature("something random happened"),
            None
        );
    }
}
