//! CLI commands for managing decisions.

use nexil::{TapeQuery, collect_active_decisions};

use crate::builtin::store::{FileTapeStore, ForkTapeStore};
use crate::builtin::tape::TapeService;

/// Resolve the default tape service and tape name for the CLI.
fn default_tape_service() -> anyhow::Result<(TapeService, String)> {
    let _ = dotenvy::dotenv();
    let tapes_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".eli")
        .join("tapes");
    let store = ForkTapeStore::from_sync(FileTapeStore::new(tapes_dir.clone()));
    let service = TapeService::new(tapes_dir.clone(), store);

    // Derive tape name from workspace + session (same logic as the agent)
    let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let tape_name = TapeService::session_tape_name("local", &workspace);
    Ok((service, tape_name))
}

/// List active decisions.
pub(crate) async fn list_command() -> anyhow::Result<()> {
    let (service, tape_name) = default_tape_service()?;
    let query = TapeQuery::new(&tape_name);
    let entries = service.store().fetch_all(&query).await?;
    let decisions = collect_active_decisions(&entries);

    if decisions.is_empty() {
        println!("No active decisions.");
        return Ok(());
    }

    println!("Active decisions ({}):", decisions.len());
    for (i, d) in decisions.iter().enumerate() {
        println!("  {}. {}", i + 1, d);
    }
    Ok(())
}

/// Remove a decision by ordinal index.
pub(crate) async fn remove_command(index: usize) -> anyhow::Result<()> {
    if index == 0 {
        anyhow::bail!("index must be 1 or greater");
    }
    let (service, tape_name) = default_tape_service()?;
    let query = TapeQuery::new(&tape_name);
    let entries = service.store().fetch_all(&query).await?;
    let decisions = collect_active_decisions(&entries);

    if index > decisions.len() {
        anyhow::bail!(
            "no decision #{index}. There are {} active decisions.",
            decisions.len()
        );
    }

    let text = &decisions[index - 1];
    let meta = serde_json::json!({});
    let tombstone = nexil::TapeEntry::decision_revoked(text, meta);
    service.store().append(&tape_name, &tombstone).await?;
    println!("Removed decision: {text}");
    Ok(())
}

/// Export decisions as markdown.
pub(crate) async fn export_command() -> anyhow::Result<()> {
    let (service, tape_name) = default_tape_service()?;
    let query = TapeQuery::new(&tape_name);
    let entries = service.store().fetch_all(&query).await?;
    let decisions = collect_active_decisions(&entries);

    if decisions.is_empty() {
        println!("No active decisions to export.");
        return Ok(());
    }

    eprintln!(
        "WARNING: Exported decisions may contain sensitive information discussed in conversation."
    );
    println!("# Decisions\n");
    for (i, d) in decisions.iter().enumerate() {
        println!("{}. {}", i + 1, d);
    }
    Ok(())
}
