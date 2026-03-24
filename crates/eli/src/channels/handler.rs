//! Buffered message handler with debounce and active-time-window logic.

use std::sync::Arc;

use tokio::sync::{Mutex, Notify, mpsc};
use tokio::time::{Duration, Instant};
use tracing::{info, warn};

use super::message::ChannelMessage;

/// A message handler that buffers incoming messages and processes them in batch
/// with debounce and active-time-window semantics.
///
/// When a new message arrives:
///
/// 1. **Commands** (content starting with `,`) bypass buffering entirely.
/// 2. **Inactive** messages outside the active-time window are silently dropped.
/// 3. **Active** messages reset the debounce timer and (re-)schedule a batch
///    flush.
/// 4. **Follow-up** (non-active) messages arriving while the window is open
///    schedule a flush after `max_wait_seconds`.
pub struct BufferedMessageHandler {
    handler: mpsc::UnboundedSender<ChannelMessage>,
    inner: Arc<Mutex<BufferInner>>,
    notify: Arc<Notify>,
    active_time_window: Duration,
    max_wait_seconds: Duration,
    debounce_seconds: Duration,
}

struct BufferInner {
    pending: Vec<ChannelMessage>,
    last_active_time: Option<Instant>,
}

impl BufferedMessageHandler {
    /// Create a new buffered handler.
    ///
    /// * `sink` — unbounded sender where the final (batched) message is placed
    ///   for framework processing.
    /// * `active_time_window` — how long (seconds) to consider the channel
    ///   "active" after the last active message.
    /// * `max_wait_seconds` — max time to wait before flushing follow-up
    ///   messages.
    /// * `debounce_seconds` — debounce delay after each active message.
    pub fn new(
        sink: mpsc::UnboundedSender<ChannelMessage>,
        active_time_window: f64,
        max_wait_seconds: f64,
        debounce_seconds: f64,
    ) -> Self {
        Self {
            handler: sink,
            inner: Arc::new(Mutex::new(BufferInner {
                pending: Vec::new(),
                last_active_time: None,
            })),
            notify: Arc::new(Notify::new()),
            active_time_window: Duration::from_secs_f64(active_time_window),
            max_wait_seconds: Duration::from_secs_f64(max_wait_seconds),
            debounce_seconds: Duration::from_secs_f64(debounce_seconds),
        }
    }

    /// Feed a message into the buffer. This mirrors the Python `__call__`.
    pub async fn handle(&self, message: ChannelMessage) {
        let now = Instant::now();

        // Commands bypass buffering entirely.
        if message.content.starts_with('/') {
            let mut inner = self.inner.lock().await;
            let dropped = inner.pending.len();
            inner.pending.clear();
            inner.last_active_time = None;
            drop(inner);
            self.notify.notify_waiters();
            info!(
                session_id = %message.session_id,
                content = %message.content,
                dropped_pending = dropped,
                "session.message received command"
            );
            let _ = self.handler.send(message);
            return;
        }

        let mut inner = self.inner.lock().await;

        // Drop inactive messages outside the active window.
        if !message.is_active {
            let in_window = match inner.last_active_time {
                Some(last) => now.duration_since(last) <= self.active_time_window,
                None => false,
            };
            if !in_window {
                inner.last_active_time = None;
                info!(
                    session_id = %message.session_id,
                    content = %message.content,
                    "session.message received ignored"
                );
                return;
            }
        }

        inner.pending.push(message.clone());

        if message.is_active {
            inner.last_active_time = Some(now);
            info!(
                session_id = %message.session_id,
                content = %message.content,
                "session.message received active"
            );
            drop(inner);
            self.schedule_flush(self.debounce_seconds);
        } else if inner.last_active_time.is_some() {
            info!(
                session_id = %message.session_id,
                content = %message.content,
                "session.receive followup"
            );
            drop(inner);
            self.schedule_flush(self.max_wait_seconds);
        }
    }

