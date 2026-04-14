//! Governed self-evolution storage for prompt rules and skill candidates.

mod distill;
mod eval;

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::builtin::config::eli_home;
use crate::skills::is_valid_skill_name;

const EVOLUTION_DIR: &str = ".agents/evolution";
const CANDIDATES_DIR: &str = "candidates";
const EVALUATIONS_DIR: &str = "evaluations";
const PROMOTIONS_DIR: &str = "promotions";
const SNAPSHOTS_DIR: &str = "snapshots";
const RULES_FILE: &str = "rules.md";

pub use distill::{DistillEvidenceSummary, DistillOutcome, DistillSkip};
pub use eval::{EvaluationCheck, EvaluationRun};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    PromptRule,
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Pending,
    Promoted,
    Rejected,
    RolledBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionCandidate {
    pub id: String,
    pub kind: CandidateKind,
    pub title: String,
    pub summary: String,
    pub content: String,
    pub skill_name: Option<String>,
    pub source_tape: Option<String>,
    pub source: String,
    pub status: CandidateStatus,
    pub promoted_to: Option<String>,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(default = "default_requires_evaluation")]
    pub requires_evaluation: bool,
    #[serde(default)]
    pub risk_level: RiskLevel,
    #[serde(default)]
    pub latest_evaluation_id: Option<String>,
    #[serde(default)]
    pub evaluation_passed: Option<bool>,
    pub created_at: String,
    pub updated_at: String,
}

impl EvolutionCandidate {
    pub fn kind_string(&self) -> &'static str {
        self.kind.as_str()
    }

    pub fn risk_level_string(&self) -> &'static str {
        self.risk_level.as_str()
    }

    pub fn status_string(&self) -> &'static str {
        self.status.as_str()
    }

    pub fn effective_fingerprint(&self) -> String {
        if self.fingerprint.is_empty() {
            candidate_fingerprint(
                self.kind,
                &self.title,
                &self.content,
                self.skill_name.as_deref(),
            )
        } else {
            self.fingerprint.clone()
        }
    }
}

#[derive(Debug, Clone)]
pub struct PromotionOutcome {
    pub candidate: EvolutionCandidate,
    pub target: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RollbackOutcome {
    pub candidate: EvolutionCandidate,
    pub target: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionRecord {
    pub candidate_id: String,
    pub target: String,
    pub snapshot_path: Option<String>,
    pub had_existing_target: bool,
    pub evaluation_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub rolled_back_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EvolutionStore {
    workspace: PathBuf,
}

struct NewCandidate {
    kind: CandidateKind,
    title: String,
    summary: String,
    content: String,
    skill_name: Option<String>,
    source_tape: Option<String>,
    source: String,
}

struct SnapshotCapture {
    had_existing_target: bool,
    snapshot_path: Option<PathBuf>,
}

impl CandidateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PromptRule => "prompt_rule",
            Self::Skill => "skill",
        }
    }
}

impl CandidateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Promoted => "promoted",
            Self::Rejected => "rejected",
            Self::RolledBack => "rolled_back",
        }
    }
}

impl RiskLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

impl SnapshotCapture {
    fn absent() -> Self {
        Self {
            had_existing_target: false,
            snapshot_path: None,
        }
    }

    fn existing(path: PathBuf) -> Self {
        Self {
            had_existing_target: true,
            snapshot_path: Some(path),
        }
    }
}

impl NewCandidate {
    fn rule(
        title: &str,
        summary: &str,
        content: &str,
        source_tape: Option<String>,
        source: &str,
    ) -> anyhow::Result<Self> {
        Self::build(
            CandidateKind::PromptRule,
            title,
            summary,
            content,
            None,
            source_tape,
            source,
        )
    }

    fn skill(
        skill_name: &str,
        title: &str,
        summary: &str,
        content: &str,
        source_tape: Option<String>,
        source: &str,
    ) -> anyhow::Result<Self> {
        Self::build(
            CandidateKind::Skill,
            title,
            summary,
            content,
            Some(skill_name.to_owned()),
            source_tape,
            source,
        )
    }

    fn build(
        kind: CandidateKind,
        title: &str,
        summary: &str,
        content: &str,
        skill_name: Option<String>,
        source_tape: Option<String>,
        source: &str,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            kind,
            title: require_text("title", title)?,
            summary: require_text("summary", summary)?,
            content: require_text("content", content)?,
            skill_name,
            source_tape,
            source: require_text("source", source)?,
        })
    }
}

