//! Provider-specific client creation and caching helpers.

use std::collections::HashMap;
use std::sync::Arc;

use reqwest::Client;

/// A key that uniquely identifies a cached HTTP client instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientCacheKey {
    pub provider: String,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
}

impl ClientCacheKey {
    pub fn new(provider: &str, api_key: Option<&str>, api_base: Option<&str>) -> Self {
        Self {
            provider: provider.to_owned(),
            api_key: api_key.map(|s| s.to_owned()),
            api_base: api_base.map(|s| s.to_owned()),
        }
    }
}

/// Configuration for creating provider HTTP clients.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Default headers to include in every request.
    pub default_headers: HashMap<String, String>,
    /// Connection timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            default_headers: HashMap::new(),
            timeout_secs: 120,
        }
    }
}

/// Registry that caches `reqwest::Client` instances by (provider, key, base) tuple.
#[derive(Debug, Clone)]
pub struct ClientRegistry {
    cache: HashMap<ClientCacheKey, Arc<Client>>,
    config: ClientConfig,
}

impl ClientRegistry {
    /// Create a new empty registry with default config.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            config: ClientConfig::default(),
        }
    }

    /// Create a new registry with the given client config.
    pub fn with_config(config: ClientConfig) -> Self {
        Self {
            cache: HashMap::new(),
            config,
        }
    }

    /// Get or create a reqwest `Client` for the given provider/key/base combination.
    pub fn get_or_create(
        &mut self,
        provider: &str,
        api_key: Option<&str>,
        api_base: Option<&str>,
    ) -> Arc<Client> {
        let cache_key = ClientCacheKey::new(provider, api_key, api_base);
        if let Some(client) = self.cache.get(&cache_key) {
            return Arc::clone(client);
        }

        let client = Arc::new(self.build_client(provider, api_key));
        self.cache.insert(cache_key, Arc::clone(&client));
        client
    }

    /// Check if a client is already cached for the given key.
    pub fn contains(&self, provider: &str, api_key: Option<&str>, api_base: Option<&str>) -> bool {
        let cache_key = ClientCacheKey::new(provider, api_key, api_base);
        self.cache.contains_key(&cache_key)
    }

    /// Remove all cached clients.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Number of cached clients.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    fn build_client(&self, provider: &str, api_key: Option<&str>) -> Client {
        let mut headers = reqwest::header::HeaderMap::new();

        for (key, value) in &self.config.default_headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                headers.insert(name, val);
            }
        }

        let is_anthropic = provider.eq_ignore_ascii_case("anthropic");
        let is_oauth_token = api_key
            .map(|k| k.to_ascii_lowercase().starts_with("sk-ant-oat"))
            .unwrap_or(false);

        if let Some(key) = api_key {
            if is_anthropic && !is_oauth_token {
                // Anthropic API keys use x-api-key header.
                if let Ok(val) = reqwest::header::HeaderValue::from_str(key)
                    && let Ok(name) = reqwest::header::HeaderName::from_bytes(b"x-api-key")
                {
                    headers.insert(name, val);
                }
            } else {
                // All other providers (and Anthropic OAuth tokens) use Bearer auth.
                if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
                    headers.insert(reqwest::header::AUTHORIZATION, val);
                }
            }
        }

        // Add anthropic-version header for Anthropic provider.
        if is_anthropic
            && let Ok(val) = reqwest::header::HeaderValue::from_str("2023-06-01")
            && let Ok(name) = reqwest::header::HeaderName::from_bytes(b"anthropic-version")
        {
            headers.insert(name, val);
        }

        // Anthropic OAuth tokens require Claude Code CLI impersonation headers.
        if is_anthropic && is_oauth_token {
            headers.insert(
                reqwest::header::USER_AGENT,
                reqwest::header::HeaderValue::from_static("claude-cli/2.1.75"),
            );
            if let Ok(name) = reqwest::header::HeaderName::from_bytes(b"x-app") {
                headers.insert(name, reqwest::header::HeaderValue::from_static("cli"));
            }
            headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json"),
            );
            if let Ok(name) = reqwest::header::HeaderName::from_bytes(
                b"anthropic-dangerous-direct-browser-access",
            ) {
                headers.insert(name, reqwest::header::HeaderValue::from_static("true"));
            }
            if let Ok(name) = reqwest::header::HeaderName::from_bytes(b"anthropic-beta") {
                headers.insert(
                    name,
                    reqwest::header::HeaderValue::from_static(
                        "claude-code-20250219,oauth-2025-04-20,fine-grained-tool-streaming-2025-05-14,interleaved-thinking-2025-05-14"
                    ),
                );
            }
        }

        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(self.config.timeout_secs))
            .build()
            .unwrap_or_else(|err| {
                tracing::warn!(%err, provider, "failed to build HTTP client with custom config, falling back to default");
                Client::new()
            })
    }
}

impl Default for ClientRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_equality() {
        let k1 = ClientCacheKey::new("openai", Some("sk-123"), None);
        let k2 = ClientCacheKey::new("openai", Some("sk-123"), None);
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_get_or_create_caches() {
        let mut registry = ClientRegistry::new();
        let c1 = registry.get_or_create("openai", Some("sk-123"), None);
        let c2 = registry.get_or_create("openai", Some("sk-123"), None);
        assert!(Arc::ptr_eq(&c1, &c2));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_different_keys_separate_clients() {
        let mut registry = ClientRegistry::new();
        let c1 = registry.get_or_create("openai", Some("sk-111"), None);
        let c2 = registry.get_or_create("openai", Some("sk-222"), None);
        assert!(!Arc::ptr_eq(&c1, &c2));
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn test_clear() {
        let mut registry = ClientRegistry::new();
        registry.get_or_create("openai", Some("sk-123"), None);
        assert!(!registry.is_empty());
        registry.clear();
        assert!(registry.is_empty());
    }
}
