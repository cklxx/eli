//! Streaming example — print text chunks as they arrive from the model.
//!
//! ```bash
//! export OPENAI_API_KEY="sk-..."
//! cargo run --example streaming -p nexil
//! ```

use futures::StreamExt;
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

    let text_stream = llm
        .stream(ChatRequest {
            prompt: Some("Count from 1 to 10, one number per line."),
            ..Default::default()
        })
        .await
        .expect("stream request failed");

    // AsyncTextStream implements Stream<Item = String> via into_stream().
    let mut stream = text_stream.into_stream();
    while let Some(chunk) = stream.next().await {
        print!("{chunk}");
    }
    println!();
}
