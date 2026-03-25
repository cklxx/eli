//! One-shot run command.

/// Run a single message through the agent.
pub(crate) async fn run_command(
    message: String,
    channel: String,
    chat_id: String,
    sender_id: String,
    session_id: Option<String>,
) -> anyhow::Result<()> {
    let session = session_id.unwrap_or_else(|| format!("{channel}:{chat_id}"));
    let (framework, _builtin) = super::builtin_framework().await;
    let inbound = serde_json::json!({
        "session_id": session,
        "channel": channel,
        "chat_id": chat_id,
        "sender_id": sender_id,
        "content": message,
        "output_channel": "cli",
    });

    match framework.process_inbound(inbound).await {
        Ok(result) => {
            tracing::debug!(session_id = %result.session_id, "run complete");
            super::print_usage(&result.usage);
        }
        Err(e) => {
            eprintln!("Error: {e}");
        }
    }

    Ok(())
}
