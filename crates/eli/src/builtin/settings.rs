//! Agent configuration loaded from environment variables.

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

pub use conduit::core::execution::{ApiBaseConfig, ApiKeyConfig};
pub use conduit::llm::ApiFormat;

/// Default model identifier.
pub const DEFAULT_MODEL: &str = "openrouter:qwen/qwen3-coder-next";
/// Default maximum tokens per model response.
pub const DEFAULT_MAX_TOKENS: usize = 1024;

/// Return the default home directory (`~/.eli`).
fn default_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".eli")
}

/// Agent configuration, loaded from environment variables with the `ELI_` prefix
/// and optionally from a `.env` file via `dotenvy`.
#[derive(Debug, Clone)]
pub struct AgentSettings {
    pub home: PathBuf,
    pub model: String,
    pub fallback_models: Option<Vec<String>>,
    pub api_key: ApiKeyConfig,
    pub api_base: ApiBaseConfig,
    pub api_format: ApiFormat,
    pub max_steps: usize,
    pub max_tokens: usize,
    pub model_timeout_seconds: Option<u64>,
    pub verbose: u8,
}

fn api_format_from_str_lossy(s: &str) -> ApiFormat {
    match s.trim().to_lowercase().as_str() {
        "auto" => ApiFormat::Auto,
        "responses" => ApiFormat::Responses,
        "messages" => ApiFormat::Messages,
        "completion" => ApiFormat::Completion,
        _ => ApiFormat::Auto,
    }
}

