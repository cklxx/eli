//! E2E test harness — provider matrix, fixtures, helpers.

mod vision;

use std::env;
use std::time::Duration;

use conduit::{ApiFormat, LLM, LLMBuilder};

// ---------------------------------------------------------------------------
// Provider matrix
// ---------------------------------------------------------------------------

pub struct TestProvider {
    pub name: &'static str,
    pub model: &'static str,
    pub key_names: &'static [&'static str],
    pub api_format: Option<ApiFormat>,
}

/// Resolve an API key by trying multiple env var names in order.
fn resolve_key(names: &[&str]) -> Option<String> {
    let _ = dotenvy::dotenv();
    for name in names {
        if let Ok(val) = env::var(name) {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// Try to build an OpenAI test provider.  Returns `None` when the key is absent.
pub fn openai_provider() -> Option<TestProvider> {
    resolve_key(&[
        "ELI_OPENAI_API_KEY",
        "OPENAI_API_KEY",
        "ELI_API_KEY",
    ])?;
    Some(TestProvider {
        name: "openai",
        model: "gpt-4o-mini",
        key_names: &["ELI_OPENAI_API_KEY", "OPENAI_API_KEY", "ELI_API_KEY"],
        // Responses transport silently drops multimodal content — force Completion.
        api_format: Some(ApiFormat::Completion),
    })
}

/// Try to build an Anthropic test provider.  Returns `None` when the key is absent.
pub fn anthropic_provider() -> Option<TestProvider> {
    resolve_key(&[
        "ELI_ANTHROPIC_API_KEY",
        "ANTHROPIC_API_KEY",
    ])?;
    Some(TestProvider {
        name: "anthropic",
        model: "claude-haiku-3-5-20241022",
        key_names: &["ELI_ANTHROPIC_API_KEY", "ANTHROPIC_API_KEY"],
        api_format: None, // Auto → Messages (correct for Anthropic)
    })
}

// ---------------------------------------------------------------------------
// LLM builder helper
// ---------------------------------------------------------------------------

pub fn build_llm(provider: &TestProvider) -> LLM {
    let key = resolve_key(provider.key_names)
        .expect("API key must be set (checked before calling build_llm)");

    let model_str = format!("{}:{}", provider.name, provider.model);
    let mut builder = LLMBuilder::new()
        .model(&model_str)
        .api_key(&key)
        .max_retries(1)
        .verbose(0);

    if let Some(fmt) = provider.api_format {
        builder = builder.api_format(fmt);
    }

    builder.build().expect("LLMBuilder::build failed")
}

// ---------------------------------------------------------------------------
// Image fixtures — 8×8 solid-color PNGs, base64-encoded
// ---------------------------------------------------------------------------

pub const RED_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEklEQVR4nGP4z8CAFWEXHbQSACj/P8Fu7N9hAAAAAElFTkSuQmCC";

pub const BLUE_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEElEQVR4nGNgYPiPAw0pCQCpcD/BFMrqcwAAAABJRU5ErkJggg==";

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

pub const RED_SYNONYMS: &[&str] = &["red", "scarlet", "crimson", "rouge", "rojo"];
pub const BLUE_SYNONYMS: &[&str] = &["blue", "azul", "bleu", "cobalt", "navy"];

/// Assert the response contains at least one of the given keywords (case-insensitive).
pub fn assert_contains_any(response: &str, keywords: &[&str], context: &str) {
    let lower = response.to_lowercase();
    let found = keywords.iter().any(|kw| lower.contains(kw));
    assert!(
        found,
        "[{context}] Expected response to contain one of {keywords:?}, got:\n{response}"
    );
}

/// Timeout wrapper for chat_async calls.
pub const CHAT_TIMEOUT: Duration = Duration::from_secs(30);

/// Build an image_base64 content block for the user message.
pub fn image_block(base64_data: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "image_base64",
        "mime_type": "image/png",
        "data": base64_data,
    })
}

/// Build a text content block for the user message.
pub fn text_block(text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "text",
        "text": text,
    })
}
