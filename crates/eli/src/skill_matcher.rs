//! Multi-signal skill matching engine with weighted scoring, exclusive groups, and cooldown.
//!
//! Replaces the simple `$hint` regex matching with a richer activation model:
//! - Intent patterns (regex, weight 0.6)
//! - Tool signals (recent tool usage, weight 0.25)
//! - Context keywords (keyword presence, weight 0.15)
//! - Exclusive groups (only highest per group activates)
//! - Per-session cooldown tracking

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

use regex::Regex;

use crate::skills::SkillMetadata;

// ---------------------------------------------------------------------------
// Signal weights
// ---------------------------------------------------------------------------

const WEIGHT_INTENT: f64 = 0.6;
const WEIGHT_TOOL: f64 = 0.25;
const WEIGHT_KEYWORD: f64 = 0.15;

const DEFAULT_CONFIDENCE_THRESHOLD: f64 = 0.3;
const DEFAULT_PRIORITY: u32 = 100;
const DEFAULT_TOKEN_BUDGET: usize = 16_000;

// ---------------------------------------------------------------------------
// Trigger configuration (parsed from SKILL.md YAML frontmatter)
// ---------------------------------------------------------------------------

/// Extended skill trigger configuration.
#[derive(Debug, Clone, Default)]
pub struct SkillTriggers {
    /// Regex patterns matched against user input.
    pub intent_patterns: Vec<String>,
    /// Tool names that signal relevance of this skill.
    pub tool_signals: Vec<String>,
    /// Keywords whose presence in the prompt boosts the score.
    pub context_keywords: Vec<String>,
    /// Minimum composite score to activate (default 0.3).
    pub confidence_threshold: f64,
    /// Tiebreaker within exclusive groups (higher wins, default 100).
    pub priority: u32,
    /// If set, only the highest-scoring skill in the same group activates.
    pub exclusive_group: Option<String>,
    /// Per-session cooldown in seconds before re-activation.
    pub cooldown_secs: Option<u64>,
}

// ---------------------------------------------------------------------------
// Match context & result
// ---------------------------------------------------------------------------

/// Context provided to the matcher for scoring.
pub struct MatchContext<'a> {
    /// The user's current input text.
    pub task_input: &'a str,
    /// Tool names used in recent turns.
    pub recent_tools: &'a [String],
    /// Session identifier for cooldown tracking.
    pub session_id: &'a str,
}

/// Detailed score breakdown for diagnostics.
#[derive(Debug, Clone, Default)]
pub struct MatchSignals {
    pub intent_score: f64,
    pub tool_score: f64,
    pub keyword_score: f64,
}

/// Result of matching one skill.
#[derive(Debug, Clone)]
pub struct SkillMatch {
    pub skill_name: String,
    pub score: f64,
    pub signals: MatchSignals,
    pub priority: u32,
    pub exclusive_group: Option<String>,
}

// ---------------------------------------------------------------------------
// Cooldown tracker
// ---------------------------------------------------------------------------

/// Per-session, per-skill cooldown tracker.
pub struct CooldownTracker {
    /// session_id → (skill_name → last_activated_at)
    entries: Mutex<HashMap<String, HashMap<String, Instant>>>,
}

impl CooldownTracker {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Check whether the skill is still within its cooldown period.
    pub fn is_cooled_down(&self, session_id: &str, skill_name: &str, cooldown_secs: u64) -> bool {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = entries.get(session_id)
            && let Some(last) = session.get(skill_name)
        {
            return last.elapsed().as_secs() >= cooldown_secs;
        }
        true // No record → not cooling down
    }

    /// Record that a skill was just activated.
    pub fn record_activation(&self, session_id: &str, skill_name: &str) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .entry(session_id.to_owned())
            .or_default()
            .insert(skill_name.to_owned(), Instant::now());
    }
}

impl Default for CooldownTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Skill matcher engine
// ---------------------------------------------------------------------------

/// Multi-signal skill matching engine.
pub struct SkillMatcher {
    cooldowns: CooldownTracker,
    token_budget: usize,
}