impl EvolutionStore {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    pub fn capture_rule(
        &self,
        title: &str,
        summary: &str,
        content: &str,
        source_tape: Option<String>,
        source: &str,
    ) -> anyhow::Result<EvolutionCandidate> {
        let input = NewCandidate::rule(title, summary, content, source_tape, source)?;
        let candidate = self.new_candidate(input)?;
        self.write_candidate(&candidate)?;
        Ok(candidate)
    }

    pub fn capture_skill(
        &self,
        skill_name: &str,
        title: &str,
        summary: &str,
        content: &str,
        source_tape: Option<String>,
        source: &str,
    ) -> anyhow::Result<EvolutionCandidate> {
        validate_skill_name(skill_name)?;
        let input = NewCandidate::skill(skill_name, title, summary, content, source_tape, source)?;
        let candidate = self.new_candidate(input)?;
        self.write_candidate(&candidate)?;
        Ok(candidate)
    }

    pub fn list_candidates(&self) -> anyhow::Result<Vec<EvolutionCandidate>> {
        let candidates = read_candidates(&self.candidates_dir())?;
        Ok(sort_candidates(candidates))
    }

    pub fn distill_tape(
        &self,
        tapes_dir: &Path,
        tape_name: &str,
        persist: bool,
    ) -> anyhow::Result<DistillOutcome> {
        distill::distill_tape(self, tapes_dir, tape_name, persist)
    }

    pub fn read_candidate(&self, id: &str) -> anyhow::Result<EvolutionCandidate> {
        read_candidate_file(&self.candidate_path(id))
    }

    pub fn evaluate(&self, id: &str) -> anyhow::Result<EvaluationRun> {
        let candidate = self.read_candidate(id)?;
        ensure_evaluable(&candidate)?;
        let run = eval::evaluate_candidate(self, &candidate)?;
        self.write_evaluation(&run)?;
        self.write_candidate(&with_evaluation(candidate, &run))?;
        Ok(run)
    }

    pub fn reject(&self, id: &str) -> anyhow::Result<EvolutionCandidate> {
        let candidate = self.read_candidate(id)?;
        ensure_pending(&candidate)?;
        let rejected = with_status(candidate, CandidateStatus::Rejected, None);
        self.write_candidate(&rejected)?;
        Ok(rejected)
    }

    pub fn promote(&self, id: &str, force: bool) -> anyhow::Result<PromotionOutcome> {
        let candidate = self.read_candidate(id)?;
        ensure_pending(&candidate)?;
        ensure_promotion_gate(&candidate, force)?;
        let target = self.target_path(&candidate);
        let record = self.new_promotion_record(&candidate, &target)?;
        self.promote_candidate(&candidate, &target, force)?;
        self.write_promotion_record_file(&record)?;
        let promoted = with_promotion(candidate, target.clone());
        self.write_candidate(&promoted)?;
        Ok(PromotionOutcome {
            candidate: promoted,
            target,
        })
    }

    pub fn rollback(&self, id: &str) -> anyhow::Result<RollbackOutcome> {
        let candidate = self.read_candidate(id)?;
        ensure_promoted(&candidate)?;
        let record = self.read_promotion_record(id)?;
        let target = PathBuf::from(&record.target);
        self.restore_target(&record, &target)?;
        self.write_promotion_record_file(&with_rollback_record(record))?;
        let rolled_back = with_status(candidate, CandidateStatus::RolledBack, Some(target.clone()));
        self.write_candidate(&rolled_back)?;
        Ok(RollbackOutcome {
            candidate: rolled_back,
            target,
        })
    }

    pub fn load_prompt_rules(&self) -> anyhow::Result<String> {
        load_prompt_rules_for_workspace(&self.workspace)
    }

    fn promote_candidate(
        &self,
        candidate: &EvolutionCandidate,
        target: &Path,
        force: bool,
    ) -> anyhow::Result<()> {
        match candidate.kind {
            CandidateKind::PromptRule => self.append_prompt_rule(candidate, target),
            CandidateKind::Skill => self.write_skill(candidate, target, force),
        }
    }

    fn append_prompt_rule(
        &self,
        candidate: &EvolutionCandidate,
        path: &Path,
    ) -> anyhow::Result<()> {
        ensure_parent(path)?;
        let updated = append_rule_block(read_optional(path)?, candidate);
        fs::write(path, updated)?;
        Ok(())
    }

