//! `ProcessConsoleHub` — ephemeral ring-buffer + broadcast for process console lines.
//!
//! The hub is the single in-memory surface that the Logs window's Process
//! facet reads from. It is owned by `LoggingHandles` (set up in `gwt-core`
//! during `logging::init`) and handed to the GUI runtime so the WebSocket
//! dispatcher can subscribe.
//!
//! Storage strategy (SPEC-1924 FR-038 / NFR-006):
//!
//! - One bounded `VecDeque<ProcessLine>` per [`ProcessKind`]
//! - Default capacity 5000 lines / kind; the oldest line is dropped on overflow
//! - A `tokio::sync::broadcast::Sender<ProcessLine>` with a small capacity for
//!   live forwarding. Slow subscribers may lag and miss intermediate lines —
//!   the ring buffer is the durable replay surface.
//!
//! The hub is `Clone` cheap (it wraps `Arc<Mutex<...>>`) so any caller can
//! grab a handle without coordination.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::broadcast;

use super::kind::ProcessKind;
use super::line::ProcessLine;

/// Process-wide hub installed by `gwt_core::logging::init`. Used by
/// synchronous callers (gh / git / docker / runner wrappers) that do
/// not have a direct handle to `LoggingHandles`.
///
/// `set_global` returns an error if a hub was already installed; this
/// is intentional so that double-init in tests is loud rather than
/// silently leaking.
static GLOBAL_HUB: OnceLock<ProcessConsoleHub> = OnceLock::new();

/// Default capacity per kind. Mirrors SPEC-1924 NFR-006.
pub const DEFAULT_RING_CAPACITY: usize = 5000;

/// Broadcast channel capacity. Slow subscribers may receive `Lagged`
/// errors; the ring buffer is the durable replay surface.
const DEFAULT_BROADCAST_CAPACITY: usize = 1024;

/// Cheap `Clone` handle to the process console.
///
/// All clones share the same ring buffers and broadcast sender.
#[derive(Clone)]
pub struct ProcessConsoleHub {
    inner: Arc<HubInner>,
}

struct HubInner {
    capacity: usize,
    rings: [Mutex<VecDeque<ProcessLine>>; ProcessKind::ALL.len()],
    sender: broadcast::Sender<ProcessLine>,
}

impl ProcessConsoleHub {
    /// Build a hub with the default 5000-line ring buffer per kind.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_RING_CAPACITY)
    }

    /// Build a hub with a custom ring buffer capacity (test-only knob).
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_BROADCAST_CAPACITY);
        let rings = std::array::from_fn(|_| Mutex::new(VecDeque::with_capacity(capacity)));
        Self {
            inner: Arc::new(HubInner {
                capacity,
                rings,
                sender,
            }),
        }
    }

    /// Push a line into the kind's ring buffer and broadcast it.
    ///
    /// The broadcast is best-effort; if there are no live subscribers the
    /// send returns an error which we silently drop. The ring buffer is
    /// always updated so the next subscriber can replay history.
    pub fn push(&self, line: ProcessLine) {
        let idx = kind_index(line.kind);
        if let Ok(mut ring) = self.inner.rings[idx].lock() {
            if ring.len() == self.inner.capacity {
                ring.pop_front();
            }
            ring.push_back(line.clone());
        }
        let _ = self.inner.sender.send(line);
    }

    /// Snapshot the ring buffer for one kind, oldest first.
    pub fn snapshot_kind(&self, kind: ProcessKind) -> Vec<ProcessLine> {
        let idx = kind_index(kind);
        self.inner.rings[idx]
            .lock()
            .map(|ring| ring.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Snapshot every ring buffer merged into a single time-sorted vec.
    pub fn snapshot_all(&self) -> Vec<ProcessLine> {
        let mut out: Vec<ProcessLine> = ProcessKind::ALL
            .iter()
            .flat_map(|kind| self.snapshot_kind(*kind))
            .collect();
        out.sort_by_key(|line| line.timestamp);
        out
    }

    /// Subscribe to the live broadcast stream.
    ///
    /// The returned receiver yields every line pushed AFTER subscription.
    /// Use [`snapshot_kind`](Self::snapshot_kind) or
    /// [`snapshot_all`](Self::snapshot_all) to replay history.
    pub fn subscribe(&self) -> broadcast::Receiver<ProcessLine> {
        self.inner.sender.subscribe()
    }

    /// Ring buffer capacity per kind (test helper).
    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }
}

