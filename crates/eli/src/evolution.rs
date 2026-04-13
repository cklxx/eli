//! Governed self-evolution storage for prompt rules and skill candidates.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::builtin::config::eli_home;
use crate::skills::is_valid_skill_name;

const EVOLUTION_DIR: &str = ".agents/evolution";
const CANDIDATES_DIR: &str = "candidates";
const RULES_FILE: &str = "rules.md";

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
    pub created_at: String,
    pub updated_at: String,
}

impl EvolutionCandidate {
    pub fn kind_string(&self) -> &'static str {
        self.kind.as_str()
    }

    pub fn status_string(&self) -> &'static str {
        self.status.as_str()
    }
}

#[derive(Debug, Clone)]
pub struct PromotionOutcome {
    pub candidate: EvolutionCandidate,
    pub target: PathBuf,
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

    pub fn read_candidate(&self, id: &str) -> anyhow::Result<EvolutionCandidate> {
        read_candidate_file(&self.candidate_path(id))
    }

    pub fn reject(&self, id: &str) -> anyhow::Result<EvolutionCandidate> {
        let candidate = self.read_candidate(id)?;
        let rejected = with_status(candidate, CandidateStatus::Rejected, None);
        self.write_candidate(&rejected)?;
        Ok(rejected)
    }

    pub fn promote(&self, id: &str, force: bool) -> anyhow::Result<PromotionOutcome> {
        let candidate = self.read_candidate(id)?;
        ensure_pending(&candidate)?;
        let target = self.promote_candidate(&candidate, force)?;
        let promoted = with_status(candidate, CandidateStatus::Promoted, Some(target.clone()));
        self.write_candidate(&promoted)?;
        Ok(PromotionOutcome {
            candidate: promoted,
            target,
        })
    }

    pub fn load_prompt_rules(&self) -> anyhow::Result<String> {
        load_prompt_rules_for_workspace(&self.workspace)
    }

    fn promote_candidate(
        &self,
        candidate: &EvolutionCandidate,
        force: bool,
    ) -> anyhow::Result<PathBuf> {
        match candidate.kind {
            CandidateKind::PromptRule => self.append_prompt_rule(candidate),
            CandidateKind::Skill => self.write_skill(candidate, force),
        }
    }

    fn append_prompt_rule(&self, candidate: &EvolutionCandidate) -> anyhow::Result<PathBuf> {
        let path = self.rules_path();
        ensure_parent(&path)?;
        let updated = append_rule_block(read_optional(&path)?, candidate);
        fs::write(&path, updated)?;
        Ok(path)
    }

    fn write_skill(&self, candidate: &EvolutionCandidate, force: bool) -> anyhow::Result<PathBuf> {
        let skill_name = candidate.skill_name.as_deref().unwrap_or_default();
        let path = self.skill_path(skill_name);
        ensure_writable_target(&path, force)?;
        ensure_parent(&path)?;
        fs::write(&path, render_skill(candidate, skill_name))?;
        Ok(path)
    }

    fn new_candidate(&self, input: NewCandidate) -> anyhow::Result<EvolutionCandidate> {
        let timestamp = now_rfc3339();
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

    fn candidates_dir(&self) -> PathBuf {
        self.evolution_root().join(CANDIDATES_DIR)
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
    Ok(serde_json::from_str(&body)?)
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

fn display_string(path: PathBuf) -> String {
    path.display().to_string()
}

fn ensure_pending(candidate: &EvolutionCandidate) -> anyhow::Result<()> {
    if candidate.status == CandidateStatus::Pending {
        return Ok(());
    }
    anyhow::bail!("candidate '{}' is not pending", candidate.id)
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
        let outcome = store(&tmp).promote(&candidate.id, false).unwrap();
        let rules = fs::read_to_string(outcome.target).unwrap();
        assert!(rules.contains("Prefer evidence"));
        assert!(rules.contains("Cite file paths when possible."));
    }

    #[test]
    fn test_promote_skill_writes_skill_file() {
        let tmp = tempfile::tempdir().unwrap();
        let candidate = skill_candidate(&store(&tmp));
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
}