    fn write_skill(
        &self,
        candidate: &EvolutionCandidate,
        path: &Path,
        force: bool,
    ) -> anyhow::Result<()> {
        let skill_name = candidate.skill_name.as_deref().unwrap_or_default();
        ensure_writable_target(path, force)?;
        ensure_parent(path)?;
        fs::write(path, render_skill(candidate, skill_name))?;
        Ok(())
    }

    fn new_candidate(&self, input: NewCandidate) -> anyhow::Result<EvolutionCandidate> {
        let timestamp = now_rfc3339();
        let fingerprint = candidate_fingerprint(
            input.kind,
            &input.title,
            &input.content,
            input.skill_name.as_deref(),
        );
        let evidence_ids = input
            .source_tape
            .iter()
            .map(|tape| format!("tape:{tape}"))
            .collect();
        Ok(EvolutionCandidate {
            id: new_candidate_id(),
            kind: input.kind,
            title: input.title,
            summary: input.summary,
            content: input.content,
            skill_name: input.skill_name,
            source_tape: input.source_tape,
            source: input.source,
            status: CandidateStatus::Pending,
            promoted_to: None,
            fingerprint,
            evidence_ids,
            requires_evaluation: true,
            risk_level: risk_level_for(input.kind),
            latest_evaluation_id: None,
            evaluation_passed: None,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        })
    }

    fn write_candidate(&self, candidate: &EvolutionCandidate) -> anyhow::Result<()> {
        let path = self.candidate_path(&candidate.id);
        ensure_parent(&path)?;
        let body = serde_json::to_string_pretty(candidate)?;
        fs::write(path, format!("{body}\n"))?;
        Ok(())
    }

    fn candidate_path(&self, id: &str) -> PathBuf {
        self.candidates_dir().join(format!("{id}.json"))
    }

    fn evaluation_path(&self, id: &str) -> PathBuf {
        self.evaluations_dir().join(format!("{id}.json"))
    }

    fn promotion_record_path(&self, id: &str) -> PathBuf {
        self.promotions_dir().join(format!("{id}.json"))
    }

    fn snapshot_path(&self, id: &str) -> PathBuf {
        self.snapshots_dir().join(format!("{id}.bak"))
    }

    fn candidates_dir(&self) -> PathBuf {
        self.evolution_root().join(CANDIDATES_DIR)
    }

    fn evaluations_dir(&self) -> PathBuf {
        self.evolution_root().join(EVALUATIONS_DIR)
    }

    fn promotions_dir(&self) -> PathBuf {
        self.evolution_root().join(PROMOTIONS_DIR)
    }

    fn snapshots_dir(&self) -> PathBuf {
        self.evolution_root().join(SNAPSHOTS_DIR)
    }

    fn rules_path(&self) -> PathBuf {
        self.evolution_root().join(RULES_FILE)
    }

    fn skill_path(&self, skill_name: &str) -> PathBuf {
        self.workspace
            .join(".agents/skills")
            .join(skill_name)
            .join("SKILL.md")
    }

    fn evolution_root(&self) -> PathBuf {
        self.workspace.join(EVOLUTION_DIR)
    }

    fn target_path(&self, candidate: &EvolutionCandidate) -> PathBuf {
        match candidate.kind {
            CandidateKind::PromptRule => self.rules_path(),
            CandidateKind::Skill => self.skill_path(candidate.skill_name.as_deref().unwrap_or("")),
        }
    }

    fn write_evaluation(&self, run: &EvaluationRun) -> anyhow::Result<()> {
        let path = self.evaluation_path(&run.id);
        write_json_file(&path, run)
    }

    fn read_promotion_record(&self, id: &str) -> anyhow::Result<PromotionRecord> {
        let body = fs::read_to_string(self.promotion_record_path(id))?;
        Ok(serde_json::from_str(&body)?)
    }

    fn write_promotion_record_file(&self, record: &PromotionRecord) -> anyhow::Result<()> {
        write_json_file(&self.promotion_record_path(&record.candidate_id), record)
    }

    fn new_promotion_record(
        &self,
        candidate: &EvolutionCandidate,
        target: &Path,
    ) -> anyhow::Result<PromotionRecord> {
        let timestamp = now_rfc3339();
        let snapshot = self.capture_snapshot(&candidate.id, target)?;
        Ok(PromotionRecord {
            candidate_id: candidate.id.clone(),
            target: display_string(target.to_path_buf()),
            snapshot_path: snapshot.snapshot_path.map(display_string),
            had_existing_target: snapshot.had_existing_target,
            evaluation_id: candidate.latest_evaluation_id.clone(),
            created_at: timestamp.clone(),
            updated_at: timestamp,
            rolled_back_at: None,
        })
    }

