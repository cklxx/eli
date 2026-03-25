//! Spill large tool results to disk, keeping only head + tail in tape.

use std::cmp::min;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Configuration for spilling large tool outputs to disk.
pub struct SpillConfig {
    /// Character count threshold (Unicode scalar values, not bytes).
    pub threshold_chars: usize,
    pub head_lines: usize,
    pub tail_lines: usize,
}

/// Default spill configuration.
pub const DEFAULT_SPILL: SpillConfig = SpillConfig {
    threshold_chars: 500,
    head_lines: 15,
    tail_lines: 5,
};

/// If `content` exceeds the threshold (in Unicode chars), write full content to
/// a spill file under `{spill_dir}/{call_id}.txt` and return a truncated
/// version with head + tail + file reference. Otherwise return `None`.
pub fn spill_if_needed(
    content: &str,
    call_id: &str,
    spill_dir: &Path,
    config: &SpillConfig,
) -> io::Result<Option<String>> {
    if content.chars().count() <= config.threshold_chars {
        return Ok(None);
    }

    fs::create_dir_all(spill_dir)?;
    let spill_path = spill_dir.join(format!("{call_id}.txt"));
    fs::write(&spill_path, content)?;

    Ok(Some(build_truncated(content, &spill_path, config)))
}

/// Build the spill directory path for a given tape name.
/// Convention: `{tapes_dir}/{tape_name}.d/`
pub fn spill_dir_for_tape(tapes_dir: &Path, tape_name: &str) -> PathBuf {
    tapes_dir.join(format!("{tape_name}.d"))
}

fn build_truncated(content: &str, path: &Path, config: &SpillConfig) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let total_chars = content.chars().count();

    let head_end = min(config.head_lines, total_lines);
    let tail_start = total_lines.saturating_sub(config.tail_lines);

    // If line-based truncation doesn't actually remove anything (e.g. a single
    // long line, or fewer lines than head+tail), fall back to char-level split.
    let line_based_keeps_all = tail_start <= head_end;
    if line_based_keeps_all {
        return char_level_truncate(content, total_chars, path, config);
    }

    let head = lines[..head_end].join("\n");
    let tail = lines[tail_start..].join("\n");
    let omitted_lines = tail_start - head_end;
    let omitted_chars = total_chars.saturating_sub(head.chars().count() + tail.chars().count());

    format!(
        "{head}\n\n[{omitted_lines} lines, {omitted_chars} chars omitted — full output: {}]\n\n{tail}",
        path.display()
    )
}

/// Fall back to character-level truncation when line-based splitting can't
/// remove content (single long line, minified JSON, etc.).
///
/// Takes first `budget / 2` chars and last `budget / 4` chars, where
/// `budget = threshold_chars`. Always cuts at char boundaries.
fn char_level_truncate(
    content: &str,
    total_chars: usize,
    path: &Path,
    config: &SpillConfig,
) -> String {
    let head_chars = config.threshold_chars / 2;
    let tail_chars = config.threshold_chars / 4;

    let head_byte_end = char_offset_to_byte(content, head_chars);
    let tail_byte_start = char_offset_from_end(content, tail_chars);

    let head = &content[..head_byte_end];
    let tail = if tail_byte_start > head_byte_end {
        &content[tail_byte_start..]
    } else {
        ""
    };

    let kept_chars = head.chars().count() + tail.chars().count();
    let omitted = total_chars.saturating_sub(kept_chars);

    if tail.is_empty() {
        format!(
            "{head}\n\n[{omitted} chars omitted — full output: {}]",
            path.display()
        )
    } else {
        format!(
            "{head}\n\n[{omitted} chars omitted — full output: {}]\n\n{tail}",
            path.display()
        )
    }
}

/// Return the byte offset of the n-th char (or content.len() if n >= char count).
fn char_offset_to_byte(s: &str, n: usize) -> usize {
    s.char_indices()
        .nth(n)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(s.len())
}

