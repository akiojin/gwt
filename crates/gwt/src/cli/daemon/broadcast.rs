//! Daemon-side event hub used by Phase H1+ runtime ownership migration.
//!
//! [`BroadcastHub`] keeps a `tokio::sync::broadcast` channel per logical
//! event channel ("board", "runtime-status", ...). When a per-connection
//! handler observes a [`gwt_core::daemon::ClientFrame::Subscribe`], it asks
//! the hub for a receiver. Daemon-side code paths (Board projection writer,
//! runtime status aggregator, hook event router) call
//! [`BroadcastHub::publish`] to fan a single payload out to all subscribers.
//! Issue Monitor controls are commands rather than notifications, so they use
//! a separate single-worker queue. A bounded admission semaphore covers both
//! queued and in-flight commands; acceptance alone is not an ACK. Each caller
//! receives a completion receipt that resolves only after the worker commits
//! the corresponding durable transaction or rejects it explicitly.
//!
//! The notification registry remains one mutex around a `HashMap<String,
//! broadcast::Sender<DaemonFrame>>`; the single-consumer control queue is an
//! independent field. Phase H1 wired the Board projection writer through
//! `daemon_publisher::publish_event` to `BroadcastHub::publish("board", ...)`.
//! Phase H2-H4 layer `runtime-output` / `runtime-status` / `runtime-hook` /
//! `launch-complete` notifications onto that broadcast primitive without
//! changing the control queue's lossless contract.

#![cfg(unix)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use gwt_core::daemon::DaemonFrame;
use tokio::sync::{broadcast, mpsc, oneshot, watch, OwnedSemaphorePermit, Semaphore};

/// Default per-channel capacity. 64 is enough headroom for a burst of
/// Board projection events without forcing slow subscribers to drop
/// frames (subscribers that do fall behind get a `RecvError::Lagged`).
pub(super) const DEFAULT_CHANNEL_CAPACITY: usize = 64;

/// Issue Monitor controls are commands whose daemon ACK confirms the exact
/// control transaction was durably committed. Keep them off the lossy
/// broadcast rings.
const ISSUE_MONITOR_CONTROL_CAPACITY: usize = 64;

struct IssueMonitorControlQueue {
    sender: mpsc::Sender<IssueMonitorControlRequest>,
    receiver: Mutex<Option<mpsc::Receiver<IssueMonitorControlRequest>>>,
    state: watch::Sender<IssueMonitorControlState>,
    admission: Arc<Semaphore>,
}

impl IssueMonitorControlQueue {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel(ISSUE_MONITOR_CONTROL_CAPACITY);
        Self {
            sender,
            receiver: Mutex::new(Some(receiver)),
            state: watch::channel(IssueMonitorControlState::Starting).0,
            admission: Arc::new(Semaphore::new(ISSUE_MONITOR_CONTROL_CAPACITY)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IssueMonitorControlState {
    Starting,
    Ready,
    RecoveryBlocked,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IssueMonitorControlQueueError {
    RecoveryBlocked,
    Closed,
    Rejected,
    Busy,
}

impl IssueMonitorControlQueueError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::RecoveryBlocked => {
                crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_RECOVERY_BLOCKED_ERROR
            }
            Self::Closed => crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CLOSED_ERROR,
            Self::Rejected => crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_REJECTED_ERROR,
            Self::Busy => crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_BUSY_ERROR,
        }
    }
}

pub(crate) struct IssueMonitorControlCompletion {
    sender: Option<oneshot::Sender<Result<(), IssueMonitorControlQueueError>>>,
    _admission: OwnedSemaphorePermit,
}

impl IssueMonitorControlCompletion {
    pub(crate) fn commit(mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Ok(()));
        }
    }

    pub(crate) fn reject(mut self, error: IssueMonitorControlQueueError) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Err(error));
        }
    }
}

pub(crate) struct IssueMonitorControlRequest {
    frame: DaemonFrame,
    completion: IssueMonitorControlCompletion,
}

impl IssueMonitorControlRequest {
    #[cfg(test)]
    pub(crate) fn frame(&self) -> &DaemonFrame {
        &self.frame
    }

    pub(crate) fn into_parts(self) -> (DaemonFrame, IssueMonitorControlCompletion) {
        (self.frame, self.completion)
    }