    fn capture_snapshot(&self, id: &str, target: &Path) -> anyhow::Result<SnapshotCapture> {
        if !target.exists() {
            return Ok(SnapshotCapture::absent());
        }
        let path = self.snapshot_path(id);
        ensure_parent(&path)?;
        fs::copy(target, &path)?;
        Ok(SnapshotCapture::existing(path))
    }

    fn restore_target(&self, record: &PromotionRecord, target: &Path) -> anyhow::Result<()> {
        match &record.snapshot_path {
            Some(path) => self.restore_snapshot(path, target),
            None => remove_target_if_exists(target),
        }
    }

    fn restore_snapshot(&self, snapshot: &str, target: &Path) -> anyhow::Result<()> {
        ensure_parent(target)?;
        fs::copy(snapshot, target)?;
        Ok(())
    }
}

pub fn load_prompt_rules_for_workspace(workspace: &Path) -> anyhow::Result<String> {
    load_prompt_rules_from_paths(
        &global_rules_path(),
        &EvolutionStore::new(workspace).rules_path(),
    )
}

fn global_rules_path() -> PathBuf {
    eli_home().join("evolution").join(RULES_FILE)
}

fn load_prompt_rules_from_paths(global: &Path, local: &Path) -> anyhow::Result<String> {
    Ok(join_rule_sources([
        read_optional(global)?,
        read_optional(local)?,
    ]))
}

fn join_rule_sources(parts: [String; 2]) -> String {
    parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn sort_candidates(mut candidates: Vec<EvolutionCandidate>) -> Vec<EvolutionCandidate> {
    candidates.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    candidates
}

fn read_candidates(dir: &Path) -> anyhow::Result<Vec<EvolutionCandidate>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    fs::read_dir(dir)?
        .map(read_dir_entry_candidate)
        .collect::<anyhow::Result<Vec<_>>>()
}

fn read_dir_entry_candidate(
    entry: std::io::Result<fs::DirEntry>,
) -> anyhow::Result<EvolutionCandidate> {
    let path = entry?.path();
    read_candidate_file(&path)
}

fn read_candidate_file(path: &Path) -> anyhow::Result<EvolutionCandidate> {
    let body = fs::read_to_string(path)?;
    Ok(hydrate_candidate(serde_json::from_str(&body)?))
}

fn read_optional(path: &Path) -> anyhow::Result<String> {
    if !path.is_file() {
        return Ok(String::new());
    }
    Ok(fs::read_to_string(path)?)
}

fn append_rule_block(existing: String, candidate: &EvolutionCandidate) -> String {
    let block = rule_block(candidate);
    if trimmed(&existing).contains(trimmed(&block)) {
        return ensure_trailing_newline(existing);
    }
    [ensure_trailing_newline(existing), block]
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
        + "\n"
}

fn rule_block(candidate: &EvolutionCandidate) -> String {
    format!(
        "## {}\n{}",
        candidate.title.trim(),
        candidate.content.trim()
    )
}

fn render_skill(candidate: &EvolutionCandidate, skill_name: &str) -> String {
    format!(
        "---\nname: {skill_name}\ndescription: {}\n---\n{}{}\n",
        candidate.summary.trim(),
        render_skill_title(candidate),
        candidate.content.trim()
    )
}

fn render_skill_title(candidate: &EvolutionCandidate) -> String {
    let title = candidate.title.trim();
    if title.is_empty() {
        String::new()
    } else {
        format!("# {title}\n\n")
    }
}

fn with_status(
    mut candidate: EvolutionCandidate,
    status: CandidateStatus,
    promoted_to: Option<PathBuf>,
) -> EvolutionCandidate {
    candidate.status = status;
    candidate.promoted_to = promoted_to.map(display_string);
    candidate.updated_at = now_rfc3339();
    candidate
}

fn with_evaluation(mut candidate: EvolutionCandidate, run: &EvaluationRun) -> EvolutionCandidate {
    candidate.latest_evaluation_id = Some(run.id.clone());
    candidate.evaluation_passed = Some(run.passed);
    candidate.updated_at = now_rfc3339();
    candidate
}

fn with_promotion(mut candidate: EvolutionCandidate, target: PathBuf) -> EvolutionCandidate {
    candidate.status = CandidateStatus::Promoted;
    candidate.promoted_to = Some(display_string(target));
    candidate.updated_at = now_rfc3339();
    candidate
}

