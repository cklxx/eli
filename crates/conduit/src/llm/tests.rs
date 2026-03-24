use super::*;
use crate::core::results::StreamEventKind;
use serde_json::json;

// ----- LLM::new -----

#[test]
fn test_llm_new_default_config() {
    let llm = LLM::new(
        Some("openai:gpt-4o"),
        None,
        None,
        None,
        Some("test-key".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(llm.model(), "gpt-4o");
    assert_eq!(llm.provider(), "openai");
    assert!(llm.fallback_models().is_empty());
}

#[test]
fn test_llm_new_with_provider_prefix() {
    let llm = LLM::new(
        Some("anthropic:claude-3-5-sonnet"),
        None,
        None,
        None,
        Some("key".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(llm.model(), "claude-3-5-sonnet");
    assert_eq!(llm.provider(), "anthropic");
}

#[test]
fn test_llm_new_with_explicit_provider() {
    let llm = LLM::new(
        Some("gpt-4o"),
        Some("openai"),
        None,
        None,
        Some("key".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(llm.model(), "gpt-4o");
    assert_eq!(llm.provider(), "openai");
}

#[test]
fn test_llm_new_defaults_to_gpt4o_mini() {
    let llm = LLM::new(
        None,
        None,
        None,
        None,
        Some("key".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    // Default model is "openai:gpt-4o-mini"
    assert_eq!(llm.provider(), "openai");
    assert_eq!(llm.model(), "gpt-4o-mini");
}

#[test]
fn test_llm_new_rejects_invalid_verbose() {
    let result = LLM::new(
        Some("openai:gpt-4o"),
        None,
        None,
        None,
        Some("key".to_string()),
        None,
        None,
        None,
        None,
        Some(5),
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("verbose"));
}

#[test]
fn test_llm_new_rejects_provider_prefix_with_explicit_provider() {
    let result = LLM::new(
        Some("openai:gpt-4o"),
        Some("anthropic"),
        None,
        None,
        Some("key".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
}

#[test]
fn test_llm_new_with_fallback_models() {
    let llm = LLM::new(
        Some("openai:gpt-4o"),
        None,
        Some(vec!["openai:gpt-4o-mini".to_string()]),
        None,
        Some("key".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(llm.fallback_models(), &["openai:gpt-4o-mini"]);
}

// ----- Display / Debug -----

#[test]
fn test_llm_display() {
    let llm = LLM::new(
        Some("openai:gpt-4o"),
        None,
        None,
        None,
        Some("key".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let display = format!("{}", llm);
    assert_eq!(display, "LLM(openai:gpt-4o)");
}

#[test]
fn test_llm_debug() {
    let llm = LLM::new(
        Some("openai:gpt-4o"),
        None,
        None,
        None,
        Some("key".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let debug = format!("{:?}", llm);
    assert!(debug.contains("LLM"));
    assert!(debug.contains("openai"));
}

// ----- ApiFormat -----

#[test]
fn test_api_format_as_str() {
    assert_eq!(ApiFormat::Auto.as_str(), "auto");
    assert_eq!(ApiFormat::Completion.as_str(), "completion");
    assert_eq!(ApiFormat::Responses.as_str(), "responses");
    assert_eq!(ApiFormat::Messages.as_str(), "messages");
}

#[test]
fn test_api_format_equality() {
    assert_eq!(ApiFormat::Auto, ApiFormat::Auto);
    assert_eq!(ApiFormat::Completion, ApiFormat::Completion);
    assert_ne!(ApiFormat::Completion, ApiFormat::Responses);
}

// ----- EmbedInput -----

#[test]
fn test_embed_input_from_str() {
    let input: EmbedInput = "hello".into();
    matches!(input, EmbedInput::Single("hello"));
}

#[test]
fn test_embed_input_from_slice() {
    let data = vec!["a".to_string(), "b".to_string()];
    let input: EmbedInput = data.as_slice().into();
    matches!(input, EmbedInput::Multiple(_));
}

// ----- build_messages -----

#[test]
fn test_build_messages_with_prompt_only() {
    let msgs = build_messages(Some("hello"), None, None);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"], "hello");
}

#[test]
fn test_build_messages_with_system_and_prompt() {
    let msgs = build_messages(Some("hello"), Some("you are helpful"), None);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[0]["content"], "you are helpful");
    assert_eq!(msgs[1]["role"], "user");
}

#[test]
fn test_build_messages_with_existing_messages() {
    let existing = vec![json!({"role": "assistant", "content": "hi"})];
    let msgs = build_messages(Some("follow up"), None, Some(&existing));
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "assistant");
    assert_eq!(msgs[1]["role"], "user");
}

#[test]
fn test_build_messages_empty() {
    let msgs = build_messages(None, None, None);
    assert!(msgs.is_empty());
}

#[test]
fn test_build_full_context_from_entries_normalizes_responses_tool_calls() {
    let entries = vec![
        TapeEntry::tool_call(
            vec![json!({
                "type": "function_call",
                "call_id": "call_123",
                "name": "tape_info",
                "arguments": "{}"
            })],
            json!({}),
        ),
        TapeEntry::tool_result(
            vec![json!({
                "call_id": "call_123",
                "output": {"count": 1}
            })],
            json!({}),
        ),
    ];

    let messages = build_full_context_from_entries(&entries);

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["tool_calls"][0]["id"], "call_123");
    assert_eq!(
        messages[0]["tool_calls"][0]["function"]["name"],
        "tape_info"
    );
    assert_eq!(messages[1]["tool_call_id"], "call_123");
}

#[test]
fn test_build_full_context_from_entries_preserves_tool_call_content() {
    let entries = vec![TapeEntry::tool_call_with_content(
        vec![json!({
            "type": "function",
            "id": "call_123",
            "function": {
                "name": "tape_info",
                "arguments": "{}"
            }
        })],
        Some("Checking tape state".to_owned()),
        json!({}),
    )];

    let messages = build_full_context_from_entries(&entries);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "Checking tape state");
    assert_eq!(messages[0]["tool_calls"][0]["id"], "call_123");
}

#[tokio::test]
async fn test_prepare_messages_with_tape_persists_initial_prompt_and_system_prompt() {
    let llm = LLM::builder()
        .tape_store(AsyncTapeStoreAdapter::new(InMemoryTapeStore::new()))
        .build()
        .unwrap();
    let initial_round_msgs = build_messages(Some("hello"), Some("system"), None);
    llm.persist_initial_messages("test-tape", &initial_round_msgs)
        .await
        .unwrap();

    let messages = llm
        ._prepare_messages(Some("test-tape"), None, &initial_round_msgs)
        .await
        .unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "system");
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"], "hello");

    let entries = llm
        .async_tape
        .fetch_entries(&llm.async_tape.query_tape("test-tape"))
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].kind, "system");
    assert_eq!(entries[1].kind, "message");
}

#[tokio::test]
async fn test_persist_initial_messages_skips_duplicate_system_prompt() {
    let llm = LLM::builder()
        .tape_store(AsyncTapeStoreAdapter::new(InMemoryTapeStore::new()))
        .build()
        .unwrap();
    let tape = "test-dedup";

    // First call writes system + user.
    let msgs1 = build_messages(Some("hello"), Some("system prompt"), None);
    llm.persist_initial_messages(tape, &msgs1).await.unwrap();

    // Second call with same system prompt should NOT duplicate it.
    let msgs2 = build_messages(Some("world"), Some("system prompt"), None);
    llm.persist_initial_messages(tape, &msgs2).await.unwrap();

    let entries = llm
        .async_tape
        .fetch_entries(&llm.async_tape.query_tape(tape))
        .await
        .unwrap();

    let system_count = entries.iter().filter(|e| e.kind == "system").count();
    assert_eq!(system_count, 1, "system prompt should only appear once");

    let message_count = entries.iter().filter(|e| e.kind == "message").count();
    assert_eq!(message_count, 2, "both user messages should be persisted");
}

#[tokio::test]
async fn test_persist_initial_messages_writes_changed_system_prompt() {
    let llm = LLM::builder()
        .tape_store(AsyncTapeStoreAdapter::new(InMemoryTapeStore::new()))
        .build()
        .unwrap();
    let tape = "test-changed";

    let msgs1 = build_messages(Some("hello"), Some("prompt v1"), None);
    llm.persist_initial_messages(tape, &msgs1).await.unwrap();

    // Different system prompt should be written.
    let msgs2 = build_messages(Some("world"), Some("prompt v2"), None);
    llm.persist_initial_messages(tape, &msgs2).await.unwrap();

    let entries = llm
        .async_tape
        .fetch_entries(&llm.async_tape.query_tape(tape))
        .await
        .unwrap();

    let system_count = entries.iter().filter(|e| e.kind == "system").count();
    assert_eq!(system_count, 2, "changed system prompt should be written");
}

#[tokio::test]
async fn test_persist_initial_messages_no_system_in_msgs() {
    let llm = LLM::builder()
        .tape_store(AsyncTapeStoreAdapter::new(InMemoryTapeStore::new()))
        .build()
        .unwrap();
    let tape = "test-no-system";

    // Messages with no system prompt — only user message.
    let msgs = build_messages(Some("hello"), None, None);
    llm.persist_initial_messages(tape, &msgs).await.unwrap();

    let entries = llm
        .async_tape
        .fetch_entries(&llm.async_tape.query_tape(tape))
        .await
        .unwrap();

    assert_eq!(entries.iter().filter(|e| e.kind == "system").count(), 0);
    assert_eq!(entries.iter().filter(|e| e.kind == "message").count(), 1);
}

#[tokio::test]
async fn test_persist_initial_messages_three_calls_same_prompt() {
    let llm = LLM::builder()
        .tape_store(AsyncTapeStoreAdapter::new(InMemoryTapeStore::new()))
        .build()
        .unwrap();
    let tape = "test-triple";

    for i in 0..3 {
        let msgs = build_messages(Some(&format!("msg {i}")), Some("stable system"), None);
        llm.persist_initial_messages(tape, &msgs).await.unwrap();
    }

    let entries = llm
        .async_tape
        .fetch_entries(&llm.async_tape.query_tape(tape))
        .await
        .unwrap();

    assert_eq!(
        entries.iter().filter(|e| e.kind == "system").count(),
        1,
        "system prompt should appear exactly once across 3 calls"
    );
    assert_eq!(
        entries.iter().filter(|e| e.kind == "message").count(),
        3,
        "all 3 user messages should be persisted"
    );
}

#[tokio::test]
async fn test_persist_initial_messages_change_then_revert() {
    let llm = LLM::builder()
        .tape_store(AsyncTapeStoreAdapter::new(InMemoryTapeStore::new()))
        .build()
        .unwrap();
    let tape = "test-revert";

    let msgs_v1 = build_messages(Some("a"), Some("v1"), None);
    llm.persist_initial_messages(tape, &msgs_v1).await.unwrap();

    let msgs_v2 = build_messages(Some("b"), Some("v2"), None);
    llm.persist_initial_messages(tape, &msgs_v2).await.unwrap();

    // Revert back to v1 — should write again since latest is v2.
    let msgs_v1_again = build_messages(Some("c"), Some("v1"), None);
    llm.persist_initial_messages(tape, &msgs_v1_again)
        .await
        .unwrap();

    let entries = llm
        .async_tape
        .fetch_entries(&llm.async_tape.query_tape(tape))
        .await
        .unwrap();

    assert_eq!(
        entries.iter().filter(|e| e.kind == "system").count(),
        3,
        "v1 -> v2 -> v1 should produce 3 system entries"
    );
}

#[tokio::test]
async fn test_persist_initial_messages_empty_list() {
    let llm = LLM::builder()
        .tape_store(AsyncTapeStoreAdapter::new(InMemoryTapeStore::new()))
        .build()
        .unwrap();
    let tape = "test-empty";

    llm.persist_initial_messages(tape, &[]).await.unwrap();

    let entries = llm
        .async_tape
        .fetch_entries(&llm.async_tape.query_tape(tape))
        .await
        .unwrap();
    assert!(entries.is_empty());
}

#[tokio::test]
async fn test_build_full_context_dedup_with_legacy_duplicate_system_entries() {
    // Even if the tape contains duplicate system entries (e.g. from before
    // the dedup fix), build_full_context_from_entries keeps only the last one.
    let meta = json!({});
    let entries = vec![
        TapeEntry::system("prompt v1", meta.clone()),
        TapeEntry::message(json!({"role": "user", "content": "hello"}), meta.clone()),
        TapeEntry::system("prompt v1", meta.clone()), // legacy duplicate
        TapeEntry::message(json!({"role": "user", "content": "world"}), meta),
    ];
    let messages = build_full_context_from_entries(&entries);

    let system_msgs: Vec<_> = messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
        .collect();
    assert_eq!(
        system_msgs.len(),
        1,
        "build_full_context should keep only the last system"
    );
    assert_eq!(system_msgs[0]["content"], "prompt v1");

    let user_msgs: Vec<_> = messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .collect();
    assert_eq!(user_msgs.len(), 2, "all user messages should be preserved");
}

#[tokio::test]
async fn test_prepare_messages_no_tape_returns_in_memory() {
    let llm = LLM::builder()
        .tape_store(AsyncTapeStoreAdapter::new(InMemoryTapeStore::new()))
        .build()
        .unwrap();

    let in_memory = vec![json!({"role": "user", "content": "hello"})];
    let msgs = llm._prepare_messages(None, None, &in_memory).await.unwrap();

    assert_eq!(msgs, in_memory);
}

// ----- extract_content -----

#[test]
fn test_extract_content_completion_format() {
    let response = json!({
        "choices": [{
            "message": {
                "content": "Hello there!"
            }
        }]
    });
    assert_eq!(extract_content(&response).unwrap(), "Hello there!");
}

#[test]
fn test_extract_content_responses_format() {
    let response = json!({
        "output": [{
            "type": "message",
            "content": [{"text": "Response text"}]
        }]
    });
    assert_eq!(extract_content(&response).unwrap(), "Response text");
}

#[test]
fn test_extract_content_missing() {
    let response = json!({});
    assert!(extract_content(&response).is_err());
}

#[test]
fn test_extract_content_anthropic_empty_content_errors() {
    let response = json!({
        "role": "assistant",
        "content": []
    });
    assert!(extract_content(&response).is_err());
}

// ----- extract_tool_calls -----

#[test]
fn test_extract_tool_calls_completion_format() {
    let response = json!({
        "choices": [{
            "message": {
                "tool_calls": [
                    {"type": "function", "function": {"name": "tool1", "arguments": "{}"}}
                ]
            }
        }]
    });
    let calls = extract_tool_calls(&response).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["function"]["name"], "tool1");
}

#[test]
fn test_extract_tool_calls_responses_format() {
    let response = json!({
        "output": [
            {"type": "function_call", "name": "tool1", "arguments": "{}"},
            {"type": "message", "content": [{"text": "hello"}]},
        ]
    });
    let calls = extract_tool_calls(&response).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["function"]["name"], "tool1");
}

#[test]
fn test_extract_tool_calls_empty() {
    let response = json!({"choices": [{"message": {"content": "no tools"}}]});
    let calls = extract_tool_calls(&response).unwrap();
    assert!(calls.is_empty());
}

// ----- default_api_base -----

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

// ----- LLMBuilder -----

#[test]
fn test_builder_basic() {
    let llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key("test-key")
        .build()
        .unwrap();

    assert_eq!(llm.model(), "gpt-4o");
    assert_eq!(llm.provider(), "openai");
    assert!(llm.fallback_models().is_empty());
    assert!(llm.stream_filter().is_none());
}

#[test]
fn test_builder_with_provider() {
    let llm = LLM::builder()
        .model("gpt-4o")
        .provider("openai")
        .api_key("test-key")
        .build()
        .unwrap();

    assert_eq!(llm.model(), "gpt-4o");
    assert_eq!(llm.provider(), "openai");
}

#[test]
fn test_builder_with_fallback_models() {
    let llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key("test-key")
        .fallback_models(vec!["openai:gpt-4o-mini".to_string()])
        .build()
        .unwrap();

    assert_eq!(llm.fallback_models(), &["openai:gpt-4o-mini"]);
}

#[test]
fn test_builder_with_api_format() {
    let llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key("test-key")
        .api_format(ApiFormat::Responses)
        .build()
        .unwrap();

    assert_eq!(llm.provider(), "openai");
}

#[test]
fn test_builder_with_verbose() {
    let llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key("test-key")
        .verbose(2)
        .build()
        .unwrap();

    assert_eq!(llm.model(), "gpt-4o");
}

#[test]
fn test_builder_rejects_invalid_verbose() {
    let result = LLM::builder()
        .model("openai:gpt-4o")
        .api_key("test-key")
        .verbose(5)
        .build();

    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("verbose"));
}

#[test]
fn test_builder_defaults_to_gpt4o_mini() {
    // Default model is "openai:gpt-4o-mini" which includes provider prefix
    let llm = LLM::builder().api_key("test-key").build().unwrap();

    assert_eq!(llm.provider(), "openai");
    assert_eq!(llm.model(), "gpt-4o-mini");
}

#[test]
fn test_builder_with_api_key_resolver() {
    let resolver: APIKeyResolver = Box::new(|provider: &str| {
        if provider == "openai" {
            Some("resolved-key".to_string())
        } else {
            None
        }
    });

    let llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key_resolver(resolver)
        .build()
        .unwrap();

    assert_eq!(llm.model(), "gpt-4o");
    assert_eq!(llm.provider(), "openai");
}

#[test]
fn test_builder_api_key_resolver_fallback_to_map() {
    // Resolver returns None for "openai", should fall back to map
    let resolver: APIKeyResolver = Box::new(|_provider: &str| None);

    let mut map = HashMap::new();
    map.insert("openai".to_string(), "map-key".to_string());

    let llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key_resolver(resolver)
        .api_key_map(map)
        .build()
        .unwrap();

    assert_eq!(llm.model(), "gpt-4o");
}

#[test]
fn test_builder_explicit_key_overrides_resolver() {
    let resolver: APIKeyResolver = Box::new(|_provider: &str| Some("resolver-key".to_string()));

    let llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key("explicit-key")
        .api_key_resolver(resolver)
        .build()
        .unwrap();

    // Explicit key takes priority
    assert_eq!(llm.model(), "gpt-4o");
}

#[test]
fn test_builder_with_stream_filter() {
    let filter: StreamEventFilter = Arc::new(|event| Some(event));

    let llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key("test-key")
        .stream_filter(filter)
        .build()
        .unwrap();

    assert!(llm.stream_filter().is_some());
}

#[test]
fn test_builder_with_max_retries() {
    let llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key("test-key")
        .max_retries(5)
        .build()
        .unwrap();

    assert_eq!(llm.model(), "gpt-4o");
}

#[test]
fn test_builder_with_api_base() {
    let llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key("test-key")
        .api_base("https://custom.api.com/v1")
        .build()
        .unwrap();

    assert_eq!(llm.model(), "gpt-4o");
}

#[test]
fn test_builder_default() {
    // LLMBuilder::default() should be equivalent to LLMBuilder::new()
    let _builder = LLMBuilder::default();
}

// ----- StreamEventFilter -----

#[test]
fn test_stream_filter_drops_events() {
    // Filter that drops all Text events
    let filter: StreamEventFilter = Arc::new(|event| {
        if event.kind == StreamEventKind::Text {
            None
        } else {
            Some(event)
        }
    });

    let text_event = StreamEvent::new(StreamEventKind::Text, json!({"delta": "hello"}));
    let usage_event = StreamEvent::new(StreamEventKind::Usage, json!({"tokens": 42}));

    assert!(filter(text_event).is_none());
    assert!(filter(usage_event).is_some());
}

#[test]
fn test_stream_filter_transforms_events() {
    // Filter that uppercases text deltas
    let filter: StreamEventFilter = Arc::new(|mut event| {
        if event.kind == StreamEventKind::Text {
            if let Some(delta) = event.data.get("delta").and_then(|d| d.as_str()) {
                event.data = json!({"delta": delta.to_uppercase()});
            }
        }
        Some(event)
    });

    let event = StreamEvent::new(StreamEventKind::Text, json!({"delta": "hello"}));
    let result = filter(event).unwrap();
    assert_eq!(result.data["delta"], "HELLO");
}

#[test]
fn test_stream_filter_passthrough() {
    let filter: StreamEventFilter = Arc::new(|event| Some(event));

    let event = StreamEvent::new(StreamEventKind::Final, json!({"ok": true}));
    let result = filter(event);
    assert!(result.is_some());
    let result = result.unwrap();
    assert_eq!(result.kind, StreamEventKind::Final);
}

#[test]
fn test_with_stream_filter_set_and_clear() {
    let mut llm = LLM::builder()
        .model("openai:gpt-4o")
        .api_key("test-key")
        .build()
        .unwrap();

    assert!(llm.stream_filter().is_none());

    let filter: StreamEventFilter = Arc::new(|event| Some(event));
    llm.with_stream_filter(filter);
    assert!(llm.stream_filter().is_some());

    llm.clear_stream_filter();
    assert!(llm.stream_filter().is_none());
}

// ----- LLM::responses URL/body building -----

#[test]
fn test_responses_url_default_provider() {
    // Verify the URL is built correctly from the default provider base
    let base = default_api_base("openai");
    let url = format!("{}/responses", base.trim_end_matches('/'));
    assert_eq!(url, "https://api.openai.com/v1/responses");
}

#[test]
fn test_responses_url_custom_base() {
    let base = "https://custom.api.com/v2/";
    let url = format!("{}/responses", base.trim_end_matches('/'));
    assert_eq!(url, "https://custom.api.com/v2/responses");
}

#[test]
fn test_responses_url_anthropic() {
    let base = default_api_base("anthropic");
    let url = format!("{}/responses", base.trim_end_matches('/'));
    assert_eq!(url, "https://api.anthropic.com/v1/responses");
}

#[test]
fn test_responses_body_structure() {
    // Verify the body JSON structure matches what responses() would build
    let input = json!("Tell me a joke");
    let model = "gpt-4o";
    let body = json!({
        "model": model,
        "input": input,
    });
    assert_eq!(body["model"], "gpt-4o");
    assert_eq!(body["input"], "Tell me a joke");
}

#[test]
fn test_responses_body_with_array_input() {
    let input = json!([
        {"role": "user", "content": "Hello"},
        {"role": "assistant", "content": "Hi there"},
    ]);
    let body = json!({
        "model": "gpt-4o",
        "input": input,
    });
    assert!(body["input"].is_array());
    assert_eq!(body["input"].as_array().unwrap().len(), 2);
}

// ----- Decision functions -----

#[test]
fn test_collect_active_decisions_basic() {
    let meta = json!({});
    let entries = vec![
        TapeEntry::decision("Use PostgreSQL", meta.clone()),
        TapeEntry::decision("API-first design", meta.clone()),
    ];
    let decisions = collect_active_decisions(&entries);
    assert_eq!(decisions, vec!["Use PostgreSQL", "API-first design"]);
}

#[test]
fn test_collect_active_decisions_with_revocation() {
    let meta = json!({});
    let entries = vec![
        TapeEntry::decision("Use PostgreSQL", meta.clone()),
        TapeEntry::decision("API-first design", meta.clone()),
        TapeEntry::decision_revoked("Use PostgreSQL", meta.clone()),
    ];
    let decisions = collect_active_decisions(&entries);
    assert_eq!(decisions, vec!["API-first design"]);
}

#[test]
fn test_collect_active_decisions_empty() {
    let decisions = collect_active_decisions(&[]);
    assert!(decisions.is_empty());
}

#[test]
fn test_collect_active_decisions_skips_empty_text() {
    let meta = json!({});
    let entries = vec![
        TapeEntry::decision("", meta.clone()),
        TapeEntry::decision("Real decision", meta.clone()),
    ];
    let decisions = collect_active_decisions(&entries);
    assert_eq!(decisions, vec!["Real decision"]);
}

#[test]
fn test_collect_active_decisions_duplicate_text() {
    let meta = json!({});
    let entries = vec![
        TapeEntry::decision("Use PostgreSQL", meta.clone()),
        TapeEntry::decision("Use PostgreSQL", meta.clone()),
    ];
    let decisions = collect_active_decisions(&entries);
    // Both kept — dedup is caller's concern
    assert_eq!(decisions.len(), 2);
}

#[test]
fn test_inject_decisions_into_system_prompt() {
    let mut messages = vec![
        json!({"role": "system", "content": "You are helpful."}),
        json!({"role": "user", "content": "Hello"}),
    ];
    inject_decisions_into_system_prompt(
        &mut messages,
        &["Use PostgreSQL".to_string(), "API-first".to_string()],
    );
    let system_content = messages[0]["content"].as_str().unwrap();
    assert!(system_content.contains("Active decisions:"));
    assert!(system_content.contains("1. Use PostgreSQL"));
    assert!(system_content.contains("2. API-first"));
    assert!(system_content.starts_with("You are helpful."));
}

#[test]
fn test_inject_decisions_no_system_message() {
    let mut messages = vec![json!({"role": "user", "content": "Hello"})];
    inject_decisions_into_system_prompt(&mut messages, &["Use PostgreSQL".to_string()]);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert!(
        messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("1. Use PostgreSQL")
    );
}

#[test]
fn test_inject_decisions_empty_list() {
    let mut messages = vec![json!({"role": "system", "content": "Original"})];
    inject_decisions_into_system_prompt(&mut messages, &[]);
    assert_eq!(messages[0]["content"], "Original");
}

#[test]
fn test_decisions_survive_full_context_build() {
    // Decisions should NOT appear in build_full_context_from_entries output
    // (they are injected separately via full-tape scan)
    let meta = json!({});
    let entries = vec![
        TapeEntry::decision("Use PostgreSQL", meta.clone()),
        TapeEntry::message(json!({"role": "user", "content": "Hello"}), meta.clone()),
    ];
    let messages = build_full_context_from_entries(&entries);
    // Decision entries are skipped by build_full_context (kind is not message/system/tool_*)
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "Hello");
}