    /// Schedule a flush after `delay`. Any previously scheduled flush is
    /// implicitly superseded because we use a single `Notify`.
    fn schedule_flush(&self, delay: Duration) {
        let inner = Arc::clone(&self.inner);
        let notify = Arc::clone(&self.notify);
        let sink = self.handler.clone();

        // Notify wakes any existing waiter, effectively resetting the timer.
        self.notify.notify_waiters();

        tokio::spawn(async move {
            // We race: either the delay elapses, or a new notify arrives
            // (meaning a newer schedule_flush was called).
            tokio::select! {
                () = tokio::time::sleep(delay) => {
                    let mut guard = inner.lock().await;
                    if guard.pending.is_empty() {
                        return;
                    }
                    let batch: Vec<ChannelMessage> = guard.pending.drain(..).collect();
                    drop(guard);

                    let merged = ChannelMessage::from_batch(&batch);
                    if sink.send(merged).is_err() {
                        warn!("buffered handler: sink closed, dropping batch");
                    }
                }
                () = notify.notified() => {
                    // A newer flush was scheduled; this one is superseded.
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::message::{ChannelMessage, MessageKind};
    use tokio::sync::mpsc;

    fn make_msg(content: &str, is_active: bool) -> ChannelMessage {
        ChannelMessage {
            session_id: "test:session".into(),
            channel: "telegram".into(),
            content: content.into(),
            chat_id: "chat".into(),
            is_active,
            kind: MessageKind::Normal,
            context: serde_json::Map::new(),
            media: Vec::new(),
            output_channel: "telegram".into(),
        }
    }

    #[tokio::test]
    async fn test_command_passes_through_immediately() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handler = BufferedMessageHandler::new(tx, 10.0, 10.0, 0.01);

        handler.handle(make_msg("/help", false)).await;

        let received = rx.try_recv().unwrap();
        assert_eq!(received.content, "/help");
    }

    #[tokio::test]
    async fn test_command_clears_pending_buffer() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handler = BufferedMessageHandler::new(tx, 10.0, 10.0, 0.05);

        handler.handle(make_msg("buffered", true)).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        handler.handle(make_msg("/reset", false)).await;

        let received = rx.try_recv().unwrap();
        assert_eq!(received.content, "/reset");

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_inactive_message_outside_window_is_dropped() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handler = BufferedMessageHandler::new(tx, 0.01, 10.0, 0.01);

        // No prior active message, so inactive should be dropped
        handler.handle(make_msg("ignored", false)).await;

        // Give a small window for any potential flush
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_active_message_schedules_flush() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handler = BufferedMessageHandler::new(tx, 10.0, 10.0, 0.01);

        handler.handle(make_msg("hello", true)).await;

        // Wait for debounce to flush
        tokio::time::sleep(Duration::from_millis(100)).await;
        let received = rx.try_recv().unwrap();
        assert_eq!(received.content, "hello");
    }

    #[tokio::test]
    async fn test_multiple_active_messages_debounced_into_batch() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handler = BufferedMessageHandler::new(tx, 10.0, 10.0, 0.05);

        handler.handle(make_msg("msg1", true)).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        handler.handle(make_msg("msg2", true)).await;

        // Wait for debounce
        tokio::time::sleep(Duration::from_millis(200)).await;
        let received = rx.try_recv().unwrap();
        assert_eq!(received.content, "msg1\nmsg2");
    }

    #[tokio::test]
    async fn test_followup_within_active_window_is_buffered() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handler = BufferedMessageHandler::new(tx, 10.0, 0.05, 0.05);

        // Send active message to open window
        handler.handle(make_msg("active", true)).await;
        // Immediately send followup (non-active) within the window
        handler.handle(make_msg("followup", false)).await;

        // Wait for flush
        tokio::time::sleep(Duration::from_millis(200)).await;
        let received = rx.try_recv().unwrap();
        assert!(received.content.contains("active"));
        assert!(received.content.contains("followup"));
    }
}
