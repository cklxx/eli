//! Streaming chat completion.

use serde_json::Value;
use uuid::Uuid;

use crate::clients::parsing::parser_for_transport;
use crate::core::errors::ConduitError;
use crate::core::results::AsyncTextStream;

use super::{LLM, build_messages, prepend_tape_history};

impl LLM {
    /// Stream chat completion as an async `TextStream`.
    pub async fn stream(
        &mut self,
        req: super::ChatRequest<'_>,
    ) -> Result<AsyncTextStream, ConduitError> {
        let super::ChatRequest {
            prompt,
            user_content,
            system_prompt,
            model,
            provider,
            messages,
            max_tokens,
            tape,
            ..
        } = req;
        use futures::StreamExt;

        let tape_messages = match tape {
            Some(tape_name) => self.build_tape_messages(tape_name, None).await,
            None => Vec::new(),
        };

        let mut msgs = build_messages(
            prompt,
            user_content.as_deref(),
            system_prompt,
            messages.as_deref(),
        );
        prepend_tape_history(&mut msgs, tape_messages);

        if let Some(tape_name) = tape {
            let new_messages: Vec<Value> = msgs
                .iter()
                .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
                .cloned()
                .collect();
            let run_id = Uuid::new_v4().to_string();
            if let Err(e) = self
                .async_tape
                .record_chat(
                    tape_name,
                    &run_id,
                    system_prompt,
                    None,
                    &new_messages,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(self.core.provider()),
                    Some(self.core.model()),
                )
                .await
            {
                tracing::error!(error = %e, tape = %tape_name, "failed to record streaming chat context");
            }
        }

        let (response, transport, _prov, _model) = self
            .core
            .run_chat_stream(
                msgs,
                None,
                model,
                provider,
                max_tokens,
                None,
                Default::default(),
            )
            .await?;

        let parser = parser_for_transport(transport);
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);

        tokio::spawn(async move {
            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let bytes = match chunk_result {
                    Ok(b) => b,
                    Err(_) => break,
                };
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                // Parse complete SSE lines from the buffer, leaving partial
                // lines for the next chunk.
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim_end_matches('\r').to_owned();
                    buffer = buffer[line_end + 1..].to_owned();

                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            break;
                        }
                        if let Ok(val) = serde_json::from_str::<Value>(data) {
                            let content = parser.extract_chunk_text(&val);
                            if !content.is_empty() && tx.send(content).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(AsyncTextStream::new(stream, None))
    }
}
