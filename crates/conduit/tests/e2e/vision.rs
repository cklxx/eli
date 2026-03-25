//! E2E vision tests — multimodal image understanding across providers.

use conduit::ChatRequest;
use tokio::time::timeout;

use super::{
    BLUE_PNG_BASE64, BLUE_SYNONYMS, CHAT_TIMEOUT, RED_PNG_BASE64, RED_SYNONYMS,
    TestProvider, anthropic_provider, assert_contains_any, build_llm, image_block, openai_provider,
    text_block,
};

// ---------------------------------------------------------------------------
// Macro: generate one #[tokio::test] per (provider, test_case)
// ---------------------------------------------------------------------------

macro_rules! vision_test {
    ($test_name:ident, $provider_fn:ident, $impl_fn:ident) => {
        #[tokio::test]
        #[ignore]
        async fn $test_name() {
            let Some(provider) = $provider_fn() else {
                eprintln!(
                    "SKIP {}: no API key ({})",
                    stringify!($test_name),
                    stringify!($provider_fn)
                );
                return;
            };
            eprintln!(
                "RUN  {}: {} ({})",
                stringify!($test_name),
                provider.name,
                provider.model
            );
            $impl_fn(&provider).await;
        }
    };
}

// ===== text_only_sanity =====

async fn text_only_sanity_impl(provider: &TestProvider) {
    let mut llm = build_llm(provider);
    let result = timeout(
        CHAT_TIMEOUT,
        llm.chat_async(ChatRequest {
            prompt: Some("Reply with exactly: hello"),
            max_tokens: Some(100),
            ..Default::default()
        }),
    )
    .await
    .expect("timeout")
    .expect("chat_async failed");

    assert!(
        !result.is_empty(),
        "[{} text_only_sanity] Empty response",
        provider.name
    );
    eprintln!(
        "[{} text_only_sanity] OK: {}",
        provider.name,
        &result[..result.len().min(80)]
    );
}

vision_test!(text_only_sanity_openai, openai_provider, text_only_sanity_impl);
vision_test!(text_only_sanity_anthropic, anthropic_provider, text_only_sanity_impl);

// ===== single_image_describe =====

async fn single_image_describe_impl(provider: &TestProvider) {
    let mut llm = build_llm(provider);
    let result = timeout(
        CHAT_TIMEOUT,
        llm.chat_async(ChatRequest {
            user_content: Some(vec![
                image_block(RED_PNG_BASE64),
                text_block("What color is this image? Answer in one word."),
            ]),
            max_tokens: Some(100),
            ..Default::default()
        }),
    )
    .await
    .expect("timeout")
    .expect("chat_async failed");

    assert_contains_any(
        &result,
        RED_SYNONYMS,
        &format!("{} single_image_describe", provider.name),
    );
    eprintln!(
        "[{} single_image_describe] OK: {}",
        provider.name,
        &result[..result.len().min(80)]
    );
}

vision_test!(single_image_describe_openai, openai_provider, single_image_describe_impl);
vision_test!(single_image_describe_anthropic, anthropic_provider, single_image_describe_impl);

// ===== multi_image_compare =====

async fn multi_image_compare_impl(provider: &TestProvider) {
    let mut llm = build_llm(provider);
    let result = timeout(
        CHAT_TIMEOUT,
        llm.chat_async(ChatRequest {
            user_content: Some(vec![
                image_block(RED_PNG_BASE64),
                image_block(BLUE_PNG_BASE64),
                text_block("What two colors are shown in these images? Answer briefly."),
            ]),
            max_tokens: Some(100),
            ..Default::default()
        }),
    )
    .await
    .expect("timeout")
    .expect("chat_async failed");

    assert_contains_any(
        &result,
        RED_SYNONYMS,
        &format!("{} multi_image red", provider.name),
    );
    assert_contains_any(
        &result,
        BLUE_SYNONYMS,
        &format!("{} multi_image blue", provider.name),
    );
    eprintln!(
        "[{} multi_image_compare] OK: {}",
        provider.name,
        &result[..result.len().min(80)]
    );
}

vision_test!(multi_image_compare_openai, openai_provider, multi_image_compare_impl);
vision_test!(multi_image_compare_anthropic, anthropic_provider, multi_image_compare_impl);

// ===== image_with_system_prompt =====

async fn image_with_system_prompt_impl(provider: &TestProvider) {
    let mut llm = build_llm(provider);
    let result = timeout(
        CHAT_TIMEOUT,
        llm.chat_async(ChatRequest {
            system_prompt: Some(
                "You are a color detection assistant. Always respond in the format: COLOR=<name>",
            ),
            user_content: Some(vec![
                image_block(RED_PNG_BASE64),
                text_block("Detect the color."),
            ]),
            max_tokens: Some(100),
            ..Default::default()
        }),
    )
    .await
    .expect("timeout")
    .expect("chat_async failed");

    assert!(
        result.to_lowercase().contains("color=") || result.to_lowercase().contains("red"),
        "[{} image_with_system_prompt] Expected 'COLOR=' format or 'red', got:\n{}",
        provider.name,
        result
    );
    eprintln!(
        "[{} image_with_system_prompt] OK: {}",
        provider.name,
        &result[..result.len().min(80)]
    );
}

vision_test!(image_with_system_prompt_openai, openai_provider, image_with_system_prompt_impl);
vision_test!(image_with_system_prompt_anthropic, anthropic_provider, image_with_system_prompt_impl);

// ===== image_only_no_text =====

async fn image_only_no_text_impl(provider: &TestProvider) {
    let mut llm = build_llm(provider);
    // Send a BLUE image with no text — if the model hallucinates "red" from prior
    // context, it means the image wasn't actually processed.
    let result = timeout(
        CHAT_TIMEOUT,
        llm.chat_async(ChatRequest {
            user_content: Some(vec![image_block(BLUE_PNG_BASE64)]),
            max_tokens: Some(100),
            ..Default::default()
        }),
    )
    .await
    .expect("timeout")
    .expect("chat_async failed");

    assert!(
        !result.is_empty(),
        "[{} image_only_no_text] Empty response",
        provider.name
    );
    // The model should mention blue (the actual image color), not red.
    let lower = result.to_lowercase();
    let mentions_blue = BLUE_SYNONYMS.iter().any(|kw| lower.contains(kw));
    let mentions_red = RED_SYNONYMS.iter().any(|kw| lower.contains(kw));
    assert!(
        mentions_blue || !mentions_red,
        "[{} image_only_no_text] Model appears to hallucinate — says red but image is blue:\n{}",
        provider.name,
        result
    );
    eprintln!(
        "[{} image_only_no_text] OK: {}",
        provider.name,
        &result[..result.len().min(80)]
    );
}

vision_test!(image_only_no_text_openai, openai_provider, image_only_no_text_impl);
vision_test!(image_only_no_text_anthropic, anthropic_provider, image_only_no_text_impl);
