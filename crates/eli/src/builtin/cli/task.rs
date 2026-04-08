//! CLI commands for task board management.

use crate::taskboard::{self, NewTask, TaskFilter};

/// Create a new task.
pub(crate) async fn add_command(
    description: String,
    kind: Option<String>,
    priority: u8,
    parent: Option<String>,
) -> anyhow::Result<()> {
    let store = require_store()?;
    let parent_id = parent
        .map(|s| uuid::Uuid::parse_str(&s))
        .transpose()
        .map_err(|e| anyhow::anyhow!("invalid parent ID: {e}"))?;

    let id = store
        .create(NewTask {
            kind: kind.unwrap_or_else(|| "general".into()),
            session_origin: "cli".into(),
            context: serde_json::json!({"prompt": description}),
            parent: parent_id,
            priority,
            metadata: serde_json::Value::Null,
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Created task {id}");
    Ok(())
}

/// List tasks.
pub(crate) async fn list_command(
    status: Option<String>,
    kind: Option<String>,
    limit: usize,
) -> anyhow::Result<()> {
    let store = require_store()?;
    let tasks = store
        .list(TaskFilter {
            status,
            kind,
            limit: Some(limit),
            ..Default::default()
        })
        .await;

    if tasks.is_empty() {
        println!("No tasks.");
        return Ok(());
    }

    println!(
        "{:<10} {:<10} {:<12} {:<4} PROMPT",
        "ID", "STATUS", "KIND", "PRI"
    );
    println!("{}", "-".repeat(70));
    for t in &tasks {
        let prompt = t
            .context
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("(no prompt)");
        let prompt_short = if prompt.len() > 40 {
            format!("{}...", &prompt[..prompt.floor_char_boundary(40)])
        } else {
            prompt.to_string()
        };
        println!(
            "{:<10} {:<10} {:<12} {:<4} {}",
            &t.id.to_string()[..8],
            t.status.label(),
            t.kind,
            t.priority,
            prompt_short
        );
    }
    println!("\n{} task(s)", tasks.len());
    Ok(())
}

/// Show task details.
pub(crate) async fn show_command(task_id: String) -> anyhow::Result<()> {
    let store = require_store()?;
    let id =
        uuid::Uuid::parse_str(&task_id).map_err(|e| anyhow::anyhow!("invalid task ID: {e}"))?;

    match store.get(id).await {
        Some(t) => {
            println!("Task:       {}", t.id);
            println!("Kind:       {}", t.kind);
            println!("Status:     {}", t.status.label());
            println!("Priority:   {}", t.priority);
            println!("Created:    {}", t.created_at.format("%Y-%m-%d %H:%M:%S"));
            println!("Updated:    {}", t.updated_at.format("%Y-%m-%d %H:%M:%S"));
            if let Some(ref agent) = t.assigned_to {
                println!("Assigned:   {agent}");
            }
            if let Some(ref parent) = t.parent {
                println!("Parent:     {parent}");
            }
            println!("Session:    {}", t.session_origin);

            if let Some(prompt) = t.context.get("prompt").and_then(|v| v.as_str()) {
                println!("\nPrompt:\n  {prompt}");
            }
            if let Some(ref result) = t.result {
                println!(
                    "\nResult:\n  {}",
                    serde_json::to_string_pretty(result).unwrap_or_default()
                );
            }

            // Show rich failure info
            if let taskboard::Status::Failed {
                ref error,
                ref agent_id,
                ref stage,
                ref suggested_fix,
                retries,
                ..
            } = t.status
            {
                println!("\nFailure:");
                println!("  Error:    {error}");
                if let Some(agent) = agent_id {
                    println!("  Agent:    {agent}");
                }
                if let Some(s) = stage {
                    println!("  Stage:    {s}");
                }
                println!("  Retries:  {retries}");
                if let Some(fix) = suggested_fix {
                    println!("  Fix:      {fix}");
                }
            }
            Ok(())
        }
        None => {
            anyhow::bail!("task '{task_id}' not found");
        }
    }
}

/// Cancel a task.
pub(crate) async fn cancel_command(task_id: String, reason: Option<String>) -> anyhow::Result<()> {
    let store = require_store()?;
    let id =
        uuid::Uuid::parse_str(&task_id).map_err(|e| anyhow::anyhow!("invalid task ID: {e}"))?;

    store
        .cancel(id, reason.unwrap_or_else(|| "cancelled via CLI".into()))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Task {task_id} cancelled.");
    Ok(())
}

/// Show kanban-style board view.
pub(crate) async fn board_command() -> anyhow::Result<()> {
    let store = require_store()?;
    let tasks = store.list(TaskFilter::default()).await;

    if tasks.is_empty() {
        println!("Board is empty.");
        return Ok(());
    }

    let mut todo = vec![];
    let mut wip = vec![];
    let mut done = vec![];

    for t in &tasks {
        let label = format!(
            "{} [{}] {}",
            &t.id.to_string()[..8],
            t.kind,
            t.context
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("(no prompt)")
                .chars()
                .take(30)
                .collect::<String>()
        );
        match t.status.label() {
            "todo" => todo.push(label),
            "claimed" | "running" | "blocked" => wip.push(label),
            "done" | "failed" | "cancelled" => done.push(label),
            _ => {}
        }
    }

    println!("┌─── TODO ──────────────┬─── WIP ───────────────┬─── DONE ──────────────┐");
    let max = todo.len().max(wip.len()).max(done.len());
    for i in 0..max {
        let t = todo.get(i).map(|s| s.as_str()).unwrap_or("");
        let w = wip.get(i).map(|s| s.as_str()).unwrap_or("");
        let d = done.get(i).map(|s| s.as_str()).unwrap_or("");
        println!(
            "│ {:<22}│ {:<22}│ {:<22}│",
            truncate(t, 22),
            truncate(w, 22),
            truncate(d, 22)
        );
    }
    println!("└───────────────────────┴───────────────────────┴───────────────────────┘");
    println!(
        "  {} todo, {} in progress, {} done",
        todo.len(),
        wip.len(),
        done.len()
    );
    Ok(())
}

/// Show task stats.
pub(crate) async fn stats_command() -> anyhow::Result<()> {
    let store = require_store()?;
    let all = store.list(TaskFilter::default()).await;

    let mut by_status: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for t in &all {
        *by_status.entry(t.status.label()).or_default() += 1;
    }

    println!("Task Board Stats:");
    println!("  Total:     {}", all.len());
    for status in &[
        "todo",
        "claimed",
        "running",
        "done",
        "failed",
        "blocked",
        "cancelled",
    ] {
        let count = by_status.get(status).unwrap_or(&0);
        if *count > 0 {
            println!("  {:<11}{}", format!("{status}:"), count);
        }
    }
    let active = store.active_count().await;
    println!("  Active:    {active}");
    Ok(())
}

fn require_store() -> anyhow::Result<&'static taskboard::store::TaskStore> {
    taskboard::task_store()
        .ok_or_else(|| anyhow::anyhow!("taskboard not initialized — ensure ~/.eli/ exists"))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..s.floor_char_boundary(max.saturating_sub(1))])
    }
}
