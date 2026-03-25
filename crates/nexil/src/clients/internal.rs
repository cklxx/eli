//! InternalOps client for direct provider API access.

use serde_json::Value;
use std::sync::Arc;

use crate::core::errors::{ConduitError, ErrorKind};
use crate::core::execution::LLMCore;
use crate::llm::default_api_base;

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

    fn resolve_provider(&self, provider: Option<&str>) -> String {
        provider.unwrap_or(self.core.provider()).to_owned()
    }

    fn resolve_base(&self, provider: &str) -> String {
        self.core
            .resolve_api_base(provider)
            .unwrap_or_else(|| default_api_base(provider))
    }

    fn resolve_key(&self, provider: &str) -> Result<String, ConduitError> {
        self.core.resolve_api_key(provider).ok_or_else(|| {
            ConduitError::new(
                ErrorKind::Config,
                format!("No API key found for provider '{provider}'"),
            )
        })
    }

    fn get_client(&mut self, provider: &str) -> Arc<reqwest::Client> {
        self.core.get_client(provider)
    }

    /// Build the URL for a given provider and path suffix (e.g. "/models").
    pub fn build_url(base: &str, path: &str) -> String {
        let base = base.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        format!("{base}/{path}")
    }

    async fn send_and_parse(
        resp: reqwest::Response,
        operation: &str,
    ) -> Result<Value, ConduitError> {
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
                format!("Failed to parse {operation} response: {e}"),
            )
        })
    }

    async fn send_request(
        request: reqwest::RequestBuilder,
        operation: &str,
    ) -> Result<Value, ConduitError> {
        let resp = request.send().await.map_err(|e| {
            ConduitError::new(ErrorKind::Provider, format!("HTTP request failed: {e}"))
        })?;
        Self::send_and_parse(resp, operation).await
    }

    /// List available models from a provider.
    pub async fn list_models(&mut self, provider: Option<&str>) -> Result<Value, ConduitError> {
        let prov = self.resolve_provider(provider);
        let base = self.resolve_base(&prov);
        let api_key = self.resolve_key(&prov)?;
        let client = self.get_client(&prov);
        let url = Self::build_url(&base, "/models");
        Self::send_request(client.get(&url).bearer_auth(&api_key), "list_models").await
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

        let mdl = model.unwrap_or(self.core.model());
        let url = Self::build_url(&base, "/responses");
        let body = serde_json::json!({ "model": mdl, "input": input });

        Self::send_request(
            client.post(&url).bearer_auth(&api_key).json(&body),
            "responses",
        )
        .await
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
                .expect("SAFETY: json! above produces an object")
                .insert("metadata".to_owned(), meta);
        }

        Self::send_request(
            client.post(&url).bearer_auth(&api_key).json(&body),
            "create_batch",
        )
        .await
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
        Self::send_request(client.get(&url).bearer_auth(&api_key), "retrieve_batch").await
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
        Self::send_request(client.post(&url).bearer_auth(&api_key), "cancel_batch").await
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
        Self::send_request(request, "list_batches").await
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