/// Return the byte offset such that the last `n` chars start there.
fn char_offset_from_end(s: &str, n: usize) -> usize {
    let total = s.chars().count();
    if n >= total {
        return 0;
    }
    char_offset_to_byte(s, total - n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn below_threshold_returns_none() {
        let dir = tempdir().unwrap();
        let result = spill_if_needed("short", "call1", dir.path(), &DEFAULT_SPILL).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn above_threshold_spills_to_file() {
        let dir = tempdir().unwrap();
        let content: String = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = spill_if_needed(&content, "call_abc", dir.path(), &DEFAULT_SPILL).unwrap();
        assert!(result.is_some());

        // Spill file exists with full content
        let spill_path = dir.path().join("call_abc.txt");
        assert!(spill_path.exists());
        assert_eq!(fs::read_to_string(&spill_path).unwrap(), content);

        // Truncated output contains head, tail, and reference
        let truncated = result.unwrap();
        assert!(truncated.contains("line 0"));
        assert!(truncated.contains("line 14")); // last head line
        assert!(truncated.contains("line 99")); // last tail line
        assert!(truncated.contains("lines,"));
        assert!(truncated.contains("chars omitted"));
        assert!(truncated.contains("call_abc.txt"));
    }

    #[test]
    fn head_tail_no_overlap() {
        let dir = tempdir().unwrap();
        // 25 lines, head=15, tail=5 → omit lines 15..20
        let content: String = (0..25)
            .map(|i| format!("L{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let config = SpillConfig {
            threshold_chars: 10,
            head_lines: 15,
            tail_lines: 5,
        };
        let truncated = spill_if_needed(&content, "c1", dir.path(), &config)
            .unwrap()
            .unwrap();
        assert!(truncated.contains("L0"));
        assert!(truncated.contains("L14"));
        assert!(truncated.contains("L20"));
        assert!(truncated.contains("L24"));
        assert!(truncated.contains("5 lines,"));
    }

    // -- Threshold uses char count, not byte count --

    #[test]
    fn threshold_counts_chars_not_bytes() {
        let dir = tempdir().unwrap();
        // 200 Chinese chars = 600 bytes. Threshold=500 chars → should NOT spill.
        let content: String = "你".repeat(200);
        assert_eq!(content.len(), 600); // 600 bytes
        assert_eq!(content.chars().count(), 200); // 200 chars
        let result = spill_if_needed(&content, "cjk", dir.path(), &DEFAULT_SPILL).unwrap();
        assert!(
            result.is_none(),
            "200 chars < 500 threshold, should not spill"
        );
    }

    #[test]
    fn threshold_spills_cjk_over_limit() {
        let dir = tempdir().unwrap();
        // 501 Chinese chars → should spill
        let content: String = "你".repeat(501);
        assert_eq!(content.chars().count(), 501);
        let result = spill_if_needed(&content, "cjk_over", dir.path(), &DEFAULT_SPILL).unwrap();
        assert!(result.is_some(), "501 chars > 500 threshold, should spill");

        // Reversible
        let recovered = fs::read_to_string(dir.path().join("cjk_over.txt")).unwrap();
        assert_eq!(recovered, content);
    }

    // -- Single-line large content (no newlines) --

    #[test]
    fn single_line_large_content_uses_char_level_truncation() {
        let dir = tempdir().unwrap();
        // One long line, no newlines — 1000 chars
        let content: String = "x".repeat(1000);
        let config = SpillConfig {
            threshold_chars: 100,
            head_lines: 5,
            tail_lines: 2,
        };
        let truncated = spill_if_needed(&content, "single", dir.path(), &config)
            .unwrap()
            .unwrap();

        // Should be significantly shorter than original
        assert!(
            truncated.chars().count() < content.chars().count(),
            "truncated ({}) should be shorter than original ({})",
            truncated.chars().count(),
            content.chars().count()
        );
        assert!(truncated.contains("chars omitted"));
        assert!(truncated.contains("single.txt"));

        // Head is threshold/2 = 50 chars of 'x'
        assert!(truncated.starts_with(&"x".repeat(50)));

        // Reversible
        let recovered = fs::read_to_string(dir.path().join("single.txt")).unwrap();
        assert_eq!(recovered, content);
    }

    #[test]
    fn single_line_json_blob_truncates() {
        let dir = tempdir().unwrap();
        // Minified JSON — no newlines, very long
        let json_blob = format!(
            r#"{{"data":[{}]}}"#,
            (0..200)
                .map(|i| format!(r#"{{"id":{i},"value":"item_{i}"}}"#))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(json_blob.chars().count() > 500);
        assert_eq!(json_blob.lines().count(), 1);

        let truncated = spill_if_needed(&json_blob, "json", dir.path(), &DEFAULT_SPILL)
            .unwrap()
            .unwrap();

        assert!(truncated.chars().count() < json_blob.chars().count());
        assert!(truncated.contains("chars omitted"));

        // Reversible
        let recovered = fs::read_to_string(dir.path().join("json.txt")).unwrap();
        assert_eq!(recovered, json_blob);
    }

    // -- CJK char-level truncation --

    #[test]
    fn cjk_single_line_splits_at_char_boundary() {
        let dir = tempdir().unwrap();
        // 600 Chinese chars, no newlines
        let content: String = (0..600)
            .map(|i| char::from_u32('一' as u32 + (i % 100)).unwrap_or('？'))
            .collect();
        let config = SpillConfig {
            threshold_chars: 100,
            head_lines: 5,
            tail_lines: 2,
        };
        let truncated = spill_if_needed(&content, "cjk_line", dir.path(), &config)
            .unwrap()
            .unwrap();

        // Head should be exactly 50 chars (threshold/2)
        let head_part: &str = truncated.split("\n\n[").next().unwrap();
        assert_eq!(head_part.chars().count(), 50);

        // Verify it's valid UTF-8 (no broken chars)
        assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());

        // Reversible
        let recovered = fs::read_to_string(dir.path().join("cjk_line.txt")).unwrap();
        assert_eq!(recovered, content);
    }

    #[test]
    fn emoji_content_splits_correctly() {
        let dir = tempdir().unwrap();
        // Emoji are 4 bytes each
        let content: String = "🎉".repeat(200);
        assert_eq!(content.len(), 800); // 800 bytes
        assert_eq!(content.chars().count(), 200); // 200 chars
        let config = SpillConfig {
            threshold_chars: 50,
            head_lines: 5,
            tail_lines: 2,
        };
        let truncated = spill_if_needed(&content, "emoji", dir.path(), &config)
            .unwrap()
            .unwrap();

        // Valid UTF-8
        assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
        // Head = 25 emoji (threshold/2)
        let head_part: &str = truncated.split("\n\n[").next().unwrap();
        assert_eq!(head_part.chars().count(), 25);
        // Each char in head is the full emoji, not broken
        assert!(head_part.chars().all(|c| c == '🎉'));

        // Reversible
        let recovered = fs::read_to_string(dir.path().join("emoji.txt")).unwrap();
        assert_eq!(recovered, content);
    }

    // -- Mixed content --

    #[test]
    fn mixed_ascii_and_cjk_multiline() {
        let dir = tempdir().unwrap();
        let content: String = (0..100)
            .map(|i| {
                if i % 2 == 0 {
                    format!("english line {i}")
                } else {
                    format!("中文行 {i} 内容测试")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let truncated = spill_if_needed(&content, "mixed", dir.path(), &DEFAULT_SPILL)
            .unwrap()
            .unwrap();

        // Line-based truncation should work (many lines)
        assert!(truncated.contains("english line 0"));
        assert!(truncated.contains("中文行 1 内容测试"));
        assert!(truncated.contains("lines,"));
        assert!(truncated.contains("chars omitted"));

        // Reversible
        let recovered = fs::read_to_string(dir.path().join("mixed.txt")).unwrap();
        assert_eq!(recovered, content);
    }

    // -- Exact threshold boundary --

    #[test]
    fn exact_threshold_does_not_spill() {
        let dir = tempdir().unwrap();
        let config = SpillConfig {
            threshold_chars: 10,
            head_lines: 5,
            tail_lines: 2,
        };
        let content = "0123456789"; // exactly 10 chars
        let result = spill_if_needed(content, "exact", dir.path(), &config).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn one_over_threshold_spills() {
        let dir = tempdir().unwrap();
        let config = SpillConfig {
            threshold_chars: 10,
            head_lines: 5,
            tail_lines: 2,
        };
        let content = "01234567890"; // 11 chars
        let result = spill_if_needed(content, "over", dir.path(), &config).unwrap();
        assert!(result.is_some());

        let recovered = fs::read_to_string(dir.path().join("over.txt")).unwrap();
        assert_eq!(recovered, content);
    }

    // -- Previously existing tests kept --

    #[test]
    fn head_covers_all_lines_falls_back_to_char_level() {
        let dir = tempdir().unwrap();
        // Many chars on few lines — line-based can't truncate, char-level kicks in
        let content: String = (0..5)
            .map(|i| format!("line {i}: {}", "x".repeat(200)))
            .collect::<Vec<_>>()
            .join("\n");
        let config = SpillConfig {
            threshold_chars: 100,
            head_lines: 10, // more than 5 lines → covers all
            tail_lines: 3,
        };
        let truncated = spill_if_needed(&content, "c2", dir.path(), &config)
            .unwrap()
            .unwrap();
        // Falls back to char-level since head_lines covers all lines
        assert!(truncated.contains("chars omitted"));
        // Head is threshold/2 = 50 chars
        let head_part: &str = truncated.split("\n\n[").next().unwrap();
        assert_eq!(head_part.chars().count(), 50);

        // Reversible
        let recovered = fs::read_to_string(dir.path().join("c2.txt")).unwrap();
        assert_eq!(recovered, content);
    }
}
