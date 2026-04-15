//! CLI commands for governed self-evolution.

use std::path::PathBuf;

use crate::builtin::config::eli_home;
use crate::evolution::{
    AutoEvolutionPolicy, AutoJournalEntry, CandidateKind, CandidateStatus, DistillOutcome,
    EvaluationRun, EvolutionStore,
};

pub(crate) async fn list_command(status: Option<CandidateStatus>) -> anyhow::Result<()> {
    let store = default_store()?;
    let candidates = filter_status(store.list_candidates()?, status);
    if candidates.is_empty() {
        println!("No evolution candidates.");
        return Ok(());
    }
    for candidate in candidates {
        println!("{}", render_candidate_summary(&candidate));
    }
    Ok(())
}

pub(crate) async fn show_command(id: String) -> anyhow::Result<()> {
    let candidate = default_store()?.read_candidate(&id)?;
    println!("{}", render_candidate_detail(&candidate));
    Ok(())
}

pub(crate) async fn history_command(limit: usize) -> anyhow::Result<()> {
    let entries = default_store()?.load_auto_journal()?;
    println!("{}", render_history_output(&entries, limit));
    Ok(())
}

pub(crate) async fn distill_command(tape: String, persist: bool) -> anyhow::Result<()> {
    let outcome = default_store()?.distill_tape(&default_tapes_dir()?, &tape, persist)?;
    println!("{}", render_distill_result(&outcome));
    Ok(())
}

pub(crate) async fn auto_run_command(tape: String) -> anyhow::Result<()> {
    let store = default_store()?;
    let Some(policy) = store
        .load_runtime_policy()?
        .apply_to_auto_policy(AutoEvolutionPolicy::default())
    else {
        anyhow::bail!("auto evolution disabled by runtime policy");
    };
    let outcome = store.auto_evolve_tape(&default_tapes_dir()?, &tape, policy)?;
    println!("{}", render_auto_run_result(&outcome));
    Ok(())
}

pub(crate) async fn evaluate_command(id: String) -> anyhow::Result<()> {
    let run = default_store()?.evaluate(&id)?;
    println!("{}", render_evaluation(&run));
    Ok(())
}

pub(crate) async fn capture_rule_command(
    title: String,
    summary: String,
    content: String,
) -> anyhow::Result<()> {
    let candidate = default_store()?.capture_rule(&title, &summary, &content, None, "cli")?;
    println!("Captured prompt-rule candidate {}", candidate.id);
    Ok(())
}

pub(crate) async fn capture_skill_command(
    skill_name: String,
    title: Option<String>,
    description: String,
    content: String,
) -> anyhow::Result<()> {
    let title = title.unwrap_or_else(|| skill_name.clone());
    let candidate =
        default_store()?.capture_skill(&skill_name, &title, &description, &content, None, "cli")?;
    println!("Captured skill candidate {}", candidate.id);
    Ok(())
}

pub(crate) async fn capture_knowledge_command(
    artifact_name: String,
    title: Option<String>,
    summary: String,
    content: String,
) -> anyhow::Result<()> {
    let title = title.unwrap_or_else(|| artifact_name.clone());
    let candidate = default_store()?.capture_compiled_knowledge(
        &artifact_name,
        &title,
        &summary,
        &content,
        None,
        "cli",
    )?;
    println!("Captured compiled-knowledge candidate {}", candidate.id);
    Ok(())
}

pub(crate) async fn capture_runtime_policy_command(
    artifact_name: String,
    title: Option<String>,
    summary: String,
    content: String,
) -> anyhow::Result<()> {
    let title = title.unwrap_or_else(|| artifact_name.clone());
    let candidate = default_store()?.capture_runtime_policy(
        &artifact_name,
        &title,
        &summary,
        &content,
        None,
        "cli",
    )?;
    println!("Captured runtime-policy candidate {}", candidate.id);
    Ok(())
}

pub(crate) async fn promote_command(id: String, force: bool) -> anyhow::Result<()> {
    let outcome = default_store()?.promote(&id, force)?;
    println!(
        "Promoted {} to {}",
        outcome.candidate.id,
        outcome.target.display()
    );
    Ok(())
}

pub(crate) async fn reject_command(id: String) -> anyhow::Result<()> {
    let candidate = default_store()?.reject(&id)?;
    println!("Rejected candidate {}", candidate.id);
    Ok(())
}

