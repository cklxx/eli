//! Sectioned system prompt builder with mode-based composition and size management.
//!
//! Replaces the monolithic `build_system_prompt()` with composable section builders,
//! supporting Full/Minimal/None modes and per-section truncation.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use std::sync::LazyLock;

use crate::builtin::settings::AgentSettings;
use crate::skills::{SkillMetadata, render_skills_prompt};

static HINT_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\$([A-Za-z0-9_.\-]+)").expect("SAFETY: static regex"));

/// Default hard cap for the total system prompt size (chars).
const DEFAULT_HARD_CAP: usize = 32_000;

/// Which sections to include in the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    /// All sections: identity, tools, skills, runtime.
    Full,
    /// Lightweight: identity + runtime only.
    Minimal,
    /// Empty system prompt.
    None,
}

/// A named section of the system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionKind {
    /// Base system prompt from SOUL.md or built-in default.
    Identity,
    /// `<available_skills>` block listing discovered skills.
    Skills,
    /// Runtime context (date, workspace, etc.).
    Runtime,
}

/// Configuration for a single section.
#[derive(Debug, Clone)]
pub struct SectionConfig {
    pub kind: SectionKind,
    /// Optional per-section character limit.
    pub max_chars: Option<usize>,
    /// Lower values are truncated first when the total exceeds the hard cap.
    /// Identity (255) is never truncated.
    pub truncation_priority: u8,
}

/// Builder that assembles a system prompt from composable sections.
pub struct PromptBuilder {
    mode: PromptMode,
    hard_cap: usize,
    sections: Vec<SectionConfig>,
}

impl PromptBuilder {
    /// Create a builder with sensible default sections for the given mode.
    pub fn new(mode: PromptMode) -> Self {
        let sections = match mode {
            PromptMode::Full => vec![
                SectionConfig {
                    kind: SectionKind::Identity,
                    max_chars: None,
                    truncation_priority: 255,
                },
                SectionConfig {
                    kind: SectionKind::Skills,
                    max_chars: Some(8_000),
                    truncation_priority: 10,
                },
                SectionConfig {
                    kind: SectionKind::Runtime,
                    max_chars: Some(1_000),
                    truncation_priority: 200,
                },
            ],
            PromptMode::Minimal => vec![
                SectionConfig {
                    kind: SectionKind::Identity,
                    max_chars: None,
                    truncation_priority: 255,
                },
                SectionConfig {
                    kind: SectionKind::Runtime,
                    max_chars: Some(1_000),
                    truncation_priority: 200,
                },
            ],
            PromptMode::None => vec![],
        };

        Self {
            mode,
            hard_cap: DEFAULT_HARD_CAP,
            sections,
        }
    }

    /// Override the total character hard cap.
    pub fn with_hard_cap(mut self, chars: usize) -> Self {
        self.hard_cap = chars;
        self
    }

    /// Build the final system prompt string.
    pub fn build(
        &self,
        settings: &AgentSettings,
        prompt_text: &str,
        _state: &HashMap<String, Value>,
        allowed_skills: Option<&HashSet<String>>,
        expanded_skills: &HashSet<String>,
        workspace: &Path,
    ) -> String {
        if self.mode == PromptMode::None {
            return String::new();
        }

        let mut rendered = self.render_all_sections(
            settings,
            prompt_text,
            allowed_skills,
            expanded_skills,
            workspace,
        );

        let total = Self::total_size(&rendered);
        if total > self.hard_cap {
            Self::truncate_to_budget(&mut rendered, total, self.hard_cap);
        }

        Self::join_sections(rendered)
    }

    fn render_all_sections(
        &self,
        settings: &AgentSettings,
        prompt_text: &str,
        allowed_skills: Option<&HashSet<String>>,
        expanded_skills: &HashSet<String>,
        workspace: &Path,
    ) -> Vec<(u8, String)> {
        self.sections
            .iter()
            .filter_map(|sec| {
                let content = self.render_section(
                    sec,
                    settings,
                    prompt_text,
                    allowed_skills,
                    expanded_skills,
                    workspace,
                );
                if content.is_empty() {
                    return None;
                }
                let content = match sec.max_chars {
                    Some(max) => truncate_chars(&content, max),
                    None => content,
                };
                Some((sec.truncation_priority, content))
            })
            .collect()
    }

    fn total_size(sections: &[(u8, String)]) -> usize {
        sections.iter().map(|(_, s)| s.len()).sum::<usize>() + sections.len().saturating_sub(1) * 2
    }

    fn truncate_to_budget(
        sections: &mut [(u8, String)],
        mut current_total: usize,
        hard_cap: usize,
    ) {
        let mut indices: Vec<usize> = (0..sections.len()).collect();
        indices.sort_by_key(|&i| sections[i].0);

        for &idx in &indices {
            if current_total <= hard_cap {
                break;
            }
            if sections[idx].0 == 255 {
                continue;
            }
            let section_len = sections[idx].1.len();
            let need_to_remove = current_total - hard_cap;
            if section_len <= need_to_remove {
                current_total -= section_len + 2;
                sections[idx].1.clear();
            } else {
                let new_len = section_len - need_to_remove;
                sections[idx].1 = truncate_chars(&sections[idx].1, new_len);
                current_total = hard_cap;
            }
        }
    }

