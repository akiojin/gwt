//! Daemon-side broadcast hub used by Phase H1+ runtime ownership migration.
//!
//! [`BroadcastHub`] keeps a `tokio::sync::broadcast` channel per logical
//! event channel ("board", "runtime-status", ...). When a per-connection
//! handler observes a [`ClientFrame::Subscribe`], it asks the hub for a
//! receiver. Daemon-side code paths (Board projection writer, runtime
//! status aggregator, hook event router) call [`BroadcastHub::publish`]
//! to fan a single payload out to all subscribers.
//!
//! The hub is intentionally small: one mutex around a `HashMap<String,
//! broadcast::Sender<DaemonFrame>>`. Phase H1 will graft the actual
//! handler-to-publish wiring; this module ships the storage primitive
//! plus tests so future PRs can layer real ownership migrations on top
//! without re-deriving the synchronization boundary.

#![cfg(unix)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use gwt_core::daemon::DaemonFrame;
use tokio::sync::broadcast;

/// Default per-channel capacity. 64 is enough headroom for a burst of
/// Board projection events without forcing slow subscribers to drop
/// frames (subscribers that do fall behind get a `RecvError::Lagged`).
pub(super) const DEFAULT_CHANNEL_CAPACITY: usize = 64;

/// Multi-channel broadcast registry shared by all per-connection tasks.
///
/// Cheap to clone via [`Arc`] — internal mutation is guarded by a
/// single short-lived [`Mutex`]. Channels are created on-demand the
/// first time `subscribe` or `publish` references them.
#[derive(Clone, Default)]
pub(crate) struct BroadcastHub {
    channels: Arc<Mutex<HashMap<String, broadcast::Sender<DaemonFrame>>>>,
}

impl BroadcastHub {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Return a subscriber receiver for `channel`, creating the channel
    /// if it does not exist yet.
    pub(crate) fn subscribe(&self, channel: &str) -> broadcast::Receiver<DaemonFrame> {
        let mut guard = self.channels.lock().expect("BroadcastHub mutex poisoned");
        let sender = guard
            .entry(channel.to_string())
            .or_insert_with(|| broadcast::channel(DEFAULT_CHANNEL_CAPACITY).0);
        sender.subscribe()
    }

    /// Publish `frame` to every subscriber currently registered on
    /// `channel`. Returns the number of receivers the frame was queued
    /// for (zero is a successful no-op when nobody is listening).
    ///
    /// Phase H1 GREEN will call this from real domain handlers (Board
    /// projection writer, runtime status aggregator). Until then the
    /// only callers live in tests, so the lib-only dead-code lint is
    /// suppressed.
    ///
    /// Critically, the global `channels` mutex is released *before*
    /// `sender.send` runs. `tokio::sync::broadcast::Sender::send`
    /// clones the payload for every subscriber, so retaining the lock
    /// across that call would block unrelated subscribe / publish
    /// activity on other channels for the duration of a (potentially
    /// large) fan-out.
    #[allow(dead_code)]
    pub(crate) fn publish(&self, channel: &str, frame: DaemonFrame) -> usize {
        let sender = {
            let guard = self.channels.lock().expect("BroadcastHub mutex poisoned");
            guard.get(channel).cloned()
        };
        match sender {
            Some(sender) => sender.send(frame).unwrap_or(0),
            None => 0,
        }
    }

    /// Number of distinct channels currently tracked. Used by the
    /// daemon's status snapshot frame and by tests.
    pub(crate) fn channel_count(&self) -> usize {
        self.channels
            .lock()
            .expect("BroadcastHub mutex poisoned")
            .len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gwt_core::daemon::DaemonFrame;
    use serde_json::json;
    use tokio::sync::broadcast::error::TryRecvError;

    use super::BroadcastHub;

    #[test]
    fn subscribe_creates_channel_lazily() {
        let hub = BroadcastHub::new();
        assert_eq!(hub.channel_count(), 0);
        let _rx = hub.subscribe("board");
        assert_eq!(hub.channel_count(), 1);
        let _rx2 = hub.subscribe("runtime-status");
        assert_eq!(hub.channel_count(), 2);
        let _rx3 = hub.subscribe("board");
        assert_eq!(
            hub.channel_count(),
            2,
            "second subscribe to existing channel must reuse the sender"
        );
    }

    #[test]
    fn publish_to_unknown_channel_returns_zero() {
        let hub = BroadcastHub::new();
        let queued = hub.publish("never-subscribed", DaemonFrame::Ack);
        assert_eq!(queued, 0);
    }

    #[tokio::test]
    async fn publish_fans_out_to_all_subscribers() {
        let hub = BroadcastHub::new();
        let mut rx_a = hub.subscribe("board");
        let mut rx_b = hub.subscribe("board");

        let frame = DaemonFrame::Event {
            channel: "board".into(),
            payload: json!({"entries": 5}),
        };
        let queued = hub.publish("board", frame.clone());
        assert_eq!(queued, 2);

        let received_a = tokio::time::timeout(Duration::from_millis(50), rx_a.recv())
            .await
            .expect("rx_a timed out")
            .expect("rx_a recv");
        assert_eq!(received_a, frame);

        let received_b = tokio::time::timeout(Duration::from_millis(50), rx_b.recv())
            .await
            .expect("rx_b timed out")
            .expect("rx_b recv");
        assert_eq!(received_b, frame);
    }

    #[tokio::test]
    async fn publish_skips_subscribers_on_other_channels() {
        let hub = BroadcastHub::new();
        let mut rx_board = hub.subscribe("board");
        let mut rx_runtime = hub.subscribe("runtime-status");

        let board_frame = DaemonFrame::Event {
            channel: "board".into(),
            payload: json!({"entries": 1}),
        };
        let queued = hub.publish("board", board_frame.clone());
        assert_eq!(queued, 1);

        let received = rx_board.recv().await.expect("board recv");
        assert_eq!(received, board_frame);

        // The runtime-status receiver must NOT have observed the board
        // frame. `try_recv` is non-blocking; an empty channel returns
        // `TryRecvError::Empty`.
        match rx_runtime.try_recv() {
            Err(TryRecvError::Empty) => {}
            other => panic!("expected runtime-status to be empty, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn hub_is_cheaply_cloneable_and_shares_state() {
        let hub = BroadcastHub::new();
        let hub_clone = hub.clone();
        let mut rx = hub.subscribe("board");

        // Publishing through the clone reaches the original's receiver.
        let frame = DaemonFrame::Ack;
        let queued = hub_clone.publish("board", frame.clone());
        assert_eq!(queued, 1);

        let received = rx.recv().await.expect("recv");
        assert_eq!(received, frame);
    }
}
