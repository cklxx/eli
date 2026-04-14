use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use nexil::tape::TapeStore;
use nexil::{TapeEntry, TapeEntryKind, TapeQuery, collect_active_decisions};

use crate::builtin::store::FileTapeStore;

use super::{EvolutionCandidate, EvolutionStore, NewCandidate, trimmed};

const MAX_EXAMPLES: usize = 3;
const MAX_OUTPUT_CHARS: usize = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistillEvidenceSummary {
    pub tape_entries: usize,
    pub successful_runs: usize,
    pub successful_commands: usize,
    pub active_decisions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistillSkip {
    pub title: String,
    pub fingerprint: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistillOutcome {
    pub tape: String,
    pub persisted: bool,
    pub evidence: DistillEvidenceSummary,
    pub candidates: Vec<EvolutionCandidate>,
    pub skipped: Vec<DistillSkip>,
}

pub(super) fn distill_tape(
    store: &EvolutionStore,
    tapes_dir: &Path,
    tape_name: &str,
    persist: bool,
) -> anyhow::Result<DistillOutcome> {
    let entries = load_tape_entries(tapes_dir, tape_name)?;
    let evidence = collect_evidence(&entries);
    let existing = existing_fingerprints(store)?;
    let specs = build_specs(tape_name, &evidence);
    let drafted = draft_candidates(store, tape_name, specs, &existing)?;
    let created = persist_candidates(store, persist, drafted.drafts)?;
    Ok(DistillOutcome {
        tape: tape_name.to_owned(),
        persisted: persist,
        evidence: evidence.summary(),
        candidates: created,
        skipped: drafted.skipped,
    })
}

struct EvidenceSet {
    tape_entries: usize,
    runs: Vec<RunEvidence>,
    commands: Vec<CommandEvidence>,
    decisions: Vec<DecisionEvidence>,
}

struct DraftBatch {
    drafts: Vec<EvolutionCandidate>,
    skipped: Vec<DistillSkip>,
}

#[derive(Clone)]
struct RunEvidence {
    entry_id: i64,
    provider: String,
    model: String,
    elapsed_ms: Option<i64>,
    total_tokens: Option<i64>,
}

#[derive(Clone)]
struct CommandEvidence {
    entry_id: i64,
    name: String,
    raw: String,
    output: String,
    elapsed_ms: Option<i64>,
}

#[derive(Clone)]
struct DecisionEvidence {
    entry_id: i64,
    text: String,
}

#[derive(Clone)]
struct DistillSpec {
    title: String,
    summary: String,
    content: String,
    evidence_ids: Vec<String>,
}

impl EntryId for RunEvidence {
    fn entry_id(&self) -> i64 {
        self.entry_id
    }
}

impl EntryId for CommandEvidence {
    fn entry_id(&self) -> i64 {
        self.entry_id
    }
}

impl EntryId for DecisionEvidence {
    fn entry_id(&self) -> i64 {
        self.entry_id
    }
}

impl EvidenceSet {
    fn summary(&self) -> DistillEvidenceSummary {
        DistillEvidenceSummary {
            tape_entries: self.tape_entries,
            successful_runs: self.runs.len(),
            successful_commands: self.commands.len(),
            active_decisions: self.decisions.len(),
        }
    }
}

fn collect_evidence(entries: &[TapeEntry]) -> EvidenceSet {
    EvidenceSet {
        tape_entries: entries.len(),
        runs: collect_runs(entries),
        commands: collect_commands(entries),
        decisions: collect_decisions(entries),
    }
}

fn collect_runs(entries: &[TapeEntry]) -> Vec<RunEvidence> {
    sorted_by_entry_id(entries.iter().filter_map(run_evidence).collect())
}

fn collect_commands(entries: &[TapeEntry]) -> Vec<CommandEvidence> {
    sorted_by_entry_id(entries.iter().filter_map(command_evidence).collect())
}

fn collect_decisions(entries: &[TapeEntry]) -> Vec<DecisionEvidence> {
    let active = active_decision_texts(entries);
    sorted_by_entry_id(
        entries
            .iter()
            .filter_map(|entry| decision_evidence(entry, &active))
            .collect(),
    )
}

fn active_decision_texts(entries: &[TapeEntry]) -> HashSet<String> {
    collect_active_decisions(entries).into_iter().collect()
}

fn run_evidence(entry: &TapeEntry) -> Option<RunEvidence> {
    if !is_success_event(entry, &["agent.run", "run"]) {
        return None;
    }
    Some(RunEvidence {
        entry_id: entry.id,
        provider: event_data_text(entry, "provider"),
        model: event_data_text(entry, "model"),
        elapsed_ms: event_data_number(entry, "elapsed_ms"),
        total_tokens: event_data_number_in_data(entry, "usage", "total_tokens"),
    })
}

fn command_evidence(entry: &TapeEntry) -> Option<CommandEvidence> {
    if !is_success_event(entry, &["command"]) {
        return None;
    }
    Some(CommandEvidence {
        entry_id: entry.id,
        name: event_data_text(entry, "name"),
        raw: event_data_text(entry, "raw"),
        output: event_data_text(entry, "output"),
        elapsed_ms: event_data_number(entry, "elapsed_ms"),
    })
}

fn decision_evidence(entry: &TapeEntry, active: &HashSet<String>) -> Option<DecisionEvidence> {
    if entry.kind != TapeEntryKind::Decision {
        return None;
    }
    let text = entry_text(entry, "text");
    if text.is_empty() || !active.contains(&text) {
        return None;
    }
    Some(DecisionEvidence {
        entry_id: entry.id,
        text,
    })
}

fn is_success_event(entry: &TapeEntry, names: &[&str]) -> bool {
    entry.kind == TapeEntryKind::Event
        && names.contains(&event_name(entry).as_str())
        && event_data_text(entry, "status") == "ok"
}

fn sorted_by_entry_id<T: EntryId>(mut items: Vec<T>) -> Vec<T> {
    items.sort_by_key(EntryId::entry_id);
    items
}

fn draft_candidates(
    store: &EvolutionStore,
    tape_name: &str,
    specs: Vec<DistillSpec>,
    existing: &HashSet<String>,
) -> anyhow::Result<DraftBatch> {
    let mut drafts = Vec::new();
    let mut skipped = Vec::new();
    for spec in specs {
        let candidate = build_candidate(store, tape_name, &spec)?;
        let fingerprint = candidate.effective_fingerprint();
        if existing.contains(&fingerprint) {
            skipped.push(DistillSkip {
                title: candidate.title,
                fingerprint,
                reason: "duplicate candidate already exists".to_owned(),
            });
            continue;
        }
        drafts.push(candidate);
    }
    Ok(DraftBatch { drafts, skipped })
}

fn build_candidate(
    store: &EvolutionStore,
    tape_name: &str,
    spec: &DistillSpec,
) -> anyhow::Result<EvolutionCandidate> {
    let input = NewCandidate::rule(
        &spec.title,
        &spec.summary,
        &spec.content,
        Some(tape_name.to_owned()),
        "distill_tape",
    )?;
    let mut candidate = store.new_candidate(input)?;
    candidate.evidence_ids = spec.evidence_ids.clone();
    Ok(candidate)
}

fn build_specs(tape_name: &str, evidence: &EvidenceSet) -> Vec<DistillSpec> {
    [
        run_spec(tape_name, &evidence.runs),
        command_spec(tape_name, &evidence.commands),
        decision_spec(tape_name, &evidence.decisions),
    ]
    .into_iter()
    .flatten()
    .collect()
}

trait EntryId {
    fn entry_id(&self) -> i64;
}

fn run_spec(tape_name: &str, runs: &[RunEvidence]) -> Option<DistillSpec> {
    (!runs.is_empty()).then(|| DistillSpec {
        title: "Preserve successful agent run patterns".to_owned(),
        summary: format!(
            "Distilled from {} successful agent.run events on tape {tape_name}.",
            runs.len()
        ),
        content: run_spec_content(runs),
        evidence_ids: runs
            .iter()
            .map(|run| format!("event:{}", run.entry_id))
            .collect(),
    })
}

fn command_spec(tape_name: &str, commands: &[CommandEvidence]) -> Option<DistillSpec> {
    (!commands.is_empty()).then(|| DistillSpec {
        title: "Reuse successful shell command patterns".to_owned(),
        summary: format!(
            "Distilled from {} successful command events on tape {tape_name}.",
            commands.len()
        ),
        content: command_spec_content(commands),
        evidence_ids: commands
            .iter()
            .map(|command| format!("event:{}", command.entry_id))
            .collect(),
    })
}

fn decision_spec(tape_name: &str, decisions: &[DecisionEvidence]) -> Option<DistillSpec> {
    (!decisions.is_empty()).then(|| DistillSpec {
        title: "Preserve active session decisions".to_owned(),
        summary: format!(
            "Distilled from {} active decisions on tape {tape_name}.",
            decisions.len()
        ),
        content: decision_spec_content(decisions),
        evidence_ids: decisions
            .iter()
            .map(|decision| format!("decision:{}", decision.entry_id))
            .collect(),
    })
}

fn run_spec_content(runs: &[RunEvidence]) -> String {
    bullet_lines([
        String::from("- Observed successful agent runs on this tape."),
        format!("- Example run: {}.", run_example(runs)),
        String::from(
            "- Reuse the same model, provider, and token budget when the next task has the same shape.",
        ),
    ])
}

fn command_spec_content(commands: &[CommandEvidence]) -> String {
    bullet_lines([
        String::from("- Observed successful shell commands on this tape."),
        format!("- Example command: {}.", command_example(commands)),
        String::from(
            "- Prefer the shortest successful shell form that reproduces the same result.",
        ),
    ])
}

fn decision_spec_content(decisions: &[DecisionEvidence]) -> String {
    bullet_lines([
        String::from("- Active decisions are part of the current session policy."),
        String::from("- Preserve these commitments while they remain active."),
        format!("- {}.", decisions_example(decisions)),
    ])
}

fn run_example(runs: &[RunEvidence]) -> String {
    runs.first()
        .map(|run| {
            format!(
                "provider `{}`, model `{}`, elapsed {}ms, total tokens {}",
                non_empty(&run.provider),
                non_empty(&run.model),
                display_number(run.elapsed_ms),
                display_number(run.total_tokens),
            )
        })
        .unwrap_or_else(|| "no successful runs".to_owned())
}

fn command_example(commands: &[CommandEvidence]) -> String {
    commands
        .first()
        .map(|command| {
            format!(
                "`{}` as `{}` -> `{}` (elapsed {}ms)",
                non_empty(&command.raw),
                non_empty(&command.name),
                truncate_text(&command.output, MAX_OUTPUT_CHARS),
                display_number(command.elapsed_ms),
            )
        })
        .unwrap_or_else(|| "no successful commands".to_owned())
}

fn decisions_example(decisions: &[DecisionEvidence]) -> String {
    decisions
        .iter()
        .take(MAX_EXAMPLES)
        .map(|decision| format!("Decision: {}", decision.text))
        .collect::<Vec<_>>()
        .join("\n- ")
}

fn existing_fingerprints(store: &EvolutionStore) -> anyhow::Result<HashSet<String>> {
    Ok(store
        .list_candidates()?
        .into_iter()
        .map(|candidate| candidate.effective_fingerprint())
        .collect())
}

fn persist_candidates(
    store: &EvolutionStore,
    persist: bool,
    drafts: Vec<EvolutionCandidate>,
) -> anyhow::Result<Vec<EvolutionCandidate>> {
    if !persist {
        return Ok(drafts);
    }
    drafts
        .into_iter()
        .map(|candidate| persist_candidate(store, candidate))
        .collect()
}

fn persist_candidate(
    store: &EvolutionStore,
    candidate: EvolutionCandidate,
) -> anyhow::Result<EvolutionCandidate> {
    store.write_candidate(&candidate)?;
    Ok(candidate)
}

fn load_tape_entries(tapes_dir: &Path, tape_name: &str) -> anyhow::Result<Vec<TapeEntry>> {
    let store = FileTapeStore::new(tapes_dir.to_path_buf());
    Ok(store.fetch_all(&TapeQuery::new(tape_name))?)
}

fn event_name(entry: &TapeEntry) -> String {
    entry
        .payload
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_owned()
}

fn event_data(entry: &TapeEntry) -> Option<&Value> {
    entry.payload.get("data")
}

fn event_data_text(entry: &TapeEntry, key: &str) -> String {
    event_data(entry)
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_owned()
}

fn event_data_number(entry: &TapeEntry, key: &str) -> Option<i64> {
    event_data(entry)
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_i64())
}

fn event_data_number_in_data(entry: &TapeEntry, parent: &str, key: &str) -> Option<i64> {
    event_data(entry)
        .and_then(|value| value.get(parent))
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_i64())
}

