//! Spill large tool results to disk, keeping only head + tail in tape.

use std::cmp::min;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Configuration for spilling large tool outputs to disk.
pub struct SpillConfig {
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

/// If `content` exceeds the threshold, write full content to a spill file
/// under `{spill_dir}/{call_id}.txt` and return a truncated version with
/// head + tail + file reference. Otherwise return `None`.
pub fn spill_if_needed(
    content: &str,
    call_id: &str,
    spill_dir: &Path,
    config: &SpillConfig,
) -> io::Result<Option<String>> {
    if content.len() <= config.threshold_chars {
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
    let total = lines.len();

    let head_end = min(config.head_lines, total);
    let tail_start = total.saturating_sub(config.tail_lines);

    // Avoid overlap: if head covers everything including tail, just show head
    if tail_start <= head_end {
        let head = lines[..head_end].join("\n");
        let omitted = total.saturating_sub(head_end);
        let omitted_chars = content.len().saturating_sub(head.len());
        if omitted == 0 {
            return head;
        }
        return format!(
            "{head}\n\n[{omitted} lines, {omitted_chars} chars omitted — full output: {}]",
            path.display()
        );
    }

    let head = lines[..head_end].join("\n");
    let tail = lines[tail_start..].join("\n");
    let omitted_lines = tail_start - head_end;
    let omitted_chars = content.len().saturating_sub(head.len() + tail.len());

    format!(
        "{head}\n\n[{omitted_lines} lines, {omitted_chars} chars omitted — full output: {}]\n\n{tail}",
        path.display()
    )
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

    #[test]
    fn head_covers_all_lines() {
        let dir = tempdir().unwrap();
        // 10 lines but threshold low enough to trigger spill
        let content: String = (0..10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let config = SpillConfig {
            threshold_chars: 10,
            head_lines: 15,
            tail_lines: 5,
        };
        let truncated = spill_if_needed(&content, "c2", dir.path(), &config)
            .unwrap()
            .unwrap();
        // head covers everything, no omission notice
        assert!(truncated.contains("line 0"));
        assert!(truncated.contains("line 9"));
        assert!(!truncated.contains("omitted"));
    }
}
