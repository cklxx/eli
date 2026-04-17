//! Local inference backend auto-detection.
//!
//! Probes a short ordered list of candidate URLs for an OpenAI-compatible
//! `/v1/models` endpoint. Used by `eli login agent-infer` to locate a running
//! local server without requiring the user to pass flags.

use std::time::Duration;

use serde::Deserialize;

/// Outcome of a successful probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetectResult {
    /// Base URL (without trailing slash), e.g. `http://127.0.0.1:8000/v1`.
    pub api_base: String,
    /// First model id reported by the server's `/v1/models` endpoint.
    pub model_id: String,
}

#[derive(Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

/// Build the ordered list of candidate base URLs for agent-infer.
///
/// If `$AGENT_INFER_URL` is set (non-empty), it is the *only* candidate —
/// explicit configuration shouldn't silently fall back to localhost if the
/// intended endpoint is unreachable, since that masks typos and stale env.
/// Otherwise we probe the default bind (8000) then the Metal alt port (8012).
/// Candidates are normalized to always end in `/v1` (the path `/v1/models`
/// is appended by the probe).
pub(crate) fn agent_infer_candidates() -> Vec<String> {
    if let Ok(url) = std::env::var("AGENT_INFER_URL") {
        let url = url.trim();
        if !url.is_empty() {
            return vec![normalize_base(url)];
        }
    }
    vec![
        "http://127.0.0.1:8000/v1".to_owned(),
        "http://127.0.0.1:8012/v1".to_owned(),
    ]
}

fn normalize_base(raw: &str) -> String {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/v1")
    }
}

/// Parse a `/v1/models` response body, returning the first model id.
pub(crate) fn parse_first_model_id(body: &str) -> Option<String> {
    let parsed: ModelsResponse = serde_json::from_str(body).ok()?;
    parsed
        .data
        .into_iter()
        .map(|m| m.id.trim().to_owned())
        .find(|id| !id.is_empty())
}

/// Probe a single candidate base URL.
///
/// Returns `Some(DetectResult)` on HTTP 200 with at least one model id,
/// `None` on any failure (connection refused, timeout, malformed JSON,
/// empty model list).
pub(crate) async fn probe(api_base: &str) -> Option<DetectResult> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(500))
        .timeout(Duration::from_secs(1))
        .build()
        .ok()?;
    let url = format!("{api_base}/models");
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    let model_id = parse_first_model_id(&body)?;
    Some(DetectResult {
        api_base: api_base.to_owned(),
        model_id,
    })
}

/// Probe all agent-infer candidates in order; return the first that responds.
pub(crate) async fn detect_agent_infer() -> Option<DetectResult> {
    for candidate in agent_infer_candidates() {
        if let Some(hit) = probe(&candidate).await {
            return Some(hit);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_first_model_id() {
        let body = r#"{"object":"list","data":[
            {"id":"mlx-community/Qwen3-0.6B-4bit","object":"model"},
            {"id":"second/model","object":"model"}
        ]}"#;
        assert_eq!(
            parse_first_model_id(body).as_deref(),
            Some("mlx-community/Qwen3-0.6B-4bit")
        );
    }

    #[test]
    fn parse_returns_none_on_empty_list() {
        let body = r#"{"object":"list","data":[]}"#;
        assert!(parse_first_model_id(body).is_none());
    }

    #[test]
    fn parse_returns_none_on_malformed_json() {
        assert!(parse_first_model_id("not json").is_none());
        assert!(parse_first_model_id("{").is_none());
    }

    #[test]
    fn parse_skips_empty_ids() {
        let body = r#"{"data":[{"id":""},{"id":"real-model"}]}"#;
        assert_eq!(parse_first_model_id(body).as_deref(), Some("real-model"));
    }

    #[test]
    fn candidates_use_defaults_without_env() {
        // SAFETY: env mutation is confined to this test's thread; the default
        // path does not read AGENT_INFER_URL when unset.
        unsafe {
            std::env::remove_var("AGENT_INFER_URL");
        }
        let candidates = agent_infer_candidates();
        assert_eq!(
            candidates,
            vec![
                "http://127.0.0.1:8000/v1".to_owned(),
                "http://127.0.0.1:8012/v1".to_owned(),
            ]
        );
    }

    #[test]
    fn normalize_appends_v1_when_missing() {
        assert_eq!(normalize_base("http://host:9000"), "http://host:9000/v1");
        assert_eq!(normalize_base("http://host:9000/"), "http://host:9000/v1");
        assert_eq!(normalize_base("http://host:9000/v1"), "http://host:9000/v1");
        assert_eq!(
            normalize_base("http://host:9000/v1/"),
            "http://host:9000/v1"
        );
    }

    #[test]
    fn candidates_env_var_is_exclusive() {
        // SAFETY: env mutation is confined to this test thread; AGENT_INFER_URL
        // is restored/removed before return so other tests are unaffected.
        unsafe {
            std::env::set_var("AGENT_INFER_URL", "http://explicit-host:9000");
        }
        let candidates = agent_infer_candidates();
        unsafe {
            std::env::remove_var("AGENT_INFER_URL");
        }
        assert_eq!(candidates, vec!["http://explicit-host:9000/v1".to_owned()]);
    }

    #[tokio::test]
    async fn probe_returns_none_on_connection_refused() {
        // Port 1 is privileged and unbound in standard environments; reqwest
        // fails fast. Timeout still bounds the wait at 1s regardless.
        assert!(probe("http://127.0.0.1:1/v1").await.is_none());
    }
}
