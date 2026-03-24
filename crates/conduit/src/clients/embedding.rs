//! Embedding helpers for Conduit.

use serde_json::Value;

use crate::core::errors::{ConduitError, ErrorKind};
use crate::core::execution::LLMCore;
use crate::core::results::ErrorPayload;

/// Lightweight embedding helper.
pub struct EmbeddingClient {
    core: LLMCore,
}

impl EmbeddingClient {
    /// Create a new `EmbeddingClient` wrapping an `LLMCore`.
    pub fn new(core: LLMCore) -> Self {
        Self { core }
    }

    /// Access the inner `LLMCore`.
    pub fn core(&self) -> &LLMCore {
        &self.core
    }

    /// Resolve the provider and model for an embedding call.
    fn resolve_provider_model(
        &self,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> Result<(String, String), ConduitError> {
        if model.is_none() && provider.is_none() {
            return Ok((
                self.core.provider().to_owned(),
                self.core.model().to_owned(),
            ));
        }
        let model_id = model.unwrap_or(self.core.model());
        LLMCore::resolve_model_provider(model_id, provider)
    }

    /// Build the request URL for embeddings.
    fn build_url(api_base: &str) -> String {
        let base = api_base.trim_end_matches('/');
        format!("{}/embeddings", base)
    }

    /// Execute an embedding request.
    ///
    /// `inputs` can be a single string or a JSON array of strings.
    pub async fn embed(
        &mut self,
        inputs: Value,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> Result<Value, ErrorPayload> {
        let (provider_name, model_id) = self
            .resolve_provider_model(model, provider)
            .map_err(|e| ErrorPayload::new(e.kind, e.message))?;

        let api_key = self.core.resolve_api_key(&provider_name).ok_or_else(|| {
            ErrorPayload::new(
                ErrorKind::Config,
                format!("No API key found for provider '{provider_name}'"),
            )
        })?;

        let client = self.core.get_client(&provider_name);
        let api_base = self
            .core
            .resolve_api_base(&provider_name)
            .unwrap_or_else(|| "https://api.openai.com/v1".to_owned());
        let url = Self::build_url(&api_base);

        // Normalize input
        let input_value = match &inputs {
            Value::String(s) => Value::Array(vec![Value::String(s.clone())]),
            Value::Array(_) => inputs.clone(),
            _ => {
                return Err(ErrorPayload::new(
                    ErrorKind::InvalidInput,
                    "inputs must be a string or array of strings.",
                ));
            }
        };

        let body = serde_json::json!({
            "model": model_id,
            "input": input_value,
        });

        let response = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ErrorPayload::new(
                    ErrorKind::Provider,
                    format!("{}:{}: {}", provider_name, model_id, e),
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            let kind =
                LLMCore::classify_http_status(status.as_u16()).unwrap_or(ErrorKind::Provider);
            return Err(ErrorPayload::new(
                kind,
                format!(
                    "{}:{}: HTTP {} - {}",
                    provider_name, model_id, status, error_body
                ),
            ));
        }

        let payload: Value = response.json().await.map_err(|e| {
            ErrorPayload::new(
                ErrorKind::Provider,
                format!(
                    "{}:{}: failed to parse response: {}",
                    provider_name, model_id, e
                ),
            )
        })?;

        Ok(payload)
    }

    /// Extract just the embedding vectors from a full API response.
    pub fn extract_embeddings(response: &Value) -> Vec<Vec<f64>> {
        let data = match response.get("data").and_then(|d| d.as_array()) {
            Some(arr) => arr,
            None => return Vec::new(),
        };

        data.iter()
            .filter_map(|item| {
                item.get("embedding")
                    .and_then(|e| e.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect::<Vec<f64>>())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_build_url() {
        assert_eq!(
            EmbeddingClient::build_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/embeddings"
        );
        assert_eq!(
            EmbeddingClient::build_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/embeddings"
        );
    }

    #[test]
    fn test_extract_embeddings() {
        let response = json!({
            "data": [
                {"embedding": [0.1, 0.2, 0.3]},
                {"embedding": [0.4, 0.5, 0.6]}
            ]
        });
        let embeddings = EmbeddingClient::extract_embeddings(&response);
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0], vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn test_extract_embeddings_empty() {
        let response = json!({});
        let embeddings = EmbeddingClient::extract_embeddings(&response);
        assert!(embeddings.is_empty());
    }
}
