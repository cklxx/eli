//! Provider policy decisions shared across request paths.

use serde::{Deserialize, Serialize};

/// Provider-specific behavioural toggles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPolicy {
    pub enable_responses_without_capability: bool,
    pub include_usage_in_completion_stream: bool,
    pub completion_max_tokens_arg: String,
    pub responses_tools_blocked_model_prefixes: Vec<String>,
}

impl Default for ProviderPolicy {
    fn default() -> Self {
        Self {
            enable_responses_without_capability: false,
            include_usage_in_completion_stream: false,
            completion_max_tokens_arg: "max_tokens".to_owned(),
            responses_tools_blocked_model_prefixes: Vec::new(),
        }
    }
}

fn provider_name_key(provider_name: &str) -> String {
    provider_name.trim().to_lowercase()
}

fn provider_alias(provider_name: &str) -> String {
    match provider_name_key(provider_name).as_str() {
        "claude" | "anthropic" => "anthropic".to_owned(),
        "copilot" | "github-copilot" => "github-copilot".to_owned(),
        other => other.to_owned(),
    }
}

/// Normalize provider aliases while preserving unknown provider names.
pub fn normalized_provider_name(provider_name: &str) -> String {
    provider_alias(provider_name)
}

/// Default API base URL for a provider.
pub fn default_api_base(provider_name: &str) -> String {
    match provider_alias(provider_name).as_str() {
        "anthropic" => "https://api.anthropic.com/v1".to_owned(),
        "openai" => "https://api.openai.com/v1".to_owned(),
        "openrouter" => "https://openrouter.ai/api/v1".to_owned(),
        "github-copilot" => "https://api.githubcopilot.com".to_owned(),
        other => format!("https://api.{other}.com/v1"),
    }
}

/// Default model for a given provider.
pub fn default_model_for_provider(provider_name: &str) -> &'static str {
    match provider_alias(provider_name).as_str() {
        "openai" => "openai:gpt-5.4-mini",
        "anthropic" => "anthropic:claude-sonnet-4-6",
        "github-copilot" => "github-copilot:gpt-5.4-mini",
        _ => "openrouter:openai/gpt-5.4-mini",
    }
}

/// Look up the policy for a given provider name.
///
/// Unknown providers fall back to the default policy.
pub fn provider_policy(provider_name: &str) -> ProviderPolicy {
    let normalized = provider_name_key(provider_name);
    match normalized.as_str() {
        "github-copilot" => ProviderPolicy {
            include_usage_in_completion_stream: true,
            completion_max_tokens_arg: "max_tokens".to_owned(),
            ..Default::default()
        },
        "openai" => ProviderPolicy {
            enable_responses_without_capability: true,
            include_usage_in_completion_stream: true,
            completion_max_tokens_arg: "max_completion_tokens".to_owned(),
            ..Default::default()
        },
        "openrouter" => ProviderPolicy {
            enable_responses_without_capability: true,
            include_usage_in_completion_stream: true,
            responses_tools_blocked_model_prefixes: vec!["anthropic/".to_owned()],
            ..Default::default()
        },
        _ => ProviderPolicy::default(),
    }
}

fn responses_tools_blocked_for_model(provider_name: &str, model_id: &str) -> bool {
    let policy = provider_policy(provider_name);
    let lowered_model = model_id.trim().to_lowercase();
    policy
        .responses_tools_blocked_model_prefixes
        .iter()
        .any(|prefix| lowered_model.starts_with(prefix))
}

/// Return the reason why the responses format should be rejected, or `None`
/// if it is acceptable.
pub fn responses_rejection_reason(
    provider_name: &str,
    model_id: &str,
    has_tools: bool,
    supports_responses: bool,
) -> Option<String> {
    if has_tools && responses_tools_blocked_for_model(provider_name, model_id) {
        return Some(
            "responses format is not supported for this model when tools are enabled".to_owned(),
        );
    }
    if supports_responses {
        return None;
    }
    if provider_policy(provider_name).enable_responses_without_capability {
        return None;
    }
    Some("responses format is not supported by this provider".to_owned())
}

/// Whether the provider+model combination supports the messages format.
pub fn supports_messages_format(provider_name: &str, model_id: &str) -> bool {
    let normalized_provider = normalized_provider_name(provider_name);
    let normalized_model = model_id.trim().to_lowercase();
    normalized_provider == "anthropic" || normalized_model.starts_with("anthropic/")
}

/// Whether usage data should be included in the completion stream.
pub fn should_include_completion_stream_usage(provider_name: &str) -> bool {
    provider_policy(provider_name).include_usage_in_completion_stream
}

/// The argument name used for max tokens in completion requests.
pub fn completion_max_tokens_arg(provider_name: &str) -> String {
    provider_policy(provider_name).completion_max_tokens_arg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalized_provider_name() {
        assert_eq!(normalized_provider_name("claude"), "anthropic");
        assert_eq!(normalized_provider_name("copilot"), "github-copilot");
        assert_eq!(normalized_provider_name("openai"), "openai");
        assert_eq!(normalized_provider_name(" Claude "), "anthropic");
    }

    #[test]
    fn test_default_api_base() {
        assert_eq!(default_api_base("claude"), "https://api.anthropic.com/v1");
        assert_eq!(default_api_base("copilot"), "https://api.githubcopilot.com");
        assert_eq!(default_api_base("cohere"), "https://api.cohere.com/v1");
    }

    #[test]
    fn test_default_model_for_provider() {
        assert_eq!(
            default_model_for_provider("claude"),
            "anthropic:claude-sonnet-4-6"
        );
        assert_eq!(
            default_model_for_provider("copilot"),
            "github-copilot:gpt-5.4-mini"
        );
        assert_eq!(
            default_model_for_provider("cohere"),
            "openrouter:openai/gpt-5.4-mini"
        );
    }
}
