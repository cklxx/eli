//! Command classification and semantic exit-code interpretation.
//!
//! Many commands use non-zero exit codes for informational outcomes, not errors.
//! For example `grep` returns 1 when no matches are found. Without this module
//! the bash tool would surface that as a `ConduitError`, causing the model to
//! enter unnecessary error-recovery loops.

// ---------------------------------------------------------------------------
// Exit-code semantics
// ---------------------------------------------------------------------------

/// Whether a non-zero exit code is a real error or an informational outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitOutcome {
    /// Treat as a hard error — return `ConduitError`.
    Error,
    /// Informational — return `Ok` with the given explanation.
    Info(&'static str),
}

/// Interpret a command's exit code using per-command semantic rules.
///
/// Unknown commands fall through to `Error` for any non-zero code (the
/// pre-existing behavior), so this is always safe to call.
pub fn interpret_exit(cmd: &str, code: i32) -> ExitOutcome {
    if code == 0 {
        return ExitOutcome::Info("success");
    }

    let base = base_command(cmd);
    match (base, code) {
        // grep family: 1 = no matches, ≥2 = real error
        ("grep" | "egrep" | "fgrep" | "rg" | "ag" | "ack", 1) => {
            ExitOutcome::Info("No matches found")
        }
        // diff: 1 = files differ, ≥2 = trouble
        ("diff" | "colordiff", 1) => ExitOutcome::Info("Files differ"),
        // find: 1 = some paths inaccessible
        ("find" | "fd", 1) => ExitOutcome::Info("Some paths were inaccessible"),
        // test / [: 1 = condition false
        ("test" | "[", 1) => ExitOutcome::Info("Condition is false"),
        // Everything else (or higher codes for the above)
        _ => ExitOutcome::Error,
    }
}

// ---------------------------------------------------------------------------
// Silent-command detection
// ---------------------------------------------------------------------------

/// Returns `true` for commands that produce no stdout on success by design.
///
/// When one of these exits 0 with empty output the tool returns `"Done"`
/// instead of the generic `"(command succeeded, no output)"`.
pub fn is_silent_command(cmd: &str) -> bool {
    matches!(
        base_command(cmd),
        "mv" | "cp"
            | "rm"
            | "mkdir"
            | "rmdir"
            | "touch"
            | "chmod"
            | "chown"
            | "chgrp"
            | "ln"
            | "unlink"
            | "install"
            | "cd"
            | "export"
            | "unset"
    )
}

// ---------------------------------------------------------------------------
// Sleep guard
// ---------------------------------------------------------------------------

/// Detect `sleep N` (N ≥ 2) as the first/only command.
///
/// Returns a human-readable rejection reason, or `None` if the command is fine.
/// Commands containing `&` (shell-level backgrounding) are allowed through.
pub fn is_blocking_sleep(cmd: &str) -> Option<String> {
    let trimmed = cmd.trim();

    // Shell-level backgrounding is fine
    if trimmed.contains('&') && !trimmed.contains("&&") {
        return None;
    }

    // Extract the first simple command (before any ; or &&)
    let first = trimmed.split([';', '|']).next().unwrap_or(trimmed).trim();
    // Also split on &&
    let first = first.split("&&").next().unwrap_or(first).trim();

    let base = base_command(first);
    if base != "sleep" {
        return None;
    }

    // Parse the sleep argument
    let arg = first
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())?;

    if arg < 2.0 {
        return None; // sub-2s sleeps are fine (rate limiting, pacing)
    }

    let rest = trimmed
        .trim_start_matches(first)
        .trim_start_matches(|c: char| c == ';' || c == '&' || c.is_whitespace());

    if rest.is_empty() {
        Some(format!(
            "Blocked: standalone sleep {arg}s. Use background=true for long waits."
        ))
    } else {
        Some(format!(
            "Blocked: sleep {arg}s followed by: {rest}. \
             Use background=true, or keep sleep under 2s."
        ))
    }
}

// ---------------------------------------------------------------------------
// Command extraction helpers
// ---------------------------------------------------------------------------