fn with_rollback_record(mut record: PromotionRecord) -> PromotionRecord {
    let timestamp = now_rfc3339();
    record.rolled_back_at = Some(timestamp.clone());
    record.updated_at = timestamp;
    record
}

fn display_string(path: PathBuf) -> String {
    path.display().to_string()
}

fn hydrate_candidate(mut candidate: EvolutionCandidate) -> EvolutionCandidate {
    if candidate.fingerprint.is_empty() {
        candidate.fingerprint = candidate.effective_fingerprint();
        candidate.risk_level = risk_level_for(candidate.kind);
    }
    if candidate.evidence_ids.is_empty() {
        candidate.evidence_ids = candidate
            .source_tape
            .iter()
            .map(|tape| format!("tape:{tape}"))
            .collect();
    }
    candidate
}

fn ensure_pending(candidate: &EvolutionCandidate) -> anyhow::Result<()> {
    if candidate.status == CandidateStatus::Pending {
        return Ok(());
    }
    anyhow::bail!("candidate '{}' is not pending", candidate.id)
}

fn ensure_evaluable(candidate: &EvolutionCandidate) -> anyhow::Result<()> {
    if candidate.status == CandidateStatus::Pending {
        return Ok(());
    }
    anyhow::bail!(
        "candidate '{}' cannot be evaluated from status {}",
        candidate.id,
        candidate.status_string()
    )
}

fn ensure_promoted(candidate: &EvolutionCandidate) -> anyhow::Result<()> {
    if candidate.status == CandidateStatus::Promoted {
        return Ok(());
    }
    anyhow::bail!("candidate '{}' is not promoted", candidate.id)
}

fn ensure_promotion_gate(candidate: &EvolutionCandidate, force: bool) -> anyhow::Result<()> {
    if force || !candidate.requires_evaluation || candidate.evaluation_passed == Some(true) {
        return Ok(());
    }
    anyhow::bail!(
        "candidate '{}' requires a passing evaluation before promotion",
        candidate.id
    )
}

fn ensure_writable_target(path: &Path, force: bool) -> anyhow::Result<()> {
    if !path.exists() || force {
        return Ok(());
    }
    anyhow::bail!("target already exists: {}", path.display())
}

fn ensure_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn validate_skill_name(skill_name: &str) -> anyhow::Result<()> {
    if is_valid_skill_name(skill_name) {
        return Ok(());
    }
    anyhow::bail!("invalid skill name: {skill_name}")
}

fn require_text(field: &str, value: &str) -> anyhow::Result<String> {
    let text = value.trim();
    if !text.is_empty() {
        return Ok(text.to_owned());
    }
    anyhow::bail!("{field} must not be empty")
}

fn write_json_file(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    ensure_parent(path)?;
    let body = serde_json::to_string_pretty(value)?;
    fs::write(path, format!("{body}\n"))?;
    Ok(())
}

fn remove_target_if_exists(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn default_requires_evaluation() -> bool {
    true
}

fn risk_level_for(kind: CandidateKind) -> RiskLevel {
    match kind {
        CandidateKind::PromptRule => RiskLevel::Medium,
        CandidateKind::Skill => RiskLevel::High,
    }
}

fn candidate_fingerprint(
    kind: CandidateKind,
    title: &str,
    content: &str,
    skill_name: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_str());
    hasher.update("\n");
    hasher.update(skill_name.unwrap_or(""));
    hasher.update("\n");
    hasher.update(trimmed(title));
    hasher.update("\n");
    hasher.update(trimmed(content));
    format!("{:x}", hasher.finalize())
}