/// Install the process-wide hub. Called once by `logging::init`.
///
/// Returns `false` when a hub was already installed (which happens in
/// tests that run multiple init sequences in the same process).
pub fn set_global(hub: ProcessConsoleHub) -> bool {
    GLOBAL_HUB.set(hub).is_ok()
}

/// Borrow the process-wide hub if one was installed by
/// [`set_global`]. Synchronous gh / git / docker / runner wrappers
/// fall back to a freshly allocated hub (orphan instance, ring buffer
/// only) when no global was installed — this keeps unit tests of those
/// crates working without bringing the full logging init along.
pub fn global() -> ProcessConsoleHub {
    if let Some(hub) = GLOBAL_HUB.get() {
        hub.clone()
    } else {
        ProcessConsoleHub::new()
    }
}

impl Default for ProcessConsoleHub {
    fn default() -> Self {
        Self::new()
    }
}

fn kind_index(kind: ProcessKind) -> usize {
    ProcessKind::ALL
        .iter()
        .position(|candidate| *candidate == kind)
        .expect("ProcessKind::ALL is exhaustive")
}

#[cfg(test)]
mod tests {
    use super::super::line::ProcessStream;
    use super::*;

    fn line(kind: ProcessKind, message: &str) -> ProcessLine {
        ProcessLine::new(kind, 1, ProcessStream::Stdout, message)
    }

    #[test]
    fn push_and_snapshot_round_trip_per_kind() {
        let hub = ProcessConsoleHub::new();
        hub.push(line(ProcessKind::Gh, "gh one"));
        hub.push(line(ProcessKind::Git, "git one"));
        hub.push(line(ProcessKind::Gh, "gh two"));

        let gh = hub.snapshot_kind(ProcessKind::Gh);
        assert_eq!(gh.len(), 2);
        assert_eq!(gh[0].message, "gh one");
        assert_eq!(gh[1].message, "gh two");

        let git = hub.snapshot_kind(ProcessKind::Git);
        assert_eq!(git.len(), 1);
        assert_eq!(git[0].message, "git one");

        let docker = hub.snapshot_kind(ProcessKind::Docker);
        assert!(docker.is_empty());
    }

    #[test]
    fn ring_buffer_overflows_oldest_first() {
        let hub = ProcessConsoleHub::with_capacity(3);
        for i in 0..5 {
            hub.push(line(ProcessKind::Gh, &format!("line {i}")));
        }
        let gh = hub.snapshot_kind(ProcessKind::Gh);
        assert_eq!(gh.len(), 3);
        assert_eq!(gh[0].message, "line 2");
        assert_eq!(gh[1].message, "line 3");
        assert_eq!(gh[2].message, "line 4");
    }

    #[test]
    fn snapshot_all_is_time_sorted() {
        let hub = ProcessConsoleHub::new();
        let l1 = ProcessLine::new(ProcessKind::Git, 1, ProcessStream::Stdout, "first");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let l2 = ProcessLine::new(ProcessKind::Docker, 2, ProcessStream::Stdout, "second");
        hub.push(l2.clone());
        hub.push(l1.clone());
        let merged = hub.snapshot_all();
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].message, "first");
        assert_eq!(merged[1].message, "second");
    }

    #[tokio::test]
    async fn subscribe_receives_live_lines() {
        let hub = ProcessConsoleHub::new();
        let mut rx = hub.subscribe();
        hub.push(line(ProcessKind::Gh, "live one"));
        let received = rx.recv().await.unwrap();
        assert_eq!(received.message, "live one");
        assert_eq!(received.kind, ProcessKind::Gh);
    }

    #[test]
    fn clone_shares_ring_buffer() {
        let hub = ProcessConsoleHub::new();
        let clone = hub.clone();
        hub.push(line(ProcessKind::Gh, "shared"));
        assert_eq!(clone.snapshot_kind(ProcessKind::Gh).len(), 1);
    }

    #[test]
    fn default_capacity_matches_spec() {
        let hub = ProcessConsoleHub::new();
        assert_eq!(hub.capacity(), DEFAULT_RING_CAPACITY);
    }
}
