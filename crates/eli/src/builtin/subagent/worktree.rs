//! Git worktree isolation for agents.

use std::path::{Path, PathBuf};

/// Outcome after an agent finishes in a worktree.
pub enum WorktreeOutcome {
    /// No changes were made; worktree was removed.
    NoChanges,
    /// Changes were made; worktree preserved at the given path.
    HasChanges { path: PathBuf, branch: String },
    /// Worktree was not a git repo or worktree creation failed.
    NotApplicable(String),
}

/// Create a temporary git worktree under `.claude/worktrees/`.
pub async fn create_worktree(workspace: &Path) -> Result<PathBuf, String> {
    // Verify we're in a git repo.
    let is_git = tokio::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !is_git {
        return Err("not a git repository".to_owned());
    }

    let slug = &uuid::Uuid::new_v4().to_string()[..8];
    let worktree_dir = workspace.join(".claude").join("worktrees").join(slug);

    let output = tokio::process::Command::new("git")
        .args([
            "worktree",
            "add",
            &worktree_dir.to_string_lossy(),
            "--detach",
        ])
        .current_dir(workspace)
        .output()
        .await
        .map_err(|e| format!("git worktree add: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree add failed: {stderr}"));
    }

    tracing::info!(path = %worktree_dir.display(), "created agent worktree");
    Ok(worktree_dir)
}

/// Check if a worktree has changes and clean up if not.
pub async fn cleanup_worktree(worktree_path: &Path) -> WorktreeOutcome {
    // Check for changes.
    let has_changes = tokio::process::Command::new("git")
        .args(["diff", "--stat", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .await
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false);

    // Also check for untracked files.
    let has_untracked = tokio::process::Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(worktree_path)
        .output()
        .await
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false);

    if !has_changes && !has_untracked {
        // No changes — remove the worktree.
        let _ = tokio::process::Command::new("git")
            .args(["worktree", "remove", &worktree_path.to_string_lossy()])
            .output()
            .await;
        tracing::info!(path = %worktree_path.display(), "removed clean agent worktree");
        return WorktreeOutcome::NoChanges;
    }

    // Get the branch/detached HEAD info.
    let branch = tokio::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "HEAD".to_owned());

    tracing::info!(
        path = %worktree_path.display(),
        branch = %branch,
        "agent worktree has changes, keeping"
    );
    WorktreeOutcome::HasChanges {
        path: worktree_path.to_path_buf(),
        branch,
    }
}

/// Sweep and remove stale worktrees (e.g., from crashed agents).
/// Call this at startup.
pub async fn sweep_stale_worktrees(workspace: &Path) {
    let worktrees_dir = workspace.join(".claude").join("worktrees");
    if !worktrees_dir.is_dir() {
        return;
    }
    let entries = match std::fs::read_dir(&worktrees_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // If worktree has no changes, clean it up.
        match cleanup_worktree(&path).await {
            WorktreeOutcome::NoChanges => {
                tracing::info!(path = %path.display(), "swept stale worktree");
            }
            WorktreeOutcome::HasChanges { .. } => {
                tracing::warn!(
                    path = %path.display(),
                    "stale worktree has changes, keeping for manual review"
                );
            }
            WorktreeOutcome::NotApplicable(_) => {
                // Not a valid worktree — just remove the directory.
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }
}