pub(crate) async fn rollback_command(id: String) -> anyhow::Result<()> {
    let outcome = default_store()?.rollback(&id)?;
    println!(
        "Rolled back {} to {}",
        outcome.candidate.id,
        outcome.target.display()
    );
    Ok(())
}

fn default_store() -> anyhow::Result<EvolutionStore> {
    Ok(EvolutionStore::new(default_workspace()?))
}

fn default_workspace() -> anyhow::Result<PathBuf> {
    Ok(std::env::current_dir()?)
}

fn default_tapes_dir() -> anyhow::Result<PathBuf> {
    Ok(eli_home().join("tapes"))
}

fn filter_status(
    candidates: Vec<crate::evolution::EvolutionCandidate>,
    status: Option<CandidateStatus>,
) -> Vec<crate::evolution::EvolutionCandidate> {
    candidates
        .into_iter()
        .filter(|candidate| status.is_none_or(|expected| candidate.status == expected))
        .collect()
}

fn render_candidate_summary(candidate: &crate::evolution::EvolutionCandidate) -> String {
    format!(
        "{}  {}  {}  {}",
        candidate.id,
        status_label(candidate.status),
        kind_label(candidate.kind),
        candidate.title
    )
}

fn render_candidate_detail(candidate: &crate::evolution::EvolutionCandidate) -> String {
    [
        format!("id: {}", candidate.id),
        format!("status: {}", status_label(candidate.status)),
        format!("kind: {}", kind_label(candidate.kind)),
        format!("title: {}", candidate.title),
        format!("summary: {}", candidate.summary),
        format!("source: {}", candidate.source),
        format!(
            "source_tape: {}",
            candidate.source_tape.clone().unwrap_or_default()
        ),
        format!("risk_level: {}", candidate.risk_level_string()),
        format!("fingerprint: {}", candidate.effective_fingerprint()),
        format!("requires_evaluation: {}", candidate.requires_evaluation),
        format!(
            "latest_evaluation_id: {}",
            candidate.latest_evaluation_id.clone().unwrap_or_default()
        ),
        format!(
            "evaluation_passed: {}",
            candidate
                .evaluation_passed
                .map(|value| value.to_string())
                .unwrap_or_default()
        ),
        format!(
            "promoted_to: {}",
            candidate.promoted_to.clone().unwrap_or_default()
        ),
        String::new(),
        candidate.content.clone(),
    ]
    .join("\n")
}

fn render_distill_result(outcome: &DistillOutcome) -> String {
    let mode = if outcome.persisted {
        "Distilled"
    } else {
        "Previewed"
    };
    format!(
        "{mode} tape {}: {} prompt-rule candidates, {} skipped.",
        outcome.tape,
        outcome.candidates.len(),
        outcome.skipped.len()
    )
}

fn render_evaluation(run: &EvaluationRun) -> String {
    let mut lines = vec![
        format!("id: {}", run.id),
        format!("candidate_id: {}", run.candidate_id),
        format!("passed: {}", run.passed),
        format!("score: {}", run.score),
    ];
    lines.extend(run.checks.iter().map(render_check));
    lines.join("\n")
}

fn render_check(check: &crate::evolution::EvaluationCheck) -> String {
    format!("- {}: {} ({})", check.name, check.passed, check.detail)
}

fn render_auto_run_result(outcome: &crate::evolution::AutoEvolutionReport) -> String {
    format!(
        "Auto-ran tape {}: distilled {}, skipped {}, evaluated {}, observed {}, staged {}, promoted {}, expired {}.",
        outcome.tape,
        outcome.distill.candidates.len(),
        outcome.distill.skipped.len(),
        outcome.evaluations.len(),
        outcome.observed.len(),
        outcome.staged.len(),
        outcome.promoted.len(),
        outcome.expired.len(),
    )
}

fn render_history_output(entries: &[AutoJournalEntry], limit: usize) -> String {
    let lines = history_lines(entries, limit);
    if lines.is_empty() {
        "No evolution history.".to_owned()
    } else {
        lines.join("\n")
    }
}

fn kind_label(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::PromptRule => "prompt_rule",
        CandidateKind::Skill => "skill",
        CandidateKind::CompiledKnowledge => "compiled_knowledge",
        CandidateKind::RuntimePolicy => "runtime_policy",
    }
}

fn status_label(status: CandidateStatus) -> &'static str {
    match status {
        CandidateStatus::Pending => "pending",
        CandidateStatus::Promoted => "promoted",
        CandidateStatus::Rejected => "rejected",
        CandidateStatus::RolledBack => "rolled_back",
    }
}

