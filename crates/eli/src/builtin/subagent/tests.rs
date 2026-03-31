//! Tests for the subagent subsystem: tracker, worktree, fallback.

#[cfg(test)]
mod tracker_tests {
    use crate::builtin::subagent::tracker::{AgentResult, AgentTracker};

    #[tokio::test]
    async fn test_register_and_list() {
        let tracker = AgentTracker::new();
        assert!(
            tracker
                .register(
                    "a1",
                    Some("sh-1".into()),
                    "general",
                    "test task",
                    "/tmp",
                    "claude"
                )
                .await
        );
        let list = tracker.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "a1");
        assert!(list[0].1.running);
    }

    #[tokio::test]
    async fn test_complete_and_get_result() {
        let tracker = AgentTracker::new();
        tracker
            .register("a2", None, "explore", "search", "/tmp", "codex")
            .await;
        tracker
            .complete(
                "a2",
                AgentResult {
                    exit_code: Some(0),
                    output: "found 3 matches".into(),
                    artifacts: "(no changes)".into(),
                    duration_ms: 1234,
                },
            )
            .await;
        let result = tracker.get_result("a2").await.expect("result should exist");
        assert_eq!(result.exit_code, Some(0));
        assert!(result.output.contains("found"));
    }

    #[tokio::test]
    async fn test_running_count() {
        let tracker = AgentTracker::new();
        tracker
            .register("a3", None, "general", "t1", "/", "claude")
            .await;
        tracker
            .register("a4", None, "general", "t2", "/", "claude")
            .await;
        assert_eq!(tracker.running_count().await, 2);

        tracker
            .complete(
                "a3",
                AgentResult {
                    exit_code: Some(0),
                    output: String::new(),
                    artifacts: String::new(),
                    duration_ms: 100,
                },
            )
            .await;
        assert_eq!(tracker.running_count().await, 1);
    }

    #[tokio::test]
    async fn test_evict_completed() {
        let tracker = AgentTracker::new();
        tracker
            .register("a5", None, "general", "old", "/", "claude")
            .await;
        tracker
            .complete(
                "a5",
                AgentResult {
                    exit_code: Some(0),
                    output: String::new(),
                    artifacts: String::new(),
                    duration_ms: 0,
                },
            )
            .await;

        // With a zero max_age, completed agents should be evicted.
        tracker.evict_completed(std::time::Duration::ZERO).await;
        assert!(tracker.get_result("a5").await.is_none());
    }

    #[tokio::test]
    async fn test_nonexistent_result_returns_none() {
        let tracker = AgentTracker::new();
        assert!(tracker.get_result("does-not-exist").await.is_none());
    }
}

#[cfg(test)]
mod tool_tests {
    use super::super::tracker;

    #[test]
    fn test_global_tracker_accessible() {
        let _t = tracker::agent_tracker();
    }
}
