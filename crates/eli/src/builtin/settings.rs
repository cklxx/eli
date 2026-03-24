//! Agent configuration loaded from environment variables.

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

pub use conduit::core::execution::{ApiBaseConfig, ApiKeyConfig};
pub use conduit::llm::ApiFormat;

/// Default model identifier.
pub const DEFAULT_MODEL: &str = "openrouter:qwen/qwen3-coder-next";
/// Fallback maximum output tokens when we cannot infer from the model name.
pub const DEFAULT_MAX_OUTPUT_TOKENS: usize = 65_536;
/// Fallback context window when we cannot infer from the model name.
pub const DEFAULT_CONTEXT_WINDOW: usize = 128_000;

// ---------------------------------------------------------------------------
// Model spec table
// ---------------------------------------------------------------------------

/// Match strategy for a model pattern.
#[derive(Clone, Copy)]
enum Match {
    /// Pattern is a substring of the model ID (default, most entries).
    Contains,
    /// Pattern must appear at the start of the model ID (avoids false positives
    /// for short patterns like "o3").
    Prefix,
}

/// Known model family spec: (pattern, match strategy, context_window, max_output_tokens).
///
/// Order matters — first match wins. More specific patterns must come before
/// generic catch-alls. Matching is case-insensitive against the model_id part
/// (provider prefix stripped).
const MODEL_SPECS: &[(&str, Match, usize, usize)] = &[
    // --- Anthropic Claude ---------------------------------------------------
    ("claude-opus-4-6", Match::Contains, 200_000, 128_000),
    ("claude-sonnet-4-6", Match::Contains, 200_000, 64_000),
    ("claude-sonnet-4-5", Match::Contains, 200_000, 64_000),
    ("claude-haiku-4-5", Match::Contains, 200_000, 16_384),
    ("claude-opus-4", Match::Contains, 200_000, 32_000),
    ("claude-sonnet-4", Match::Contains, 200_000, 64_000),
    ("claude-3-5-sonnet", Match::Contains, 200_000, 8_192),
    ("claude-3-5-haiku", Match::Contains, 200_000, 8_192),
    ("claude-3-opus", Match::Contains, 200_000, 4_096),
    ("claude-3-sonnet", Match::Contains, 200_000, 4_096),
    ("claude-3-haiku", Match::Contains, 200_000, 4_096),
    ("claude", Match::Contains, 200_000, 64_000),
    // --- OpenAI -------------------------------------------------------------
    ("o3", Match::Prefix, 200_000, 100_000),
    ("o4-mini", Match::Prefix, 200_000, 100_000),
    ("o1", Match::Prefix, 200_000, 100_000),
    ("gpt-4.1", Match::Contains, 1_048_576, 32_768),
    ("gpt-4o", Match::Contains, 128_000, 16_384),
    ("gpt-4-turbo", Match::Contains, 128_000, 4_096),
    ("gpt-4", Match::Contains, 8_192, 4_096),
    ("gpt-3.5", Match::Contains, 16_384, 4_096),
    // --- Google Gemini ------------------------------------------------------
    ("gemini-2.5", Match::Contains, 1_048_576, 65_536),
    ("gemini-2.0", Match::Contains, 1_048_576, 8_192),
    ("gemini-1.5", Match::Contains, 1_048_576, 8_192),
    ("gemini", Match::Contains, 128_000, 8_192),
    // --- DeepSeek -----------------------------------------------------------
    ("deepseek-reasoner", Match::Contains, 164_000, 65_536),
    ("deepseek-r1", Match::Contains, 164_000, 65_536),
    ("deepseek", Match::Contains, 128_000, 8_192),
    // --- Qwen (Alibaba) ----------------------------------------------------
    ("qwen3.5", Match::Contains, 1_000_000, 65_536),
    ("qwen3-coder", Match::Contains, 1_000_000, 65_536),
    ("qwen3", Match::Contains, 1_000_000, 65_536),
    ("qwen", Match::Contains, 128_000, 32_768),
    // --- Kimi (Moonshot AI) -------------------------------------------------
    ("kimi-k2", Match::Contains, 262_144, 65_536),
    ("moonshot", Match::Contains, 128_000, 16_384),
    // --- GLM (Zhipu AI) -----------------------------------------------------
    ("glm-5", Match::Contains, 200_000, 128_000),
    ("glm-4", Match::Contains, 128_000, 4_096),
    ("glm", Match::Contains, 128_000, 4_096),
    // --- MiniMax ------------------------------------------------------------
    ("minimax-m2", Match::Contains, 204_800, 131_072),
    ("minimax-text-01", Match::Contains, 4_000_000, 204_800),
    ("minimax", Match::Contains, 204_800, 65_536),
    // --- Llama (Meta) -------------------------------------------------------
    ("llama-4-scout", Match::Contains, 10_000_000, 16_384),
    ("llama-4", Match::Contains, 1_048_576, 16_384),
    ("llama-3", Match::Contains, 128_000, 8_192),
    ("llama", Match::Contains, 128_000, 8_192),
    // --- Mistral / Codestral ------------------------------------------------
    ("mistral-large", Match::Contains, 128_000, 32_768),
    ("codestral", Match::Contains, 256_000, 32_768),
    ("mistral", Match::Contains, 32_000, 8_192),
];