fn history_lines(entries: &[AutoJournalEntry], limit: usize) -> Vec<String> {
    let mut entries = entries.to_vec();
    entries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    entries
        .into_iter()
        .take(limit)
        .map(render_history_entry)
        .collect()
}

fn render_history_entry(entry: AutoJournalEntry) -> String {
    format!(
        "{}  {}  {}  {}  {}",
        entry.created_at,
        render_action(entry.action),
        entry.candidate_id.unwrap_or_default(),
        shorten(&entry.tape, 24),
        entry.detail,
    )
}

fn render_action(action: crate::evolution::AutoJournalAction) -> &'static str {
    match action {
        crate::evolution::AutoJournalAction::Distilled => "distilled",
        crate::evolution::AutoJournalAction::Evaluated => "evaluated",
        crate::evolution::AutoJournalAction::Observed => "observed",
        crate::evolution::AutoJournalAction::Staged => "staged",
        crate::evolution::AutoJournalAction::Promoted => "promoted",
        crate::evolution::AutoJournalAction::Rejected => "rejected",
        crate::evolution::AutoJournalAction::RolledBack => "rolled_back",
        crate::evolution::AutoJournalAction::Expired => "expired",
        crate::evolution::AutoJournalAction::Held => "held",
    }
}

fn shorten(text: &str, width: usize) -> String {
    let mut chars = text.chars();
    let body: String = chars.by_ref().take(width).collect();
    if chars.next().is_none() {
        body
    } else {
        format!("{body}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tmp: &tempfile::TempDir) -> EvolutionStore {
        EvolutionStore::new(tmp.path())
    }

    #[test]
    fn test_render_auto_run_result_summarizes_counts() {
        let outcome = crate::evolution::AutoEvolutionReport {
            tape: "abc123".to_owned(),
            policy: AutoEvolutionPolicy::default(),
            distill: DistillOutcome {
                tape: "abc123".to_owned(),
                persisted: true,
                evidence: crate::evolution::DistillEvidenceSummary {
                    tape_entries: 3,
                    successful_runs: 1,
                    successful_commands: 1,
                    active_decisions: 1,
                },
                candidates: vec![
                    store(&tempfile::tempdir().unwrap())
                        .capture_rule("a", "b", "- c", None, "test")
                        .unwrap(),
                ],
                skipped: vec![crate::evolution::DistillSkip {
                    title: "skip".to_owned(),
                    fingerprint: "fp".to_owned(),
                    reason: "duplicate".to_owned(),
                }],
            },
            evaluations: vec![],
            observed: vec![],
            staged: vec![],
            promoted: vec![],
            expired: vec![],
            journal_path: "/tmp/journal".to_owned(),
            journaled_actions: 0,
        };
        assert_eq!(
            render_auto_run_result(&outcome),
            "Auto-ran tape abc123: distilled 1, skipped 1, evaluated 0, observed 0, staged 0, promoted 0, expired 0."
        );
    }

    #[test]
    fn test_history_lines_orders_and_limits_entries() {
        let lines = history_lines(
            &[
                entry(
                    "1",
                    "tape-a",
                    crate::evolution::AutoJournalAction::Distilled,
                ),
                entry("2", "tape-b", crate::evolution::AutoJournalAction::Promoted),
                entry("3", "tape-c", crate::evolution::AutoJournalAction::Observed),
            ],
            2,
        );
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("observed"));
        assert!(lines[1].contains("promoted"));
    }

    #[test]
    fn test_history_lines_handles_empty_workspace() {
        assert_eq!(render_history_output(&[], 5), "No evolution history.");
    }

    #[test]
    fn test_history_lines_renders_manual_actions() {
        let lines = history_lines(
            &[entry(
                "1",
                "tape-a",
                crate::evolution::AutoJournalAction::RolledBack,
            )],
            1,
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("rolled_back"));
    }

    fn entry(
        ts: &str,
        tape: &str,
        action: crate::evolution::AutoJournalAction,
    ) -> AutoJournalEntry {
        AutoJournalEntry {
            id: format!("journal-{ts}"),
            tape: tape.to_owned(),
            candidate_id: Some(format!("cand-{ts}")),
            action,
            detail: format!("detail-{ts}"),
            created_at: format!("2026-04-14T00:00:0{ts}Z"),
        }
    }
}