    fn merge_expanded_skills(base: &HashSet<String>, prompt_text: &str) -> HashSet<String> {
        let mut merged = base.clone();
        for cap in HINT_RE.captures_iter(prompt_text) {
            if let Some(m) = cap.get(1) {
                merged.insert(m.as_str().to_owned());
            }
        }
        merged
    }

    fn join_sections(sections: Vec<(u8, String)>) -> String {
        sections
            .into_iter()
            .map(|(_, s)| s)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn render_section(
        &self,
        sec: &SectionConfig,
        settings: &AgentSettings,
        prompt_text: &str,
        allowed_skills: Option<&HashSet<String>>,
        expanded_skills: &HashSet<String>,
        workspace: &Path,
    ) -> String {
        match sec.kind {
            SectionKind::Identity => load_system_prompt_base(settings, workspace),
            SectionKind::Skills => {
                let skills = crate::skills::discover_skills(workspace);
                let filtered: Vec<SkillMetadata> = match allowed_skills {
                    Some(allowed) => skills
                        .into_iter()
                        .filter(|s| allowed.contains(&s.name.to_lowercase()))
                        .collect(),
                    None => skills,
                };
                let all_expanded = Self::merge_expanded_skills(expanded_skills, prompt_text);
                render_skills_prompt(&filtered, &all_expanded)
            }
            SectionKind::Runtime => {
                let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
                format!(
                    "<runtime>\nDate: {now}\nWorkspace: {}\n</runtime>",
                    workspace.display()
                )
            }
        }
    }
}

/// Load persona and system prompt with precedence:
/// 1. `.agents/SOUL.md` (project-level persona)
/// 2. `~/.eli/SOUL.md` (global user-level persona)
/// 3. Built-in default
///
/// The SOUL.md file defines the agent's personality and behavioral rules.
fn load_system_prompt_base(settings: &AgentSettings, workspace: &Path) -> String {
    // Try SOUL.md first (persona file)
    for path in [
        workspace.join(".agents").join("SOUL.md"),
        settings.home.join("SOUL.md"),
    ] {
        if path.is_file()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }

    default_system_prompt().to_owned()
}

/// Fallback system prompt used only when no SOUL.md is found in the project
/// (`.agents/SOUL.md`) or user home (`~/.eli/SOUL.md`). In practice most
/// deployments have a SOUL.md, so edits here rarely take effect — update the
/// SOUL.md file instead.
fn default_system_prompt() -> &'static str {
    "You are Eli, a helpful AI coding assistant.\n\
     \n\
     Lead with the result, then key evidence. Detail only on demand. No emojis unless asked. \
     Do not repeat information already visible in the conversation.\n\
     \n\
     Execute first — exhaust safe attempts before asking questions. \
     If intent is unclear, check context (tape.search, workspace files). \
     For low-risk read-only asks (view/check/list/inspect files, branches, project state), \
     execute directly and report — do not ask for reconfirmation. \
     Ask only when requirements are genuinely missing after viable attempts fail. \
     \"You decide\" / \"anything works\" = authorization for reversible actions.\n\
     \n\
     When you receive a non-trivial request, reply with a brief message (1-2 sentences) \
     explaining what you plan to do before executing. Skip for simple questions.\n\
     \n\
     Never speculate about code you haven't read — read first, then speak. \
     Confident about an improvement? Do it. Uncertain? Don't touch it.\n\
     \n\
     Use tools to do the work, not to explain how. \
     Use web_fetch for URLs; other tools for local operations. \
     Use /tmp for temporary files unless the user specifies another path. \
     Tool fails? Read the error, try an alternative, then report. \
     Your text output goes to the user automatically — don't call send functions or emit markup. \
     Context growing large? Use tape.info to check, then tape.handoff to trim."
}

/// Truncate a string to at most `max_chars` characters, appending "..." if truncated.
fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        return s.to_owned();
    }
    let end = max_chars.saturating_sub(3);
    let mut result = String::with_capacity(max_chars);
    for (i, ch) in s.char_indices() {
        if i >= end {
            break;
        }
        result.push(ch);
    }
    result.push_str("...");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_chars() {
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("hello world", 8), "hello...");
    }

    #[test]
    fn test_none_mode_returns_empty() {
        let builder = PromptBuilder::new(PromptMode::None);
        let result = builder.build(
            &AgentSettings::from_env(),
            "test",
            &HashMap::new(),
            None,
            &HashSet::new(),
            Path::new("/tmp"),
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_minimal_mode_has_identity_and_runtime() {
        let builder = PromptBuilder::new(PromptMode::Minimal);
        let result = builder.build(
            &AgentSettings::from_env(),
            "test",
            &HashMap::new(),
            None,
            &HashSet::new(),
            Path::new("/tmp"),
        );
        assert!(result.contains("Eli"));
        assert!(result.contains("<runtime>"));
        // Minimal mode should NOT have tools or skills
        assert!(!result.contains("<available_tools>"));
    }
}