impl Default for SkillMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillMatcher {
    pub fn new() -> Self {
        Self {
            cooldowns: CooldownTracker::new(),
            token_budget: DEFAULT_TOKEN_BUDGET,
        }
    }

    pub fn with_token_budget(mut self, budget: usize) -> Self {
        self.token_budget = budget;
        self
    }

    /// Parse extended triggers from a skill's frontmatter metadata map.
    ///
    /// The existing `parse_yaml_to_string_map` serializes non-string YAML values
    /// back to YAML strings. This function deserializes the `"triggers"` key
    /// (if present) into structured `SkillTriggers`.
    pub fn parse_triggers(metadata: &HashMap<String, String>) -> SkillTriggers {
        let mut triggers = SkillTriggers {
            confidence_threshold: DEFAULT_CONFIDENCE_THRESHOLD,
            priority: DEFAULT_PRIORITY,
            ..Default::default()
        };

        // Parse top-level simple fields.
        if let Some(v) = metadata.get("confidence_threshold") {
            triggers.confidence_threshold = v.parse().unwrap_or(DEFAULT_CONFIDENCE_THRESHOLD);
        }
        if let Some(v) = metadata.get("priority") {
            triggers.priority = v.parse().unwrap_or(DEFAULT_PRIORITY);
        }
        if let Some(v) = metadata.get("exclusive_group") {
            triggers.exclusive_group = Some(v.clone());
        }
        if let Some(v) = metadata.get("cooldown") {
            triggers.cooldown_secs = v.parse().ok();
        }

        // Parse the nested `triggers` key (serialized as YAML string).
        let Some(triggers_yaml) = metadata.get("triggers") else {
            return triggers;
        };

        let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(triggers_yaml) else {
            return triggers;
        };

        if let Some(mapping) = value.as_mapping() {
            // intent_patterns
            if let Some(patterns) = mapping.get(serde_yaml::Value::String("intent_patterns".into()))
            {
                triggers.intent_patterns = yaml_string_list(patterns);
            }
            // tool_signals
            if let Some(tools) = mapping.get(serde_yaml::Value::String("tool_signals".into())) {
                triggers.tool_signals = yaml_string_list(tools);
            }
            // context_signals.keywords
            if let Some(ctx) = mapping.get(serde_yaml::Value::String("context_signals".into()))
                && let Some(ctx_map) = ctx.as_mapping()
                && let Some(kw) = ctx_map.get(serde_yaml::Value::String("keywords".into()))
            {
                triggers.context_keywords = yaml_string_list(kw);
            }
        }

        triggers
    }

