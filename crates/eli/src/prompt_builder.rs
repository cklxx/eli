//! Sectioned system prompt builder with mode-based composition and size management.
//!
//! Replaces the monolithic `build_system_prompt()` with composable section builders,
//! supporting Full/Minimal/None modes and per-section truncation.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use crate::builtin::settings::AgentSettings;
use crate::skills::{SkillMetadata, render_skills_prompt};

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

        // Render each section.
        let mut rendered: Vec<(u8, String)> = Vec::with_capacity(self.sections.len());
        for sec in &self.sections {
            let content = self.render_section(
                sec,
                settings,
                prompt_text,
                allowed_skills,
                expanded_skills,
                workspace,
            );
            if content.is_empty() {
                continue;
            }
            // Apply per-section limit.
            let content = if let Some(max) = sec.max_chars {
                truncate_chars(&content, max)
            } else {
                content
            };
            rendered.push((sec.truncation_priority, content));
        }

        // Check total size.
        let total: usize = rendered.iter().map(|(_, s)| s.len()).sum::<usize>()
            + rendered.len().saturating_sub(1) * 2; // "\n\n" separators

        if total <= self.hard_cap {
            return rendered
                .into_iter()
                .map(|(_, s)| s)
                .collect::<Vec<_>>()
                .join("\n\n");
        }

        // Truncate sections by priority (lowest first) until under the cap.
        // Sort indices by truncation_priority ascending.
        let mut indices: Vec<usize> = (0..rendered.len()).collect();
        indices.sort_by_key(|&i| rendered[i].0);

        let mut current_total = total;
        for &idx in &indices {
            if current_total <= self.hard_cap {
                break;
            }
            // Skip identity (priority 255) — never truncate.
            if rendered[idx].0 == 255 {
                continue;
            }
            let section_len = rendered[idx].1.len();
            let need_to_remove = current_total - self.hard_cap;
            if section_len <= need_to_remove {
                // Drop entire section.
                current_total -= section_len + 2; // +2 for the "\n\n" separator
                rendered[idx].1.clear();
            } else {
                // Truncate section.
                let new_len = section_len - need_to_remove;
                rendered[idx].1 = truncate_chars(&rendered[idx].1, new_len);
                current_total = self.hard_cap;
            }
        }

        rendered
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
                let filtered: Vec<SkillMetadata> = if let Some(allowed) = allowed_skills {
                    skills
                        .into_iter()
                        .filter(|s| allowed.contains(&s.name.to_lowercase()))
                        .collect()
                } else {
                    skills
                };
                // Merge hint-expanded skills with auto-activated skills.
                let hint_re = regex::Regex::new(r"\$([A-Za-z0-9_.\-]+)").unwrap();
                let mut all_expanded = expanded_skills.clone();
                for cap in hint_re.captures_iter(prompt_text) {
                    if let Some(m) = cap.get(1) {
                        all_expanded.insert(m.as_str().to_owned());
                    }
                }
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

fn default_system_prompt() -> &'static str {
    "You are Eli, a helpful AI coding assistant.\n\
     \n\
     Output quality (priority: Clear > Coherent > Concise > Concrete): \
     Lead with result first, key evidence second, supporting detail only on demand. \
     Avoid emojis unless the user explicitly requests them.\n\
     \n\
     Execution: Always execute first and exhaust safe deterministic attempts before asking questions. \
     If intent is unclear, inspect context first (tape.search, then workspace files). \
     For explicit low-risk read-only asks (view/check/list/inspect files, branches, project state), \
     execute directly with tools and report findings — do not ask for reconfirmation. \
     Ask a question only when requirements are genuinely missing or contradictory after all viable attempts fail. \
     Treat explicit delegation signals (\"you decide\", \"anything works\", \"use your judgment\") \
     as authorization for low-risk reversible actions: choose a sensible default, execute, and report.\n\
     \n\
     Tools: Use tools to accomplish tasks rather than explaining how to do them. \
     When a tool fails, analyze the error and try an alternative approach before reporting failure. \
     Use web_fetch when you have a URL; use other tools for local operations. \
     Use /tmp as the default location for temporary files unless the user specifies another path.\n\
     \n\
     Response: Reply directly with your response text. \
     Your text output will be delivered to the user automatically — the framework handles channel routing. \
     Do NOT attempt to call channel-specific send functions or emit XML tool-call markup in your text output.\n\
     \n\
     Context: When context grows large, prefer concise responses. \
     You may use tape.info to check token usage and tape.handoff to trim older history. \
     Do not repeat information already visible in the conversation."
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
