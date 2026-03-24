## Status
done

## Summary
Code review of crates/eli/src/ covering framework, hooks, channels, and builtin modules. Found 8 issues across severity levels. The codebase is generally well-structured with good error handling and panic isolation. No critical logic bugs found.

## Findings

### 1. ChannelManager cleanup loop spin-waits with polling
- **File**: `channels/manager.rs:369-406`
- **Severity**: Medium
- **Description**: The cleanup task for finished processing tasks uses a polling loop with `yield_now()` + `sleep(50ms)` + repeated lock acquisitions on `task_handles` to detect task completion. This is wasteful compared to just awaiting the `JoinHandle` directly. Each spawned task creates a cleanup coroutine that polls every 50ms and acquires two mutexes per iteration. Under high message throughput this creates unnecessary lock contention and CPU usage.
- **Suggestion**: Instead of polling, clone the `JoinHandle` into the cleanup task and await it directly, then do the map cleanup.

### 2. Telegram bot calls `get_me()` on every single message
- **File**: `channels/telegram.rs:666-671`
- **Severity**: Medium
- **Description**: Inside the per-message endpoint handler, `bot.get_me().await` is called for every incoming message to get `bot_id` and `bot_username` for the `should_process_message` check. This is a network round-trip to the Telegram API on every message. The bot's identity doesn't change — it should be fetched once at startup and shared via a closure capture or `Arc`.
- **Suggestion**: Call `bot.get_me()` once during `start()`, store the result, and capture it in the handler closure.

### 3. `shorten_text` slices on byte boundary, not character boundary
- **File**: `tools.rs:68`
- **Severity**: Low
- **Description**: `&text[..available]` slices at a byte offset. If `text` contains multi-byte UTF-8 characters (e.g., CJK, emoji), this will panic at runtime. The function is used for logging tool output and search results, so non-ASCII content is plausible.
- **Suggestion**: Use `text.char_indices()` to find the correct byte offset for the character boundary, or use `text.chars().take(available).collect::<String>()`.

### 4. `ChannelMessage::from_batch` panics on empty input
- **File**: `channels/message.rs:213`
- **Severity**: Low
- **Description**: `from_batch` uses `assert!(!batch.is_empty())` which panics. This is called from `BufferedMessageHandler::schedule_flush` after draining the `pending` buffer. While the code checks `guard.pending.is_empty()` before calling, a race between the empty check and the drain (both under the same lock, so actually safe) means this is unlikely to trigger. However, the panic policy is inconsistent — everywhere else the codebase returns `Result` errors. The panic is documented and tested, so this is a style concern, not a bug.

### 5. `ChannelMessage::context` stores channel with `$` prefix
- **File**: `channels/message.rs:119`
- **Severity**: Low (potential confusion)
- **Description**: `ChannelMessage::new()` stores the channel in context as `format!("${channel}")` (e.g., `$telegram`). The `finalize()` method does the same. This `$` prefix convention is likely intentional for template substitution, but it's not documented and could cause confusion if anyone reads the context `channel` value expecting the raw channel name.

### 6. `load_state` iteration direction inconsistency with comment
- **File**: `framework.rs:144-150`
- **Severity**: Info
- **Description**: The comment says "Iterate last-registered-first; `or_insert` keeps the first value seen per key, so later-registered plugins take priority." The code uses `hook_states.into_iter().rev().flatten()`. The `call_load_state` in hooks.rs iterates in forward (registration) order. Then `framework.rs` reverses the results. The net effect is: last-registered plugin's state wins. This is correct and matches the "last-registered wins" policy. No bug, just noting the correctness chain is non-obvious.

### 7. `skills.rs` frontmatter parsing off-by-one
- **File**: `skills.rs:167-173`
- **Severity**: Info (not a bug)
- **Description**: `parse_frontmatter` iterates `lines[1..]` and when it finds the closing `---` at relative index `idx`, it takes `lines[1..=idx]` as the payload. Since `idx` is relative to `lines[1..]`, this actually takes lines 1 through idx (0-based from the slice), which correctly excludes the closing delimiter. The indexing is correct but the mix of relative/absolute slice indices makes it easy to misread.

### 8. `FileTapeStore::read` line length calculation assumes single-byte newlines
- **File**: `builtin/store.rs:358-359`
- **Severity**: Low
- **Description**: `let line_len = raw_line.len() as u64 + 1;` assumes each line ends with a single `\n` byte. On Windows (if the file was written on Windows or transferred), `\r\n` line endings would cause the offset tracking to drift, leading to re-reading lines or skipping content. Since this is a Rust CLI tool primarily targeting Unix, the risk is low, but the offset calculation is fragile.

## Changes
- No code changes made (review-only task)

## Test Results
All 198 tests pass (cargo test --workspace): 0 failures, 0 ignored.
