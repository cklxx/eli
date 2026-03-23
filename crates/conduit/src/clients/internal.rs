//! InternalOps client for direct provider API access.

use serde_json::Value;
use std::sync::Arc;

use crate::core::errors::{ConduitError, ErrorKind};
use crate::core::execution::LLMCore;

/// Default API base URL for a provider.
fn default_api_base(provider: &str) -> String {
    match provider {
        "openai" => "https://api.openai.com/v1".to_string(),
        "anthropic" => "https://api.anthropic.com/v1".to_string(),
        other => format!("https://api.{other}.com/v1"),
    }
}

/// Low-level operations client wrapping `LLMCore`.
///
/// Provides direct access to provider APIs for model listing, raw responses,
/// and batch operations.
pub struct InternalOps {
    core: LLMCore,
}

impl InternalOps {
    /// Create a new `InternalOps` wrapping the given `LLMCore`.
    pub fn new(core: LLMCore) -> Self {
        Self { core }
    }

    /// Access the underlying `LLMCore`.
    pub fn core(&self) -> &LLMCore {
        &self.core
    }

    /// Access the underlying `LLMCore` mutably.
    pub fn core_mut(&mut self) -> &mut LLMCore {
        &mut self.core
    }

    /// Resolve provider name, falling back to the core's default provider.
    fn resolve_provider(&self, provider: Option<&str>) -> String {
        provider
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.core.provider().to_string())
    }

    /// Resolve API base URL for a provider.
    fn resolve_base(&self, provider: &str) -> String {
        self.core
            .resolve_api_base(provider)
            .unwrap_or_else(|| default_api_base(provider))
    }

    /// Resolve API key for a provider, returning an error if not found.
    fn resolve_key(&self, provider: &str) -> Result<String, ConduitError> {
        self.core.resolve_api_key(provider).ok_or_else(|| {
            ConduitError::new(
                ErrorKind::Config,
                format!("No API key found for provider '{provider}'"),
            )
        })
    }

    /// Get an HTTP client for the provider.
    fn get_client(&mut self, provider: &str) -> Arc<reqwest::Client> {
        self.core.get_client(provider)
    }

    /// Build the URL for a given provider and path suffix (e.g. "/models").
    pub fn build_url(base: &str, path: &str) -> String {
        let base = base.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        format!("{base}/{path}")
    }

    /// List available models from a provider.
    pub async fn list_models(&mut self, provider: Option<&str>) -> Result<Value, ConduitError> {
        let prov = self.resolve_provider(provider);
        let base = self.resolve_base(&prov);
        let api_key = self.resolve_key(&prov)?;
        let client = self.get_client(&prov);

        let url = Self::build_url(&base, "/models");
        let resp = client
            .get(&url)
            .bearer_auth(&api_key)
            .send()
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Provider, format!("HTTP request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ConduitError::new(
                ErrorKind::Provider,
                format!("HTTP {status}: {text}"),
            ));
        }

        resp.json::<Value>().await.map_err(|e| {
            ConduitError::new(
                ErrorKind::Provider,
                format!("Failed to parse list_models response: {e}"),
            )
        })
    }

    /// Send a raw responses-format request.
    pub async fn responses(
        &mut self,
        input: Value,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> Result<Value, ConduitError> {
        let prov = self.resolve_provider(provider);
        let base = self.resolve_base(&prov);
        let api_key = self.resolve_key(&prov)?;
        let client = self.get_client(&prov);

        let mdl = model
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.core.model().to_string());

        let url = Self::build_url(&base, "/responses");
        let body = serde_json::json!({
            "model": mdl,
            "input": input,
        });

        let resp = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Provider, format!("HTTP request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ConduitError::new(
                ErrorKind::Provider,
                format!("HTTP {status}: {text}"),
            ));
        }

        resp.json::<Value>().await.map_err(|e| {
            ConduitError::new(
                ErrorKind::Provider,
                format!("Failed to parse responses response: {e}"),
            )
        })
    }

    /// Create a batch job.
    pub async fn create_batch(
        &mut self,
        input_file_path: &str,
        endpoint: &str,
        completion_window: &str,
        metadata: Option<Value>,
        provider: Option<&str>,
    ) -> Result<Value, ConduitError> {
        let prov = self.resolve_provider(provider);
        let base = self.resolve_base(&prov);
        let api_key = self.resolve_key(&prov)?;
        let client = self.get_client(&prov);

        let url = Self::build_url(&base, "/batches");
        let mut body = serde_json::json!({
            "input_file_id": input_file_path,
            "endpoint": endpoint,
            "completion_window": completion_window,
        });

        if let Some(meta) = metadata {
            body.as_object_mut()
                .unwrap()
                .insert("metadata".to_owned(), meta);
        }

        let resp = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Provider, format!("HTTP request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ConduitError::new(
                ErrorKind::Provider,
                format!("HTTP {status}: {text}"),
            ));
        }

        resp.json::<Value>().await.map_err(|e| {
            ConduitError::new(
                ErrorKind::Provider,
                format!("Failed to parse create_batch response: {e}"),
            )
        })
    }

    /// Retrieve batch status.
    pub async fn retrieve_batch(
        &mut self,
        batch_id: &str,
        provider: Option<&str>,
    ) -> Result<Value, ConduitError> {
        let prov = self.resolve_provider(provider);
        let base = self.resolve_base(&prov);
        let api_key = self.resolve_key(&prov)?;
        let client = self.get_client(&prov);

        let url = Self::build_url(&base, &format!("/batches/{batch_id}"));

        let resp = client
            .get(&url)
            .bearer_auth(&api_key)
            .send()
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Provider, format!("HTTP request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ConduitError::new(
                ErrorKind::Provider,
                format!("HTTP {status}: {text}"),
            ));
        }

        resp.json::<Value>().await.map_err(|e| {
            ConduitError::new(
                ErrorKind::Provider,
                format!("Failed to parse retrieve_batch response: {e}"),
            )
        })
    }

    /// Cancel a batch.
    pub async fn cancel_batch(
        &mut self,
        batch_id: &str,
        provider: Option<&str>,
    ) -> Result<Value, ConduitError> {
        let prov = self.resolve_provider(provider);
        let base = self.resolve_base(&prov);
        let api_key = self.resolve_key(&prov)?;
        let client = self.get_client(&prov);

        let url = Self::build_url(&base, &format!("/batches/{batch_id}/cancel"));

        let resp = client
            .post(&url)
            .bearer_auth(&api_key)
            .send()
            .await
            .map_err(|e| {
                ConduitError::new(ErrorKind::Provider, format!("HTTP request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ConduitError::new(
                ErrorKind::Provider,
                format!("HTTP {status}: {text}"),
            ));
        }

        resp.json::<Value>().await.map_err(|e| {
            ConduitError::new(
                ErrorKind::Provider,
                format!("Failed to parse cancel_batch response: {e}"),
            )
        })
    }

    /// List batches.
    pub async fn list_batches(
        &mut self,
        provider: Option<&str>,
        after: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Value, ConduitError> {
        let prov = self.resolve_provider(provider);
        let base = self.resolve_base(&prov);
        let api_key = self.resolve_key(&prov)?;
        let client = self.get_client(&prov);

        let url = Self::build_url(&base, "/batches");

        let mut request = client.get(&url).bearer_auth(&api_key);
        if let Some(after_val) = after {
            request = request.query(&[("after", after_val)]);
        }
        if let Some(limit_val) = limit {
            request = request.query(&[("limit", limit_val.to_string())]);
        }

        let resp = request.send().await.map_err(|e| {
            ConduitError::new(ErrorKind::Provider, format!("HTTP request failed: {e}"))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ConduitError::new(
                ErrorKind::Provider,
                format!("HTTP {status}: {text}"),
            ));
        }

        resp.json::<Value>().await.map_err(|e| {
            ConduitError::new(
                ErrorKind::Provider,
                format!("Failed to parse list_batches response: {e}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::parsing::TransportKind;
    use crate::core::execution::{ApiBaseConfig, ApiKeyConfig};

    fn make_core() -> LLMCore {
        LLMCore::new(
            "openai".into(),
            "gpt-4o".into(),
            vec![],
            3,
            ApiKeyConfig::Single("test-key".into()),
            ApiBaseConfig::None,
            TransportKind::Completion,
            0,
        )
    }

    fn make_core_with_base(base: &str) -> LLMCore {
        LLMCore::new(
            "openai".into(),
            "gpt-4o".into(),
            vec![],
            3,
            ApiKeyConfig::Single("test-key".into()),
            ApiBaseConfig::Single(base.into()),
            TransportKind::Completion,
            0,
        )
    }

    // ----- URL building -----

    #[test]
    fn test_build_url_models() {
        let url = InternalOps::build_url("https://api.openai.com/v1", "/models");
        assert_eq!(url, "https://api.openai.com/v1/models");
    }

    #[test]
    fn test_build_url_responses() {
        let url = InternalOps::build_url("https://api.openai.com/v1", "/responses");
        assert_eq!(url, "https://api.openai.com/v1/responses");
    }

    #[test]
    fn test_build_url_batches() {
        let url = InternalOps::build_url("https://api.openai.com/v1", "/batches");
        assert_eq!(url, "https://api.openai.com/v1/batches");
    }

    #[test]
    fn test_build_url_batch_by_id() {
        let url = InternalOps::build_url("https://api.openai.com/v1", "/batches/batch_123");
        assert_eq!(url, "https://api.openai.com/v1/batches/batch_123");
    }

    #[test]
    fn test_build_url_batch_cancel() {
        let url = InternalOps::build_url("https://api.openai.com/v1", "/batches/batch_123/cancel");
        assert_eq!(url, "https://api.openai.com/v1/batches/batch_123/cancel");
    }

    #[test]
    fn test_build_url_trailing_slash() {
        let url = InternalOps::build_url("https://api.openai.com/v1/", "/models");
        assert_eq!(url, "https://api.openai.com/v1/models");
    }

    #[test]
    fn test_build_url_no_leading_slash() {
        let url = InternalOps::build_url("https://api.openai.com/v1", "models");
        assert_eq!(url, "https://api.openai.com/v1/models");
    }

    // ----- Provider / base resolution -----

    #[test]
    fn test_resolve_provider_default() {
        let ops = InternalOps::new(make_core());
        assert_eq!(ops.resolve_provider(None), "openai");
    }

    #[test]
    fn test_resolve_provider_override() {
        let ops = InternalOps::new(make_core());
        assert_eq!(ops.resolve_provider(Some("anthropic")), "anthropic");
    }

    #[test]
    fn test_resolve_base_default() {
        let ops = InternalOps::new(make_core());
        assert_eq!(ops.resolve_base("openai"), "https://api.openai.com/v1");
    }

    #[test]
    fn test_resolve_base_custom() {
        let ops = InternalOps::new(make_core_with_base("https://custom.example.com"));
        assert_eq!(ops.resolve_base("openai"), "https://custom.example.com");
    }

    #[test]
    fn test_resolve_key_present() {
        let ops = InternalOps::new(make_core());
        assert_eq!(ops.resolve_key("openai").unwrap(), "test-key");
    }

    #[test]
    fn test_resolve_key_missing() {
        let core = LLMCore::new(
            "openai".into(),
            "gpt-4o".into(),
            vec![],
            3,
            ApiKeyConfig::None,
            ApiBaseConfig::None,
            TransportKind::Completion,
            0,
        );
        let ops = InternalOps::new(core);
        let err = ops.resolve_key("openai").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Config);
        assert!(err.message.contains("No API key"));
    }

    // ----- Default API base for different providers -----

    #[test]
    fn test_default_api_base_openai() {
        assert_eq!(default_api_base("openai"), "https://api.openai.com/v1");
    }

    #[test]
    fn test_default_api_base_anthropic() {
        assert_eq!(
            default_api_base("anthropic"),
            "https://api.anthropic.com/v1"
        );
    }

    #[test]
    fn test_default_api_base_other() {
        assert_eq!(default_api_base("cohere"), "https://api.cohere.com/v1");
    }
}
