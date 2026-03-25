//! Basic chat example — send a single prompt and print the response.
//!
//! ```bash
//! # Set your API key (or use a .env file):
//! export OPENAI_API_KEY="sk-..."
//! cargo run --example basic_chat -p nexil
//! ```

use nexil::{ChatRequest, LLMBuilder};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");

    let mut llm = LLMBuilder::new()
        .model("openai:gpt-4o-mini")
        .api_key(&api_key)
        .build()
        .expect("failed to build LLM");

    let response = llm
        .chat_async(ChatRequest {
            prompt: Some("What is the capital of France? Reply in one sentence."),
            ..Default::default()
        })
        .await
        .expect("chat request failed");

    println!("{response}");
}