    #[cfg(test)]
    pub(crate) fn commit(self) {
        self.completion.commit();
    }

    #[cfg(test)]
    fn reject_closed(self) {
        self.completion
            .reject(IssueMonitorControlQueueError::Closed);
    }
}

/// Runtime notification registry plus the dedicated Issue Monitor command
/// queue, shared by all per-connection tasks.
///
/// Cheap to clone via [`Arc`] — internal mutation is guarded by a
/// single short-lived [`Mutex`]. Channels are created on-demand the
/// first time `subscribe` or `publish` references them.
#[derive(Clone)]
pub struct BroadcastHub {
    channels: Arc<Mutex<HashMap<String, broadcast::Sender<DaemonFrame>>>>,
    issue_monitor_controls: Arc<IssueMonitorControlQueue>,
}

impl Default for BroadcastHub {
    fn default() -> Self {
        Self {
            channels: Arc::new(Mutex::new(HashMap::new())),
            issue_monitor_controls: Arc::new(IssueMonitorControlQueue::new()),
        }
    }
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

    /// Claim the sole lossless Issue Monitor control receiver. Until the
    /// worker calls this, publishers fail closed instead of ACKing a command
    /// that no active worker has accepted.
    pub(crate) fn take_issue_monitor_control_receiver(
        &self,
    ) -> Option<mpsc::Receiver<IssueMonitorControlRequest>> {
        if *self.issue_monitor_controls.state.borrow() != IssueMonitorControlState::Starting {
            return None;
        }
        let receiver = self
            .issue_monitor_controls
            .receiver
            .lock()
            .expect("Issue Monitor control receiver mutex poisoned")
            .take()?;
        self.issue_monitor_controls
            .state
            .send_replace(IssueMonitorControlState::Ready);
        Some(receiver)
    }

    pub(crate) fn mark_issue_monitor_control_recovery_blocked(&self) {
        self.issue_monitor_controls
            .receiver
            .lock()
            .expect("Issue Monitor control receiver mutex poisoned")
            .take();
        self.issue_monitor_controls
            .state
            .send_replace(IssueMonitorControlState::RecoveryBlocked);
    }

    pub(crate) fn close_issue_monitor_controls(&self) {
        self.issue_monitor_controls
            .state
            .send_replace(IssueMonitorControlState::Closed);
    }

    /// Admit one command and await its durable completion receipt. Successful
    /// return is the ACK boundary: the worker has committed the exact control
    /// transaction. Full admission rejects immediately instead of retaining an
    /// unbounded connection/task waiter outside the mpsc queue.
    pub(crate) async fn publish_issue_monitor_control(
        &self,
        frame: DaemonFrame,
    ) -> Result<(), IssueMonitorControlQueueError> {
        let receipt = self.enqueue_issue_monitor_control(frame).await?;
        receipt
            .await
            .unwrap_or(Err(IssueMonitorControlQueueError::Closed))
    }

    pub(crate) async fn enqueue_issue_monitor_control(
        &self,
        frame: DaemonFrame,
    ) -> Result<
        oneshot::Receiver<Result<(), IssueMonitorControlQueueError>>,
        IssueMonitorControlQueueError,
    > {
        let mut state = self.issue_monitor_controls.state.subscribe();
        match *state.borrow() {
            IssueMonitorControlState::RecoveryBlocked => {
                return Err(IssueMonitorControlQueueError::RecoveryBlocked);
            }
            IssueMonitorControlState::Closed => {
                return Err(IssueMonitorControlQueueError::Closed);
            }
            IssueMonitorControlState::Starting | IssueMonitorControlState::Ready => {}
        }
        let admission = Arc::clone(&self.issue_monitor_controls.admission)
            .try_acquire_owned()
            .map_err(|_| IssueMonitorControlQueueError::Busy)?;
        loop {
            let current = *state.borrow_and_update();
            match current {
                IssueMonitorControlState::Starting => {
                    state
                        .changed()
                        .await
                        .map_err(|_| IssueMonitorControlQueueError::Closed)?;
                }
                IssueMonitorControlState::Ready => break,
                IssueMonitorControlState::RecoveryBlocked => {
                    return Err(IssueMonitorControlQueueError::RecoveryBlocked);
                }
                IssueMonitorControlState::Closed => {
                    return Err(IssueMonitorControlQueueError::Closed);
                }
            }
        }
        let (completion, receipt) = oneshot::channel();
        self.issue_monitor_controls
            .sender
            .send(IssueMonitorControlRequest {
                frame,
                completion: IssueMonitorControlCompletion {
                    sender: Some(completion),
                    _admission: admission,
                },
            })
            .await
            .map_err(|_| IssueMonitorControlQueueError::Closed)?;
        Ok(receipt)
    }