    /// Score and select skills to auto-activate.
    ///
    /// Returns the set of skill names whose bodies should be expanded in the prompt.
    /// Skills without triggers are never auto-activated (backward compatible).
    pub fn match_skills(&self, skills: &[SkillMetadata], ctx: &MatchContext) -> HashSet<String> {
        let input_lower = ctx.task_input.to_lowercase();
        let recent_set: HashSet<&str> = ctx.recent_tools.iter().map(|s| s.as_str()).collect();

        // 1. Score each skill.
        let mut candidates: Vec<SkillMatch> = Vec::new();
        for skill in skills {
            let triggers = Self::parse_triggers(&skill.metadata);

            // Skip skills with no triggers (backward compat: these are just listed, not activated).
            if triggers.intent_patterns.is_empty()
                && triggers.tool_signals.is_empty()
                && triggers.context_keywords.is_empty()
            {
                continue;
            }

            let signals = score_signals(&triggers, &input_lower, &recent_set);
            let score = signals.intent_score * WEIGHT_INTENT
                + signals.tool_score * WEIGHT_TOOL
                + signals.keyword_score * WEIGHT_KEYWORD;

            if score < triggers.confidence_threshold {
                continue;
            }

            // Cooldown check.
            if let Some(cd) = triggers.cooldown_secs
                && !self
                    .cooldowns
                    .is_cooled_down(ctx.session_id, &skill.name, cd)
            {
                continue;
            }

            candidates.push(SkillMatch {
                skill_name: skill.name.clone(),
                score,
                signals,
                priority: triggers.priority,
                exclusive_group: triggers.exclusive_group,
            });
        }

        // 2. Resolve exclusive groups: keep only highest score per group.
        let mut group_winners: HashMap<String, usize> = HashMap::new();
        for (i, m) in candidates.iter().enumerate() {
            if let Some(ref group) = m.exclusive_group {
                let entry = group_winners.entry(group.clone()).or_insert(i);
                let current = &candidates[*entry];
                if m.score > current.score
                    || (m.score == current.score && m.priority > current.priority)
                {
                    *entry = i;
                }
            }
        }
        let excluded: HashSet<usize> = candidates
            .iter()
            .enumerate()
            .filter(|(i, m)| {
                if let Some(ref group) = m.exclusive_group {
                    group_winners.get(group) != Some(i)
                } else {
                    false
                }
            })
            .map(|(i, _)| i)
            .collect();

        // 3. Filter excluded, sort by score descending.
        let mut active: Vec<&SkillMatch> = candidates
            .iter()
            .enumerate()
            .filter(|(i, _)| !excluded.contains(i))
            .map(|(_, m)| m)
            .collect();
        active.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 4. Apply token budget.
        let mut budget_remaining = self.token_budget;
        let mut result = HashSet::new();
        for m in &active {
            // Estimate body size from the matching skill metadata.
            let body_size = skills
                .iter()
                .find(|s| s.name == m.skill_name)
                .and_then(|s| s.body().map(|b| b.len()))
                .unwrap_or(0);

            if body_size > budget_remaining {
                break;
            }
            budget_remaining -= body_size;
            result.insert(m.skill_name.clone());
        }

        // 5. Record activations for cooldown.
        for name in &result {
            self.cooldowns.record_activation(ctx.session_id, name);
        }

        result
    }
}

// ---------------------------------------------------------------------------
// Scoring helpers
// ---------------------------------------------------------------------------

fn score_signals(
    triggers: &SkillTriggers,
    input_lower: &str,
    recent_tools: &HashSet<&str>,
) -> MatchSignals {
    let intent_score = if triggers.intent_patterns.is_empty() {
        0.0
    } else {
        let matched = triggers.intent_patterns.iter().any(|pat| {
            Regex::new(pat)
                .map(|re| re.is_match(input_lower))
                .unwrap_or(false)
        });
        if matched { 1.0 } else { 0.0 }
    };

    let tool_score = if triggers.tool_signals.is_empty() {
        0.0
    } else {
        let matched = triggers
            .tool_signals
            .iter()
            .filter(|t| recent_tools.contains(t.as_str()))
            .count();
        matched as f64 / triggers.tool_signals.len() as f64
    };

    let keyword_score = if triggers.context_keywords.is_empty() {
        0.0
    } else {
        let matched = triggers
            .context_keywords
            .iter()
            .filter(|kw| input_lower.contains(&kw.to_lowercase()))
            .count();
        matched as f64 / triggers.context_keywords.len() as f64
    };

    MatchSignals {
        intent_score,
        tool_score,
        keyword_score,
    }
}

