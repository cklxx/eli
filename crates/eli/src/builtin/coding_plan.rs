//! Coding Plan provider presets.

pub(crate) const VOLCANO_PROFILE: &str = "volcano";
pub(crate) const VOLCANO_PROVIDER: &str = "volcano";
pub(crate) const VOLCANO_OPENAI_BASE: &str =
    nexil::core::provider_policies::VOLCANO_CODING_OPENAI_BASE;

const VOLCANO_MODELS: &[&str] = &[
    "ark-code-latest",
    "doubao-seed-2.0-code",
    "doubao-seed-2.0-pro",
    "doubao-seed-2.0-lite",
    "doubao-seed-code",
    "minimax-latest",
    "glm-5.1",
    "glm-4.7",
    "deepseek-v3.2",
    "kimi-k2.6",
    "kimi-k2.5",
];

pub(crate) fn volcano_models() -> Vec<String> {
    VOLCANO_MODELS
        .iter()
        .map(|model| (*model).to_owned())
        .collect()
}

pub(crate) fn volcano_model_at(choice: usize) -> Option<&'static str> {
    VOLCANO_MODELS.get(choice.checked_sub(1)?).copied()
}