impl AgentSettings {
    /// Load settings from environment (and `.env` file).
    ///
    /// Per-provider API keys are detected via `ELI_<PROVIDER>_API_KEY` and
    /// `ELI_<PROVIDER>_API_BASE` patterns, matching the Python implementation.
    pub fn from_env() -> Self {
        // Best-effort load of a .env file in the current directory.
        let _ = dotenvy::dotenv();

        let home = env::var("ELI_HOME")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(default_home);

        let model = env::var("ELI_MODEL").unwrap_or_else(|_| {
            // Try loading model from ~/.eli/config.toml active profile.
            let config = crate::builtin::config::EliConfig::load();
            config
                .resolve_model()
                .unwrap_or_else(|| DEFAULT_MODEL.to_owned())
        });

        let fallback_models = env::var("ELI_FALLBACK_MODELS").ok().map(|v| {
            v.split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        });

        let api_format = api_format_from_str_lossy(&env::var("ELI_API_FORMAT").unwrap_or_default());

        let max_steps: usize = env::var("ELI_MAX_STEPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);

        let max_tokens: usize = env::var("ELI_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_TOKENS);

        let model_timeout_seconds: Option<u64> = env::var("ELI_MODEL_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok());

        let verbose: u8 = env::var("ELI_VERBOSE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
            .min(2);

        // Resolve API key / base — single value or per-provider map.
        let single_key = env::var("ELI_API_KEY").ok();
        let single_base = env::var("ELI_API_BASE").ok();

        let (api_key, api_base) =
            if let (Some(key), Some(base)) = (single_key.clone(), single_base.clone()) {
                (ApiKeyConfig::Single(key), ApiBaseConfig::Single(base))
            } else {
                let mut key_map: HashMap<String, String> = HashMap::new();
                let mut base_map: HashMap<String, String> = HashMap::new();

                if let Some(k) = &single_key {
                    key_map.insert("default".to_owned(), k.clone());
                }
                if let Some(b) = &single_base {
                    base_map.insert("default".to_owned(), b.clone());
                }

                for (key, value) in env::vars() {
                    if let Some(provider) = key
                        .strip_prefix("ELI_")
                        .and_then(|rest| rest.strip_suffix("_API_KEY"))
                        && provider != "API"
                    {
                        key_map.insert(provider.to_lowercase(), value.clone());
                    }
                    if let Some(provider) = key
                        .strip_prefix("ELI_")
                        .and_then(|rest| rest.strip_suffix("_API_BASE"))
                        && provider != "API"
                    {
                        base_map.insert(provider.to_lowercase(), value);
                    }
                }

                let api_key = if key_map.is_empty() {
                    ApiKeyConfig::None
                } else if key_map.len() == 1 && key_map.contains_key("default") {
                    ApiKeyConfig::Single(key_map.remove("default").unwrap())
                } else {
                    ApiKeyConfig::PerProvider(key_map)
                };

                let api_base = if base_map.is_empty() {
                    ApiBaseConfig::None
                } else if base_map.len() == 1 && base_map.contains_key("default") {
                    ApiBaseConfig::Single(base_map.remove("default").unwrap())
                } else {
                    ApiBaseConfig::PerProvider(base_map)
                };

                (api_key, api_base)
            };

        Self {
            home,
            model,
            fallback_models,
            api_key,
            api_base,
            api_format,
            max_steps,
            max_tokens,
            model_timeout_seconds,
            verbose,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- ApiFormat tests ------------------------------------------------------

    #[test]
    fn test_api_format_from_str_lossy() {
        assert_eq!(
            api_format_from_str_lossy("completion"),
            ApiFormat::Completion
        );
        assert_eq!(api_format_from_str_lossy("auto"), ApiFormat::Auto);
        assert_eq!(api_format_from_str_lossy("responses"), ApiFormat::Responses);
        assert_eq!(api_format_from_str_lossy("messages"), ApiFormat::Messages);
        assert_eq!(api_format_from_str_lossy("RESPONSES"), ApiFormat::Responses);
        assert_eq!(api_format_from_str_lossy("unknown"), ApiFormat::Auto);
        assert_eq!(api_format_from_str_lossy(""), ApiFormat::Auto);
    }

    #[test]
    fn test_api_format_as_str() {
        assert_eq!(ApiFormat::Auto.as_str(), "auto");
        assert_eq!(ApiFormat::Completion.as_str(), "completion");
        assert_eq!(ApiFormat::Responses.as_str(), "responses");
        assert_eq!(ApiFormat::Messages.as_str(), "messages");
    }

    // -- ApiKeyConfig / ApiBaseConfig -----------------------------------------

    #[test]
    fn test_api_key_config_single() {
        let config = ApiKeyConfig::Single("sk-test".into());
        match config {
            ApiKeyConfig::Single(k) => assert_eq!(k, "sk-test"),
            _ => panic!("expected Single"),
        }
    }

    #[test]
    fn test_api_key_config_per_provider() {
        let mut map = HashMap::new();
        map.insert("openai".into(), "sk-openai".into());
        let config = ApiKeyConfig::PerProvider(map);
        match config {
            ApiKeyConfig::PerProvider(m) => assert_eq!(m["openai"], "sk-openai"),
            _ => panic!("expected PerProvider"),
        }
    }

    // -- AgentSettings defaults (from_env with defaults) ----------------------

    // Note: from_env reads actual env vars, so we test the default logic paths
    // by checking struct field types and default values.

    #[test]
    fn test_default_model_constant() {
        assert!(!DEFAULT_MODEL.is_empty());
    }

    #[test]
    fn test_default_max_tokens_constant() {
        assert!(DEFAULT_MAX_TOKENS > 0);
    }

    #[test]
    fn test_default_home_returns_path() {
        let home = default_home();
        // Should end with .eli
        assert!(home.ends_with(".eli"));
    }

    #[test]
    fn test_agent_settings_clone() {
        let settings = AgentSettings {
            home: PathBuf::from("/tmp"),
            model: "test-model".into(),
            fallback_models: Some(vec!["fallback1".into()]),
            api_key: ApiKeyConfig::Single("sk-test".into()),
            api_base: ApiBaseConfig::None,
            api_format: ApiFormat::Completion,
            max_steps: 10,
            max_tokens: 512,
            model_timeout_seconds: Some(30),
            verbose: 1,
        };
        let cloned = settings.clone();
        assert_eq!(cloned.model, "test-model");
        assert_eq!(cloned.max_steps, 10);
        assert_eq!(cloned.max_tokens, 512);
        assert_eq!(cloned.verbose, 1);
    }
}
