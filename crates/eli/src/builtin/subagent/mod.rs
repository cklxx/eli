//! Subagent orchestration: tracking, worktree isolation, and in-process fallback.

pub mod fallback;
pub mod tracker;
pub mod worktree;

#[cfg(test)]
mod tests;
