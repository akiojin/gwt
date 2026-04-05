use tokio::sync::mpsc;

use crate::Notification;

/// Default bounded channel capacity for the notification bus.
const CHANNEL_CAPACITY: usize = 256;

/// Sender half of the notification bus.
#[derive(Clone, Debug)]
pub struct NotificationBus {
    tx: mpsc::Sender<Notification>,
}

/// Receiver half of the notification bus.
#[derive(Debug)]
pub struct NotificationReceiver {
    rx: mpsc::Receiver<Notification>,
}

impl NotificationBus {
    /// Create a new bus pair (sender, receiver).
    pub fn new() -> (Self, NotificationReceiver) {
        Self::with_capacity(CHANNEL_CAPACITY)
    }

    /// Create a new bus pair with a custom bounded capacity.
    pub fn with_capacity(capacity: usize) -> (Self, NotificationReceiver) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (Self { tx }, NotificationReceiver { rx })
    }

    /// Send a notification (non-blocking best-effort).
    /// Returns `true` if sent, `false` if the channel is full or closed.
    pub fn send(&self, notification: Notification) -> bool {
        self.tx.try_send(notification).is_ok()
    }
}

impl NotificationReceiver {
    /// Try to receive a notification without blocking.
    pub fn try_recv(&mut self) -> Option<Notification> {
        self.rx.try_recv().ok()
    }

    /// Drain all currently queued notifications without blocking.
    pub fn drain(&mut self) -> Vec<Notification> {
        let mut notifications = Vec::new();
        while let Some(notification) = self.try_recv() {
            notifications.push(notification);
        }
        notifications
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;

    #[test]
    fn send_and_receive() {
        let (bus, mut rx) = NotificationBus::new();
        let n = Notification::new(Severity::Info, "test", "hello");
        assert!(bus.send(n));
        let received = rx.try_recv();
        assert!(received.is_some());
        assert_eq!(received.unwrap().message, "hello");
    }

    #[test]
    fn try_recv_empty_returns_none() {
        let (_bus, mut rx) = NotificationBus::new();
        assert!(rx.try_recv().is_none());
    }

    #[test]
    fn drain_returns_all_pending_notifications() {
        let (bus, mut rx) = NotificationBus::new();
        bus.send(Notification::new(Severity::Info, "core", "one"));
        bus.send(Notification::new(Severity::Warn, "core", "two"));

        let drained = rx.drain();

        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].message, "one");
        assert_eq!(drained[1].message, "two");
        assert!(rx.try_recv().is_none());
    }

    #[test]
    fn send_multiple() {
        let (bus, mut rx) = NotificationBus::new();
        for i in 0..5 {
            bus.send(Notification::new(
                Severity::Debug,
                "test",
                format!("msg-{i}"),
            ));
        }
        let mut count = 0;
        while rx.try_recv().is_some() {
            count += 1;
        }
        assert_eq!(count, 5);
    }

    #[test]
    fn bus_is_cloneable() {
        let (bus, mut rx) = NotificationBus::new();
        let bus2 = bus.clone();
        bus.send(Notification::new(Severity::Info, "a", "from-original"));
        bus2.send(Notification::new(Severity::Info, "b", "from-clone"));
        assert!(rx.try_recv().is_some());
        assert!(rx.try_recv().is_some());
    }

    #[test]
    fn dropped_sender_closes_channel() {
        let (bus, mut rx) = NotificationBus::new();
        bus.send(Notification::new(Severity::Info, "test", "last"));
        drop(bus);
        // Can still drain buffered messages
        assert!(rx.try_recv().is_some());
        assert!(rx.try_recv().is_none());
    }

    #[test]
    fn channel_bounded_at_capacity() {
        let (bus, _rx) = NotificationBus::with_capacity(3);
        let mut sent = 0;
        for _ in 0..10 {
            if bus.send(Notification::new(Severity::Debug, "test", "fill")) {
                sent += 1;
            }
        }
        // Should cap at the configured capacity
        assert_eq!(sent, 3);
    }

    #[test]
    fn new_uses_default_capacity() {
        let (bus, _rx) = NotificationBus::new();
        let mut sent = 0;
        for _ in 0..300 {
            if bus.send(Notification::new(Severity::Debug, "test", "fill")) {
                sent += 1;
            } else {
                break;
            }
        }

        assert_eq!(sent, CHANNEL_CAPACITY);
    }
}
