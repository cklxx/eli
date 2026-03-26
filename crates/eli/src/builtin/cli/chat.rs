//! Interactive REPL chat session.

/// Start an interactive REPL chat session.
pub(crate) async fn chat_command(
    chat_id: String,
    session_id: Option<String>,
) -> anyhow::Result<()> {
    let session = session_id.unwrap_or_else(|| format!("cli:{chat_id}"));
    let (framework, _builtin) = super::builtin_framework().await;

    // Wire inbound injector so subagent results trigger new turns.
    {
        let fw = framework.clone();
        crate::control_plane::set_inbound_injector(std::sync::Arc::new(move |envelope| {
            let fw = fw.clone();
            Box::pin(async move {
                if let Err(e) = fw.process_inbound(envelope).await {
                    tracing::error!(error = %e, "inject_inbound failed in chat mode");
                }
            })
        }));
    }

    println!("Eli chat session started. Type /quit to exit.");

    let stdin = tokio::io::stdin();
    let reader = tokio::io::BufReader::new(stdin);
    use tokio::io::AsyncBufReadExt;
    let mut lines = reader.lines();

    loop {
        eprint!("> ");
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => break,
            Err(_) => break,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed == "/quit" || trimmed == "quit" {
            println!("Goodbye.");
            break;
        }

        let inbound = serde_json::json!({
            "session_id": session,
            "channel": "cli",
            "chat_id": chat_id,
            "content": trimmed,
            "output_channel": "cli",
        });

        match framework.process_inbound(inbound).await {
            Ok(result) => {
                super::print_usage(&result.usage);
            }
            Err(e) => eprintln!("Error: {e}"),
        }
    }

    Ok(())
}