/// Extract the base command name from a shell command string.
///
/// Handles env-var prefixes (`FOO=bar grep …`), absolute paths
/// (`/usr/bin/grep`), and pipes (returns the *first* command since it
/// determines the semantic context for compound pipelines like
/// `grep … | head`).
pub fn base_command(cmd: &str) -> &str {
    let trimmed = cmd.trim();

    // Take content before the first pipe (the primary command)
    let before_pipe = trimmed.split('|').next().unwrap_or(trimmed).trim();

    // Walk tokens, skip env-var assignments (contain `=` without leading -)
    for token in before_pipe.split_whitespace() {
        if token.contains('=') && !token.starts_with('-') {
            continue;
        }
        // Strip path: /usr/bin/grep → grep
        return token.rsplit('/').next().unwrap_or(token);
    }

    // Fallback: return first whitespace-delimited token
    trimmed
        .split_whitespace()
        .next()
        .unwrap_or(trimmed)
        .rsplit('/')
        .next()
        .unwrap_or(trimmed)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- base_command -------------------------------------------------------

    #[test]
    fn base_command_simple() {
        assert_eq!(base_command("grep -r foo ."), "grep");
    }

    #[test]
    fn base_command_env_prefix() {
        assert_eq!(base_command("FOO=1 BAR=2 grep pattern"), "grep");
    }

    #[test]
    fn base_command_absolute_path() {
        assert_eq!(base_command("/usr/bin/grep foo"), "grep");
    }

    #[test]
    fn base_command_pipe() {
        assert_eq!(base_command("grep foo | head -5"), "grep");
    }

    #[test]
    fn base_command_bare() {
        assert_eq!(base_command("ls"), "ls");
    }

    #[test]
    fn base_command_with_whitespace() {
        assert_eq!(base_command("  diff  a.txt b.txt  "), "diff");
    }

    // -- interpret_exit -----------------------------------------------------

    #[test]
    fn grep_no_matches() {
        assert_eq!(
            interpret_exit("grep -r foo .", 1),
            ExitOutcome::Info("No matches found")
        );
    }

    #[test]
    fn grep_real_error() {
        assert_eq!(interpret_exit("grep -r foo .", 2), ExitOutcome::Error);
    }

    #[test]
    fn rg_no_matches() {
        assert_eq!(
            interpret_exit("rg pattern", 1),
            ExitOutcome::Info("No matches found")
        );
    }

    #[test]
    fn diff_files_differ() {
        assert_eq!(
            interpret_exit("diff a.txt b.txt", 1),
            ExitOutcome::Info("Files differ")
        );
    }

    #[test]
    fn diff_real_error() {
        assert_eq!(interpret_exit("diff a.txt b.txt", 2), ExitOutcome::Error);
    }

    #[test]
    fn find_partial() {
        assert_eq!(
            interpret_exit("find / -name '*.rs'", 1),
            ExitOutcome::Info("Some paths were inaccessible")
        );
    }

    #[test]
    fn test_condition_false() {
        assert_eq!(
            interpret_exit("test -f /nonexistent", 1),
            ExitOutcome::Info("Condition is false")
        );
    }

    #[test]
    fn bracket_condition_false() {
        assert_eq!(
            interpret_exit("[ -f /nonexistent ]", 1),
            ExitOutcome::Info("Condition is false")
        );
    }

    #[test]
    fn unknown_command_error() {
        assert_eq!(interpret_exit("cargo build", 1), ExitOutcome::Error);
    }

    #[test]
    fn exit_zero_always_info() {
        assert_eq!(interpret_exit("anything", 0), ExitOutcome::Info("success"));
    }

    // -- is_silent_command --------------------------------------------------

    #[test]
    fn silent_mv() {
        assert!(is_silent_command("mv a b"));
    }

    #[test]
    fn silent_mkdir() {
        assert!(is_silent_command("mkdir -p /tmp/foo"));
    }

    #[test]
    fn not_silent_grep() {
        assert!(!is_silent_command("grep foo"));
    }

    #[test]
    fn not_silent_ls() {
        assert!(!is_silent_command("ls -la"));
    }

    // -- is_blocking_sleep --------------------------------------------------

    #[test]
    fn sleep_standalone_blocked() {
        assert!(is_blocking_sleep("sleep 5").is_some());
    }

    #[test]
    fn sleep_with_follow_up_blocked() {
        assert!(is_blocking_sleep("sleep 5 && echo done").is_some());
    }

    #[test]
    fn sleep_short_allowed() {
        assert!(is_blocking_sleep("sleep 1").is_none());
    }

    #[test]
    fn sleep_sub_second_allowed() {
        assert!(is_blocking_sleep("sleep 0.5").is_none());
    }

    #[test]
    fn sleep_backgrounded_allowed() {
        assert!(is_blocking_sleep("sleep 10 &").is_none());
    }

    #[test]
    fn not_sleep_command() {
        assert!(is_blocking_sleep("echo hello").is_none());
    }

    #[test]
    fn sleep_not_first_command() {
        // sleep is second in a pipe — this is fine
        assert!(is_blocking_sleep("echo | sleep 10").is_none());
    }
}