// ---------------------------------------------------------------------------
// Inference helpers
// ---------------------------------------------------------------------------

/// Strip the `"provider:"` prefix from a model string, returning just the
/// model ID portion. If no colon is present, returns the full string.
///
/// Examples:
/// - `"anthropic:claude-sonnet-4-6"` → `"claude-sonnet-4-6"`
/// - `"openrouter:qwen/qwen3-coder-next"` → `"qwen/qwen3-coder-next"`
/// - `"claude-sonnet-4-6"` → `"claude-sonnet-4-6"`
fn strip_provider(model: &str) -> &str {
    model.split_once(':').map_or(model, |(_, id)| id)
}

/// Look up model specs from `MODEL_SPECS` table. Returns
/// `(context_window, max_output_tokens)` or `None` if no match.
fn lookup_model_spec(model: &str) -> Option<(usize, usize)> {
    let id = strip_provider(model).to_lowercase();
    // For OpenRouter paths like "qwen/qwen3-coder-next", also try the part
    // after the last slash.
    let slug = id.rsplit('/').next().unwrap_or(&id);

    for &(pattern, strategy, ctx, out) in MODEL_SPECS {
        let matched = match strategy {
            Match::Contains => id.contains(pattern) || slug.contains(pattern),
            Match::Prefix => id.starts_with(pattern) || slug.starts_with(pattern),
        };
        if matched {
            return Some((ctx, out));
        }
    }
    None
}

/// Infer context window size (in tokens) from a model name string.
fn infer_context_window(model: &str) -> usize {
    lookup_model_spec(model)
        .map(|(ctx, _)| ctx)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

/// Infer the maximum output tokens for a model from its name.
///
/// Returns the largest output-token limit the model supports so that tool
/// calls with large payloads (e.g. document creation) are not silently
/// truncated.
fn infer_max_output_tokens(model: &str) -> usize {
    lookup_model_spec(model)
        .map(|(_, out)| out)
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
}

/// Return the default home directory (`~/.eli`).
fn default_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".eli")
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

// ---------------------------------------------------------------------------
// API credential resolution
// ---------------------------------------------------------------------------

