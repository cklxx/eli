//! CLI commands for governed self-evolution.

use std::path::PathBuf;

use crate::builtin::config::eli_home;
use crate::evolution::{
    CandidateKind, CandidateStatus, DistillOutcome, EvaluationRun, EvolutionStore,
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

pub(crate) async fn distill_command(tape: String, persist: bool) -> anyhow::Result<()> {
    let outcome = default_store()?.distill_tape(&default_tapes_dir()?, &tape, persist)?;
    println!("{}", render_distill_result(&outcome));
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

fn kind_label(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::PromptRule => "prompt_rule",
        CandidateKind::Skill => "skill",
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