fn entry_text(entry: &TapeEntry, key: &str) -> String {
    entry
        .payload
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_owned()
}

fn bullet_lines<I, S>(lines: I) -> String
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    lines
        .into_iter()
        .map(Into::into)
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let trimmed = trimmed(value);
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    let mut text = trimmed.chars().take(max_chars).collect::<String>();
    text.push_str("...");
    text
}

fn display_number(value: Option<i64>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn non_empty(value: &str) -> &str {
    let trimmed = trimmed(value);
    if trimmed.is_empty() { "-" } else { trimmed }
}

#[cfg(test)]
mod tests {
    use super::super::CandidateKind;
    use super::*;
    use std::fs;

    fn store(tmp: &tempfile::TempDir) -> EvolutionStore {
        EvolutionStore::new(tmp.path())
    }

    fn write_tape(tmp: &tempfile::TempDir, tape: &str, entries: &[TapeEntry]) {
        let path = tmp.path().join(format!("{tape}.jsonl"));
        let body = entries
            .iter()
            .map(|entry| serde_json::to_string(entry).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, format!("{body}\n")).unwrap();
    }

    fn run_event(status: &str) -> TapeEntry {
        TapeEntry::event(
            "agent.run",
            Some(serde_json::json!({
                "status": status,
                "elapsed_ms": 41,
                "provider": "openai",
                "model": "gpt-5",
                "usage": { "total_tokens": 120 },
            })),
            Value::Object(Default::default()),
        )
    }

    fn command_event(status: &str) -> TapeEntry {
        TapeEntry::event(
            "command",
            Some(serde_json::json!({
                "status": status,
                "raw": "cargo test",
                "name": "cargo",
                "elapsed_ms": 88,
                "output": "ok",
            })),
            Value::Object(Default::default()),
        )
    }

    #[test]
    fn test_distill_tape_dry_run_synthesizes_prompt_rules_only() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = vec![
            run_event("ok"),
            command_event("ok"),
            TapeEntry::decision("Keep feedback concise", Value::Object(Default::default())),
            TapeEntry::event(
                "agent.run",
                Some(serde_json::json!({ "status": "error" })),
                Value::Object(Default::default()),
            ),
        ];
        write_tape(&tmp, "demo__tape", &entries);
        let outcome = store(&tmp)
            .distill_tape(tmp.path(), "demo__tape", false)
            .unwrap();
        assert!(!outcome.persisted);
        assert_eq!(outcome.candidates.len(), 3);
        assert!(
            outcome
                .candidates
                .iter()
                .all(|c| c.kind == CandidateKind::PromptRule)
        );
        assert_eq!(outcome.evidence.successful_runs, 1);
        assert_eq!(outcome.evidence.successful_commands, 1);
        assert_eq!(outcome.evidence.active_decisions, 1);
    }

    #[test]
    fn test_distill_tape_persist_writes_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = vec![run_event("ok"), command_event("ok")];
        write_tape(&tmp, "demo__tape", &entries);
        let outcome = store(&tmp)
            .distill_tape(tmp.path(), "demo__tape", true)
            .unwrap();
        let candidates = store(&tmp).list_candidates().unwrap();
        assert_eq!(outcome.candidates.len(), 2);
        assert_eq!(candidates.len(), 2);
        assert!(
            candidates
                .iter()
                .all(|c| c.status == super::super::CandidateStatus::Pending)
        );
    }

    #[test]
    fn test_distill_tape_skips_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = vec![run_event("ok")];
        let store = store(&tmp);
        write_tape(&tmp, "demo__tape", &entries);
        let first = store.distill_tape(tmp.path(), "demo__tape", true).unwrap();
        let second = store.distill_tape(tmp.path(), "demo__tape", false).unwrap();
        assert_eq!(first.candidates.len(), 1);
        assert!(second.candidates.is_empty());
        assert_eq!(second.skipped.len(), 1);
    }

    #[test]
    fn test_distill_rule_fingerprint_stays_stable_when_tape_grows() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(&tmp);
        write_tape(&tmp, "demo__tape", &[run_event("ok")]);
        let first = store.distill_tape(tmp.path(), "demo__tape", false).unwrap();
        write_tape(
            &tmp,
            "demo__tape",
            &[run_event("ok"), run_event("ok"), command_event("ok")],
        );
        let second = store.distill_tape(tmp.path(), "demo__tape", false).unwrap();
        assert_eq!(
            first.candidates[0].effective_fingerprint(),
            second.candidates[0].effective_fingerprint()
        );
    }

    #[test]
    fn test_distill_tape_reads_legacy_timestamp_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = serde_json::json!({
            "id": 1,
            "kind": "event",
            "payload": {
                "name": "agent.run",
                "data": { "status": "ok", "usage": { "total_tokens": 9 } }
            },
            "meta": {},
            "timestamp": 1_700_000_000.0
        });
        let path = tmp.path().join("legacy__tape.jsonl");
        fs::write(path, format!("{legacy}\n")).unwrap();
        let outcome = store(&tmp)
            .distill_tape(tmp.path(), "legacy__tape", false)
            .unwrap();
        assert_eq!(outcome.evidence.successful_runs, 1);
    }
}