    /// Publish `frame` to every subscriber currently registered on
    /// `channel`. Returns the number of receivers the frame was queued
    /// for (zero is a successful no-op when nobody is listening).
    ///
    /// Called from `server::handle_connection` when a
    /// `ClientFrame::Publish` arrives, so the daemon fans the payload
    /// out to every subscribed connection on `channel`.
    ///
    /// Critically, the global `channels` mutex is released *before*
    /// `sender.send` runs. `tokio::sync::broadcast::Sender::send`
    /// clones the payload for every subscriber, so retaining the lock
    /// across that call would block unrelated subscribe / publish
    /// activity on other channels for the duration of a (potentially
    /// large) fan-out.
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

    pub(crate) fn receiver_count(&self, channel: &str) -> usize {
        let guard = self.channels.lock().expect("BroadcastHub mutex poisoned");
        guard
            .get(channel)
            .map(tokio::sync::broadcast::Sender::receiver_count)
            .unwrap_or(0)
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
    use gwt_core::daemon::DaemonFrame;
    use serde_json::json;
    use tokio::sync::broadcast::error::TryRecvError;

    use super::{
        BroadcastHub, IssueMonitorControlQueueError, DEFAULT_CHANNEL_CAPACITY,
        ISSUE_MONITOR_CONTROL_CAPACITY,
    };

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

    #[test]
    fn publish_fans_out_to_all_subscribers() {
        let hub = BroadcastHub::new();
        let mut rx_a = hub.subscribe("board");
        let mut rx_b = hub.subscribe("board");

        let frame = DaemonFrame::Event {
            channel: "board".into(),
            payload: json!({"entries": 5}),
        };
        let queued = hub.publish("board", frame.clone());
        assert_eq!(queued, 2);

        let received_a = rx_a.try_recv().expect("rx_a frame was queued by publish");
        assert_eq!(received_a, frame);

        let received_b = rx_b.try_recv().expect("rx_b frame was queued by publish");
        assert_eq!(received_b, frame);
    }

    #[test]
    fn publish_skips_subscribers_on_other_channels() {
        let hub = BroadcastHub::new();
        let mut rx_board = hub.subscribe("board");
        let mut rx_runtime = hub.subscribe("runtime-status");

        let board_frame = DaemonFrame::Event {
            channel: "board".into(),
            payload: json!({"entries": 1}),
        };
        let queued = hub.publish("board", board_frame.clone());
        assert_eq!(queued, 1);

        let received = rx_board
            .try_recv()
            .expect("board frame was queued by publish");
        assert_eq!(received, board_frame);

        // The runtime-status receiver must NOT have observed the board
        // frame. `try_recv` is non-blocking; an empty channel returns
        // `TryRecvError::Empty`.
        match rx_runtime.try_recv() {
            Err(TryRecvError::Empty) => {}
            other => panic!("expected runtime-status to be empty, got: {other:?}"),
        }
    }

    #[test]
    fn hub_is_cheaply_cloneable_and_shares_state() {
        let hub = BroadcastHub::new();
        let hub_clone = hub.clone();
        let mut rx = hub.subscribe("board");

        // Publishing through the clone reaches the original's receiver.
        let frame = DaemonFrame::Ack;
        let queued = hub_clone.publish("board", frame.clone());
        assert_eq!(queued, 1);

        let received = rx
            .try_recv()
            .expect("clone publish queued the shared frame");
        assert_eq!(received, frame);
    }

    #[test]
    fn slow_subscriber_recovers_after_lag() {
        // A subscriber that does not drain fast enough loses old
        // frames once the publisher pushes more than
        // `DEFAULT_CHANNEL_CAPACITY` items. The first post-lag
        // `try_recv()` returns `TryRecvError::Lagged(skipped)`, but the
        // subscription is NOT closed — the next `try_recv()` returns the
        // newest still-buffered frame.
        //
        // The daemon's per-channel forwarder relies on this contract:
        // it must distinguish `Lagged` (recoverable, log + continue)
        // from `Closed` (terminal, break loop). A naive `match
        // result { Err(_) => break }` would silently kill a slow
        // subscriber's entire subscription on the very first lag,
        // not just the dropped frames. This test pins the recovery
        // semantic so the forwarder's `Lagged` branch stays correct.
        let hub = BroadcastHub::new();
        let mut rx = hub.subscribe("board");

        // Push capacity + a few extras with no draining, forcing the
        // broadcast channel to overwrite the oldest slots.
        let overflow = DEFAULT_CHANNEL_CAPACITY + 4;
        for i in 0..overflow {
            hub.publish(
                "board",
                DaemonFrame::Event {
                    channel: "board".into(),
                    payload: json!({"seq": i}),
                },
            );
        }

        // First receive after overflow must surface the lag signal,
        // not silently swallow or fail the subscription.
        match rx.try_recv() {
            Err(TryRecvError::Lagged(skipped)) => {
                assert!(
                    skipped > 0,
                    "expected a positive skipped count from TryRecvError::Lagged"
                );
            }
            other => panic!("expected TryRecvError::Lagged, got {other:?}"),
        }

        // The subscription is still alive: the next try_recv returns a
        // real frame. Confirms the forwarder's "log + continue"
        // strategy will keep delivering the newest events.
        match rx.try_recv() {
            Ok(DaemonFrame::Event { payload, .. }) => {
                let seq = payload["seq"].as_u64().expect("seq u64");
                assert!(
                    seq < overflow as u64,
                    "post-lag frame should still be from the recent burst"
                );
            }
            other => panic!("expected event frame after lag recovery, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn issue_monitor_control_admission_is_bounded_until_receipt_resolves() {
        let hub = BroadcastHub::new();
        let mut receiver = hub
            .take_issue_monitor_control_receiver()
            .expect("single issue monitor worker receiver");
        let mut receipts = Vec::with_capacity(ISSUE_MONITOR_CONTROL_CAPACITY);
        for seq in 0..ISSUE_MONITOR_CONTROL_CAPACITY {
            receipts.push(
                hub.enqueue_issue_monitor_control(DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload: json!({"seq": seq}),
                })
                .await
                .expect("admission remains available"),
            );
        }
        let overflow = hub.enqueue_issue_monitor_control(DaemonFrame::Ack).await;
        assert!(matches!(overflow, Err(IssueMonitorControlQueueError::Busy)));

        let first = receiver
            .try_recv()
            .expect("first admitted request was already queued");
        assert!(matches!(
            first.frame(),
            DaemonFrame::Event { payload, .. } if *payload == json!({"seq": 0})
        ));
        first.commit();
        receipts
            .remove(0)
            .await
            .expect("first receipt sender remains live")
            .expect("first request commits");
        let final_receipt = hub
            .enqueue_issue_monitor_control(DaemonFrame::Event {
                channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
                payload: json!({"seq": ISSUE_MONITOR_CONTROL_CAPACITY}),
            })
            .await
            .expect("resolved receipt releases one admission");

        for (expected, receipt) in (1..ISSUE_MONITOR_CONTROL_CAPACITY).zip(receipts) {
            let request = receiver
                .try_recv()
                .expect("admitted control request was already queued");
            assert!(matches!(
                request.frame(),
                DaemonFrame::Event { payload, .. }
                    if *payload == json!({"seq": expected})
            ));
            request.commit();
            receipt
                .await
                .expect("receipt sender remains live")
                .expect("request commits");
        }
        let final_request = receiver
            .try_recv()
            .expect("final admitted request was already queued");
        assert!(matches!(
            final_request.frame(),
            DaemonFrame::Event { payload, .. }
                if *payload == json!({"seq": ISSUE_MONITOR_CONTROL_CAPACITY})
        ));
        final_request.commit();
        final_receipt
            .await
            .expect("final receipt sender remains live")
            .expect("final request commits");
    }

    #[tokio::test]
    async fn issue_monitor_control_queue_requires_one_live_worker_and_commit() {
        let hub = BroadcastHub::new();
        let frame = DaemonFrame::Event {
            channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
            payload: json!({"enabled": true}),
        };
        let mut receiver = hub
            .take_issue_monitor_control_receiver()
            .expect("worker claims the receiver once");
        assert!(hub.take_issue_monitor_control_receiver().is_none());
        let publish = tokio::spawn({
            let hub = hub.clone();
            let frame = frame.clone();
            async move { hub.publish_issue_monitor_control(frame).await }
        });
        let request = receiver.recv().await.expect("ready worker receives frame");
        assert_eq!(request.frame(), &frame);
        assert!(!publish.is_finished(), "commit receipt is still pending");
        request.commit();
        publish
            .await
            .expect("publisher joins")
            .expect("ready worker commits the frame");

        drop(receiver);
        assert!(
            hub.publish_issue_monitor_control(frame).await.is_err(),
            "closed worker receiver rejects the frame"
        );
    }

    #[tokio::test]
    async fn issue_monitor_control_starting_waits_for_ready_and_commit_receipt() {
        let hub = BroadcastHub::new();
        let frame = DaemonFrame::Event {
            channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
            payload: json!({"enabled": false}),
        };
        let publish = tokio::spawn({
            let hub = hub.clone();
            let frame = frame.clone();
            async move { hub.publish_issue_monitor_control(frame).await }
        });

        tokio::task::yield_now().await;
        assert!(
            !publish.is_finished(),
            "Starting waits for worker initialization instead of forcing fallback"
        );

        let mut receiver = hub
            .take_issue_monitor_control_receiver()
            .expect("valid startup claims receiver and becomes Ready");
        let request = receiver.recv().await.expect("worker receives request");
        assert_eq!(request.frame(), &frame);
        assert!(
            !publish.is_finished(),
            "mpsc enqueue alone must not complete the publisher"
        );
        request.commit();

        assert!(publish.await.expect("publisher joins").is_ok());
    }

    #[tokio::test]
    async fn issue_monitor_control_starting_resolves_to_recovery_blocked_or_closed() {
        let recovery_hub = BroadcastHub::new();
        let recovery_publish = tokio::spawn({
            let hub = recovery_hub.clone();
            async move { hub.publish_issue_monitor_control(DaemonFrame::Ack).await }
        });
        tokio::task::yield_now().await;
        recovery_hub.mark_issue_monitor_control_recovery_blocked();
        let recovery_error = recovery_publish
            .await
            .expect("recovery publisher joins")
            .expect_err("recovery blocks controls");
        assert_eq!(
            recovery_error.message(),
            crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_RECOVERY_BLOCKED_ERROR
        );
        assert_eq!(
            recovery_hub
                .publish_issue_monitor_control(DaemonFrame::Ack)
                .await
                .expect_err("terminal recovery state rejects immediately")
                .message(),
            crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_RECOVERY_BLOCKED_ERROR
        );

        let closed_hub = BroadcastHub::new();
        let closed_publish = tokio::spawn({
            let hub = closed_hub.clone();
            async move { hub.publish_issue_monitor_control(DaemonFrame::Ack).await }
        });
        tokio::task::yield_now().await;
        closed_hub.close_issue_monitor_controls();
        assert!(closed_publish
            .await
            .expect("closed publisher joins")
            .is_err());
    }

    #[tokio::test]
    async fn issue_monitor_control_shutdown_rejects_uncommitted_receipts() {
        let hub = BroadcastHub::new();
        let mut receiver = hub
            .take_issue_monitor_control_receiver()
            .expect("worker receiver");
        let first = tokio::spawn({
            let hub = hub.clone();
            async move { hub.publish_issue_monitor_control(DaemonFrame::Ack).await }
        });
        let second = tokio::spawn({
            let hub = hub.clone();
            async move { hub.publish_issue_monitor_control(DaemonFrame::Ack).await }
        });

        let first_request = receiver.recv().await.expect("first queued request");
        let second_request = receiver.recv().await.expect("second queued request");
        receiver.close();
        hub.close_issue_monitor_controls();
        first_request.reject_closed();
        second_request.reject_closed();

        assert!(first.await.expect("first publisher joins").is_err());
        assert!(second.await.expect("second publisher joins").is_err());
    }
}