fn new_candidate_id() -> String {
    UuidLike::new().to_string()
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn ensure_trailing_newline(value: String) -> String {
    if value.is_empty() || value.ends_with('\n') {
        return value;
    }
    format!("{value}\n")
}

fn trimmed(value: &str) -> &str {
    value.trim()
}

struct UuidLike(uuid::Uuid);

impl UuidLike {
    fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl std::fmt::Display for UuidLike {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let compact = self.0.simple().to_string();
        f.write_str(&compact[..12])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tmp: &tempfile::TempDir) -> EvolutionStore {
        EvolutionStore::new(tmp.path())
    }

    fn skill_candidate(store: &EvolutionStore) -> EvolutionCandidate {
        store
            .capture_skill(
                "deploy-docs",
                "Deploy docs",
                "Deploy docs to the site",
                "## When to Use\nWhen the docs need publishing.\n\n## Procedure\n1. Run deploy.\n",
                Some("workspace__local".to_owned()),
                "test",
            )
            .unwrap()
    }

    #[test]
    fn test_capture_rule_candidate_writes_json() {
        let tmp = tempfile::tempdir().unwrap();
        let candidate = store(&tmp)
            .capture_rule(
                "Keep updates terse",
                "Prefer terse progress updates",
                "- Keep status updates to two sentences.\n",
                Some("workspace__local".to_owned()),
                "test",
            )
            .unwrap();
        let saved = store(&tmp).read_candidate(&candidate.id).unwrap();
        assert_eq!(saved.title, "Keep updates terse");
        assert_eq!(saved.status, CandidateStatus::Pending);
    }

    #[test]
    fn test_promote_rule_appends_rules_file() {
        let tmp = tempfile::tempdir().unwrap();
        let candidate = store(&tmp)
            .capture_rule(
                "Prefer evidence",
                "Cite files",
                "- Cite file paths when possible.",
                None,
                "test",
            )
            .unwrap();
        store(&tmp).evaluate(&candidate.id).unwrap();
        let outcome = store(&tmp).promote(&candidate.id, false).unwrap();
        let rules = fs::read_to_string(outcome.target).unwrap();
        assert!(rules.contains("Prefer evidence"));
        assert!(rules.contains("Cite file paths when possible."));
    }

    #[test]
    fn test_promote_skill_writes_skill_file() {
        let tmp = tempfile::tempdir().unwrap();
        let candidate = skill_candidate(&store(&tmp));
        store(&tmp).evaluate(&candidate.id).unwrap();
        let outcome = store(&tmp).promote(&candidate.id, false).unwrap();
        let skill = fs::read_to_string(outcome.target).unwrap();
        assert!(skill.contains("name: deploy-docs"));
        assert!(skill.contains("description: Deploy docs to the site"));
    }

    #[test]
    fn test_reject_updates_status() {
        let tmp = tempfile::tempdir().unwrap();
        let candidate = skill_candidate(&store(&tmp));
        let rejected = store(&tmp).reject(&candidate.id).unwrap();
        assert_eq!(rejected.status, CandidateStatus::Rejected);
    }

    #[test]
    fn test_load_prompt_rules_joins_global_and_local() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let global = home.path().join("rules.md");
        let local = tmp.path().join("rules.md");
        fs::write(&global, "## Global\n- One").unwrap();
        fs::write(&local, "## Local\n- Two").unwrap();
        let rules = load_prompt_rules_from_paths(&global, &local).unwrap();
        assert!(rules.contains("## Global"));
        assert!(rules.contains("## Local"));
    }

    #[test]
    fn test_evaluate_rule_updates_candidate_with_result() {
        let tmp = tempfile::tempdir().unwrap();
        let candidate = store(&tmp)
            .capture_rule(
                "Prefer evidence",
                "Cite files",
                "- Cite file paths.",
                None,
                "test",
            )
            .unwrap();
        let run = store(&tmp).evaluate(&candidate.id).unwrap();
        let saved = store(&tmp).read_candidate(&candidate.id).unwrap();
        assert!(run.passed);
        assert_eq!(saved.latest_evaluation_id, Some(run.id));
        assert_eq!(saved.evaluation_passed, Some(true));
    }

    #[test]
    fn test_promote_requires_passing_evaluation_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        let candidate = store(&tmp)
            .capture_rule(
                "Prefer evidence",
                "Cite files",
                "- Cite file paths.",
                None,
                "test",
            )
            .unwrap();
        let error = store(&tmp).promote(&candidate.id, false).unwrap_err();
        assert!(error.to_string().contains("requires a passing evaluation"));
    }

    #[test]
    fn test_rollback_restores_previous_rule_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let rules_dir = tmp.path().join(".agents/evolution");
        fs::create_dir_all(&rules_dir).unwrap();
        fs::write(rules_dir.join("rules.md"), "## Existing\n- Keep this.\n").unwrap();
        let candidate = store(&tmp)
            .capture_rule(
                "Prefer evidence",
                "Cite files",
                "- Cite file paths.",
                None,
                "test",
            )
            .unwrap();
        store(&tmp).evaluate(&candidate.id).unwrap();
        store(&tmp).promote(&candidate.id, false).unwrap();
        let outcome = store(&tmp).rollback(&candidate.id).unwrap();
        let restored = fs::read_to_string(outcome.target).unwrap();
        assert_eq!(restored, "## Existing\n- Keep this.\n");
        assert_eq!(outcome.candidate.status, CandidateStatus::RolledBack);
    }
}