/// Extract a list of strings from a YAML value (handles both sequences and single strings).
fn yaml_string_list(value: &serde_yaml::Value) -> Vec<String> {
    match value {
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_owned()))
            .collect(),
        serde_yaml::Value::String(s) => vec![s.clone()],
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(name: &str, metadata: HashMap<String, String>) -> SkillMetadata {
        SkillMetadata {
            name: name.to_owned(),
            description: format!("Test skill {name}"),
            location: std::path::PathBuf::from(format!("/tmp/skills/{name}/SKILL.md")),
            source: "test".to_owned(),
            metadata,
            content: Some("skill body content".to_owned()),
        }
    }

    #[test]
    fn test_parse_triggers_empty() {
        let t = SkillMatcher::parse_triggers(&HashMap::new());
        assert!(t.intent_patterns.is_empty());
        assert_eq!(t.confidence_threshold, DEFAULT_CONFIDENCE_THRESHOLD);
        assert_eq!(t.priority, DEFAULT_PRIORITY);
    }

    #[test]
    fn test_parse_triggers_with_yaml() {
        let mut meta = HashMap::new();
        meta.insert(
            "triggers".to_owned(),
            "intent_patterns:\n  - \"research|调研\"\ntool_signals:\n  - web_search\ncontext_signals:\n  keywords:\n    - 调研\n    - research".to_owned(),
        );
        meta.insert("confidence_threshold".to_owned(), "0.5".to_owned());
        meta.insert("priority".to_owned(), "8".to_owned());
        meta.insert("exclusive_group".to_owned(), "research".to_owned());
        meta.insert("cooldown".to_owned(), "300".to_owned());

        let t = SkillMatcher::parse_triggers(&meta);
        assert_eq!(t.intent_patterns, vec!["research|调研"]);
        assert_eq!(t.tool_signals, vec!["web_search"]);
        assert_eq!(t.context_keywords, vec!["调研", "research"]);
        assert!((t.confidence_threshold - 0.5).abs() < f64::EPSILON);
        assert_eq!(t.priority, 8);
        assert_eq!(t.exclusive_group, Some("research".to_owned()));
        assert_eq!(t.cooldown_secs, Some(300));
    }

    #[test]
    fn test_match_skills_intent_hit() {
        let mut meta = HashMap::new();
        meta.insert(
            "triggers".to_owned(),
            "intent_patterns:\n  - \"hello|hi\"".to_owned(),
        );
        meta.insert("confidence_threshold".to_owned(), "0.1".to_owned());

        let skills = vec![make_skill("greeting", meta)];
        let matcher = SkillMatcher::new();
        let ctx = MatchContext {
            task_input: "hello there",
            recent_tools: &[],
            session_id: "test-session",
        };
        let result = matcher.match_skills(&skills, &ctx);
        assert!(result.contains("greeting"));
    }

    #[test]
    fn test_match_skills_no_triggers_skipped() {
        let skills = vec![make_skill("basic", HashMap::new())];
        let matcher = SkillMatcher::new();
        let ctx = MatchContext {
            task_input: "anything",
            recent_tools: &[],
            session_id: "test-session",
        };
        let result = matcher.match_skills(&skills, &ctx);
        assert!(result.is_empty());
    }

    #[test]
    fn test_exclusive_group_keeps_highest() {
        let make = |name: &str, pattern: &str, priority: u32| {
            let mut meta = HashMap::new();
            meta.insert(
                "triggers".to_owned(),
                format!("intent_patterns:\n  - \"{pattern}\""),
            );
            meta.insert("confidence_threshold".to_owned(), "0.1".to_owned());
            meta.insert("exclusive_group".to_owned(), "research".to_owned());
            meta.insert("priority".to_owned(), priority.to_string());
            make_skill(name, meta)
        };

        let skills = vec![
            make("deep-research", "research", 8),
            make("quick-search", "research", 5),
        ];
        let matcher = SkillMatcher::new();
        let ctx = MatchContext {
            task_input: "research this topic",
            recent_tools: &[],
            session_id: "test-session",
        };
        let result = matcher.match_skills(&skills, &ctx);
        assert!(result.contains("deep-research"));
        assert!(!result.contains("quick-search"));
    }

    #[test]
    fn test_cooldown_blocks_reactivation() {
        let mut meta = HashMap::new();
        meta.insert(
            "triggers".to_owned(),
            "intent_patterns:\n  - \"hello\"".to_owned(),
        );
        meta.insert("confidence_threshold".to_owned(), "0.1".to_owned());
        meta.insert("cooldown".to_owned(), "3600".to_owned()); // 1 hour

        let skills = vec![make_skill("greeting", meta)];
        let matcher = SkillMatcher::new();
        let ctx = MatchContext {
            task_input: "hello",
            recent_tools: &[],
            session_id: "sess-1",
        };

        // First call activates.
        let r1 = matcher.match_skills(&skills, &ctx);
        assert!(r1.contains("greeting"));

        // Second call should be blocked by cooldown.
        let r2 = matcher.match_skills(&skills, &ctx);
        assert!(r2.is_empty());
    }
}