/// Resolve API key and base URL from environment variables.
///
/// Supports both single-value (`ELI_API_KEY` / `ELI_API_BASE`) and per-provider
/// (`ELI_<PROVIDER>_API_KEY` / `ELI_<PROVIDER>_API_BASE`) patterns.
fn resolve_api_credentials() -> (ApiKeyConfig, ApiBaseConfig) {
    let single_key = env::var("ELI_API_KEY").ok();
    let single_base = env::var("ELI_API_BASE").ok();

    if let (Some(key), Some(base)) = (single_key.clone(), single_base.clone()) {
        return (ApiKeyConfig::Single(key), ApiBaseConfig::Single(base));
    }

    let mut key_map: HashMap<String, String> = HashMap::new();
    let mut base_map: HashMap<String, String> = HashMap::new();

    if let Some(k) = single_key {
        key_map.insert("default".to_owned(), k);
    }
    if let Some(b) = single_base {
        base_map.insert("default".to_owned(), b);
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

    let api_key = collapse_config_map(key_map, ApiKeyConfig::None, ApiKeyConfig::Single, ApiKeyConfig::PerProvider);
    let api_base = collapse_config_map(base_map, ApiBaseConfig::None, ApiBaseConfig::Single, ApiBaseConfig::PerProvider);
    (api_key, api_base)
}

/// Collapse a `HashMap` into a config enum: empty → `none`, single "default"
/// entry → `single(value)`, otherwise → `per_provider(map)`.
fn collapse_config_map<T>(
    mut map: HashMap<String, String>,
    none: T,
    single: fn(String) -> T,
    per_provider: fn(HashMap<String, String>) -> T,
) -> T {
    if map.is_empty() {
        none
    } else if map.len() == 1 && map.contains_key("default") {
        single(map.remove("default").unwrap())
    } else {
        per_provider(map)
    }
}

// ---------------------------------------------------------------------------
// AgentSettings
// ---------------------------------------------------------------------------

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
    /// Context window size in tokens. Auto-detected from model name if not set
    /// via `ELI_CONTEXT_WINDOW`.
    pub context_window: usize,
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
            .unwrap_or_else(|| infer_max_output_tokens(&model));

        let model_timeout_seconds: Option<u64> = env::var("ELI_MODEL_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok());

        let verbose: u8 = env::var("ELI_VERBOSE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
            .min(2);

        let context_window: usize = env::var("ELI_CONTEXT_WINDOW")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| infer_context_window(&model));

        // Resolve API key / base — single value or per-provider map.
        let (api_key, api_base) = resolve_api_credentials();

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
            context_window,
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

    // -- AgentSettings defaults -----------------------------------------------

    #[test]
    fn test_default_model_constant() {
        assert!(!DEFAULT_MODEL.is_empty());
    }

    #[test]
    fn test_default_max_output_tokens_constant() {
        assert!(DEFAULT_MAX_OUTPUT_TOKENS > 0);
    }

    #[test]
    fn test_default_home_returns_path() {
        let home = default_home();
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
            context_window: 128_000,
        };
        let cloned = settings.clone();
        assert_eq!(cloned.model, "test-model");
        assert_eq!(cloned.max_steps, 10);
        assert_eq!(cloned.max_tokens, 512);
        assert_eq!(cloned.verbose, 1);
    }

    // -- strip_provider -------------------------------------------------------

    #[test]
    fn test_strip_provider() {
        assert_eq!(
            strip_provider("anthropic:claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            strip_provider("openrouter:qwen/qwen3-coder-next"),
            "qwen/qwen3-coder-next"
        );
        assert_eq!(strip_provider("claude-sonnet-4-6"), "claude-sonnet-4-6");
    }

    // -- infer_context_window ------------------------------------------------

    #[test]
    fn test_context_window_anthropic() {
        assert_eq!(infer_context_window("anthropic:claude-opus-4-6"), 200_000);
        assert_eq!(infer_context_window("anthropic:claude-sonnet-4-6"), 200_000);
        assert_eq!(
            infer_context_window("anthropic:claude-sonnet-4-20250514"),
            200_000
        );
        assert_eq!(
            infer_context_window("anthropic:claude-3-5-sonnet-20241022"),
            200_000
        );
        assert_eq!(
            infer_context_window("anthropic:claude-3-haiku-20240307"),
            200_000
        );
    }

    #[test]
    fn test_context_window_openai() {
        assert_eq!(infer_context_window("openai:gpt-4o-2024-08-06"), 128_000);
        assert_eq!(infer_context_window("openai:gpt-4.1-2025-04-14"), 1_048_576);
        assert_eq!(infer_context_window("openai:o3-2025-04-16"), 200_000);
        assert_eq!(infer_context_window("openai:o4-mini"), 200_000);
        assert_eq!(infer_context_window("openai:o1-2024-12-17"), 200_000);
        assert_eq!(infer_context_window("openai:gpt-3.5-turbo"), 16_384);
    }

    #[test]
    fn test_context_window_gemini() {
        assert_eq!(infer_context_window("google:gemini-2.5-pro"), 1_048_576);
        assert_eq!(infer_context_window("google:gemini-2.5-flash"), 1_048_576);
        assert_eq!(infer_context_window("google:gemini-1.5-flash"), 1_048_576);
    }

    #[test]
    fn test_context_window_chinese_models() {
        assert_eq!(infer_context_window("dashscope:qwen3.5-plus"), 1_000_000);
        assert_eq!(
            infer_context_window("openrouter:qwen/qwen3-coder-next"),
            1_000_000
        );
        assert_eq!(infer_context_window("moonshot:kimi-k2.5"), 262_144);
        assert_eq!(infer_context_window("zhipu:glm-5"), 200_000);
        assert_eq!(infer_context_window("minimax:minimax-text-01"), 4_000_000);
        assert_eq!(infer_context_window("minimax:minimax-m2.7"), 204_800);
    }

    #[test]
    fn test_context_window_others() {
        assert_eq!(infer_context_window("deepseek:deepseek-chat"), 128_000);
        assert_eq!(infer_context_window("deepseek:deepseek-reasoner"), 164_000);
        assert_eq!(infer_context_window("meta:llama-4-maverick"), 1_048_576);
        assert_eq!(infer_context_window("meta:llama-4-scout"), 10_000_000);
        assert_eq!(infer_context_window("meta:llama-3.1-70b"), 128_000);
        assert_eq!(
            infer_context_window("mistral:mistral-large-latest"),
            128_000
        );
        assert_eq!(infer_context_window("mistral:codestral-latest"), 256_000);
    }

    #[test]
    fn test_context_window_unknown_fallback() {
        assert_eq!(
            infer_context_window("unknown:some-model"),
            DEFAULT_CONTEXT_WINDOW
        );
    }

    // -- infer_max_output_tokens ---------------------------------------------

    #[test]
    fn test_max_output_anthropic() {
        assert_eq!(
            infer_max_output_tokens("anthropic:claude-opus-4-6"),
            128_000
        );
        assert_eq!(
            infer_max_output_tokens("anthropic:claude-sonnet-4-6"),
            64_000
        );
        assert_eq!(
            infer_max_output_tokens("anthropic:claude-opus-4-20250514"),
            32_000
        );
        assert_eq!(
            infer_max_output_tokens("anthropic:claude-sonnet-4-20250514"),
            64_000
        );
        assert_eq!(
            infer_max_output_tokens("anthropic:claude-3-5-sonnet-20241022"),
            8_192
        );
        assert_eq!(
            infer_max_output_tokens("anthropic:claude-3-haiku-20240307"),
            4_096
        );
    }

    #[test]
    fn test_max_output_openai() {
        assert_eq!(infer_max_output_tokens("openai:gpt-4o"), 16_384);
        assert_eq!(infer_max_output_tokens("openai:gpt-4.1"), 32_768);
        assert_eq!(infer_max_output_tokens("openai:o3"), 100_000);
        assert_eq!(infer_max_output_tokens("openai:o4-mini"), 100_000);
        assert_eq!(infer_max_output_tokens("openai:o1"), 100_000);
    }

    #[test]
    fn test_max_output_gemini() {
        assert_eq!(infer_max_output_tokens("google:gemini-2.5-pro"), 65_536);
        assert_eq!(infer_max_output_tokens("google:gemini-2.0-flash"), 8_192);
    }

    #[test]
    fn test_max_output_chinese_models() {
        assert_eq!(infer_max_output_tokens("dashscope:qwen3.5-plus"), 65_536);
        assert_eq!(
            infer_max_output_tokens("openrouter:qwen/qwen3-coder-next"),
            65_536
        );
        assert_eq!(infer_max_output_tokens("moonshot:kimi-k2.5"), 65_536);
        assert_eq!(infer_max_output_tokens("zhipu:glm-5"), 128_000);
        assert_eq!(infer_max_output_tokens("minimax:minimax-m2.7"), 131_072);
    }

    #[test]
    fn test_max_output_others() {
        assert_eq!(infer_max_output_tokens("deepseek:deepseek-chat"), 8_192);
        assert_eq!(
            infer_max_output_tokens("deepseek:deepseek-reasoner"),
            65_536
        );
        assert_eq!(infer_max_output_tokens("meta:llama-4-maverick"), 16_384);
        assert_eq!(
            infer_max_output_tokens("mistral:mistral-large-latest"),
            32_768
        );
    }

    #[test]
    fn test_max_output_unknown_fallback() {
        assert_eq!(
            infer_max_output_tokens("unknown:some-model"),
            DEFAULT_MAX_OUTPUT_TOKENS
        );
    }

    // -- OpenRouter-style paths ----------------------------------------------

    #[test]
    fn test_openrouter_model_paths() {
        // OpenRouter uses "provider/model" as model_id
        assert_eq!(
            infer_context_window("openrouter:anthropic/claude-sonnet-4-6"),
            200_000
        );
        assert_eq!(
            infer_max_output_tokens("openrouter:anthropic/claude-sonnet-4-6"),
            64_000
        );
        assert_eq!(
            infer_context_window("openrouter:google/gemini-2.5-pro"),
            1_048_576
        );
        assert_eq!(
            infer_max_output_tokens("openrouter:google/gemini-2.5-pro"),
            65_536
        );
        assert_eq!(
            infer_context_window("openrouter:deepseek/deepseek-r1"),
            164_000
        );
    }
}
