//! Tool registry and helpers for the eli crate.

use std::collections::HashMap;
use std::sync::Mutex;

use conduit::Tool;

/// Central tool registry. Tools are registered here by the builtin module on startup.
pub static REGISTRY: std::sync::LazyLock<Mutex<HashMap<String, Tool>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

// ---------------------------------------------------------------------------
// Sidecar state
// ---------------------------------------------------------------------------

/// URL of the running sidecar (e.g. "http://127.0.0.1:3101").
/// Set by wait_for_sidecar() when the sidecar is ready.
pub static SIDECAR_URL: std::sync::LazyLock<Mutex<Option<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Convert a tool name with dots to underscore-separated form for model APIs.
fn to_model_name(name: &str) -> String {
    name.replace('.', "_")
}

/// Produce a list of tools with names converted for model consumption.
pub fn model_tools(tools: &[Tool]) -> Vec<Tool> {
    tools
        .iter()
        .map(|tool| {
            let mut cloned = tool.clone();
            cloned.name = to_model_name(&cloned.name);
            cloned
        })
        .collect()
}

/// Render a human-readable description of tools for model prompts.
pub fn render_tools_prompt<'a>(tools: impl IntoIterator<Item = &'a Tool>) -> String {
    let mut lines: Vec<String> = Vec::new();
    for tool in tools {
        let name = to_model_name(&tool.name);
        if tool.description.is_empty() {
            lines.push(format!("- {name}"));
        } else {
            lines.push(format!("- {name}: {}", tool.description));
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "<available_tools>\n{}\n</available_tools>",
        lines.join("\n")
    )
}

/// Shorten a text string for logging.
pub fn shorten_text(text: &str, width: usize) -> String {
    if text.len() <= width {
        return text.to_owned();
    }
    let placeholder = "...";
    let available = width.saturating_sub(placeholder.len());
    if available == 0 {
        return placeholder.to_owned();
    }
    format!("{}{placeholder}", &text[..available])
}

#[cfg(test)]
mod tests {
    use super::*;
    use conduit::Tool;
    use serde_json::json;

    fn make_tool(name: &str, description: &str) -> Tool {
        Tool::schema_only(name, description, json!({}))
    }

    // -- to_model_name --------------------------------------------------------

    #[test]
    fn test_to_model_name_replaces_dots() {
        assert_eq!(to_model_name("tests.rename_me"), "tests_rename_me");
    }

    #[test]
    fn test_to_model_name_no_dots() {
        assert_eq!(to_model_name("simple"), "simple");
    }

    #[test]
    fn test_to_model_name_multiple_dots() {
        assert_eq!(to_model_name("a.b.c"), "a_b_c");
    }

    // -- model_tools ----------------------------------------------------------

    #[test]
    fn test_model_tools_rewrites_names_without_mutating_original() {
        let tool = make_tool("tests.rename_me", "rename");
        let rewritten = model_tools(&[tool.clone()]);
        assert_eq!(rewritten.len(), 1);
        assert_eq!(rewritten[0].name, "tests_rename_me");
        // Original should be unchanged
        assert_eq!(tool.name, "tests.rename_me");
    }

    #[test]
    fn test_model_tools_empty() {
        let rewritten = model_tools(&[]);
        assert!(rewritten.is_empty());
    }

    // -- render_tools_prompt --------------------------------------------------

    #[test]
    fn test_render_tools_prompt_with_description() {
        let t1 = make_tool("tests.prompt_one", "First tool");
        let t2 = make_tool("tests.prompt_two", "");
        let rendered = render_tools_prompt([&t1, &t2]);
        assert_eq!(
            rendered,
            "<available_tools>\n- tests_prompt_one: First tool\n- tests_prompt_two\n</available_tools>"
        );
    }

    #[test]
    fn test_render_tools_prompt_empty_returns_empty_string() {
        let rendered = render_tools_prompt(std::iter::empty::<&Tool>());
        assert_eq!(rendered, "");
    }

    #[test]
    fn test_render_tools_prompt_single_tool() {
        let t = make_tool("my.tool", "does stuff");
        let rendered = render_tools_prompt([&t]);
        assert!(rendered.contains("- my_tool: does stuff"));
        assert!(rendered.starts_with("<available_tools>"));
        assert!(rendered.ends_with("</available_tools>"));
    }

    // -- REGISTRY -------------------------------------------------------------

    #[test]
    fn test_registry_insert_and_lookup() {
        let tool = make_tool("test.registry_tool", "a tool");
        {
            let mut reg = REGISTRY.lock().unwrap();
            reg.insert("test.registry_tool".into(), tool.clone());
        }
        let reg = REGISTRY.lock().unwrap();
        assert!(reg.contains_key("test.registry_tool"));
        assert_eq!(reg["test.registry_tool"].name, "test.registry_tool");
    }

    // -- shorten_text ---------------------------------------------------------

    #[test]
    fn test_shorten_text_short_enough() {
        assert_eq!(shorten_text("hello", 10), "hello");
    }

    #[test]
    fn test_shorten_text_truncates_with_ellipsis() {
        assert_eq!(shorten_text("hello world", 8), "hello...");
    }

    #[test]
    fn test_shorten_text_very_small_width() {
        assert_eq!(shorten_text("hello", 3), "...");
    }

    #[test]
    fn test_shorten_text_zero_width() {
        assert_eq!(shorten_text("hello", 0), "...");
    }
}
