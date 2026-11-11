//! GUI residency is an in-memory lifetime state of the existing project owner.
use gwt_core::daemon::DaemonStartIntent;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

const INITIAL_ATTACH_GRACE: Duration = Duration::from_secs(15);
const RECONNECT_GRACE: Duration = Duration::from_secs(15);
pub(crate) const GUI_QUIT_CLOSE_REASON: &str = "gwt-quit";

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResidencySnapshot {
    pub(crate) startup_intent: DaemonStartIntent,
    pub(crate) exit_started: bool,
}
impl Default for ResidencySnapshot {
    fn default() -> Self {
        Self {
            startup_intent: DaemonStartIntent::Standalone,
            exit_started: false,
        }
    }
}

#[derive(Default)]
pub(crate) struct GuiPresence {
    intent: DaemonStartIntent,
    connections: HashSet<u64>,
    deadline: Option<Instant>,
    quit_seen: bool,
}
impl GuiPresence {
    pub(crate) fn new(intent: DaemonStartIntent, now: Instant) -> Self {
        Self {
            intent,
            connections: HashSet::new(),
            deadline: (intent == DaemonStartIntent::GuiManaged).then(|| now + INITIAL_ATTACH_GRACE),
            quit_seen: false,
        }
    }
    pub(crate) fn intent(&self) -> DaemonStartIntent {
        self.intent
    }
    pub(crate) fn attach(&mut self, id: u64) {
        self.connections.insert(id);
        self.deadline = None;
    }
    pub(crate) fn detach(&mut self, id: u64, now: Instant) {
        if self.connections.remove(&id)
            && self.connections.is_empty()
            && self.intent == DaemonStartIntent::GuiManaged
        {
            self.deadline = Some(now + RECONNECT_GRACE);
        }
    }
    pub(crate) fn detach_for_quit(&mut self, id: u64, now: Instant) {
        if self.connections.contains(&id) && self.intent == DaemonStartIntent::GuiManaged {
            // A GUI's background operation can own another lease and disconnect
            // after its main subscriber has delivered explicit Quit.
            self.quit_seen = true;
        }
        self.detach(id, now);
    }
    pub(crate) fn quit_requested(&self) -> bool {
        self.quit_seen
            && self.connections.is_empty()
            && self.intent == DaemonStartIntent::GuiManaged
    }
    pub(crate) fn promote(&mut self) {
        self.intent = DaemonStartIntent::Standalone;
        self.deadline = None;
        self.quit_seen = false;
    }
    pub(crate) fn count(&self) -> usize {
        self.connections.len()
    }
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }
    pub(crate) fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_quit_drains_only_after_the_last_managed_gui_detaches() {
        let now = Instant::now();
        let mut presence = GuiPresence::new(DaemonStartIntent::GuiManaged, now);
        presence.attach(1);
        presence.attach(2);
        presence.detach_for_quit(1, now);
        assert!(!presence.quit_requested());
        presence.detach_for_quit(2, now);
        assert!(presence.quit_requested());

        let mut background_lease = GuiPresence::new(DaemonStartIntent::GuiManaged, now);
        background_lease.attach(1);
        background_lease.attach(2);
        background_lease.detach_for_quit(1, now);
        assert!(!background_lease.quit_requested());
        background_lease.detach(2, now);
        assert!(background_lease.quit_requested());

        let mut standalone = GuiPresence::new(DaemonStartIntent::Standalone, now);
        standalone.attach(1);
        standalone.detach_for_quit(1, now);
        assert!(!standalone.quit_requested());
        let mut disconnected = GuiPresence::new(DaemonStartIntent::GuiManaged, now);
        disconnected.attach(1);
        disconnected.detach(1, now);
        assert!(!disconnected.quit_requested());
    }

    #[test]
    fn gui_presence_cancels_old_grace_and_standalone_never_expires() {
        let now = Instant::now();
        let mut presence = GuiPresence::new(DaemonStartIntent::GuiManaged, now);
        assert!(presence.expired(now + INITIAL_ATTACH_GRACE));
        presence.attach(1);
        presence.attach(2);
        presence.detach(1, now);
        assert!(presence.deadline().is_none());
        presence.detach(2, now);
        assert!(presence.expired(now + RECONNECT_GRACE));
        presence.attach(3);
        assert!(!presence.expired(now + RECONNECT_GRACE));
        presence.detach(3, now);
        presence.promote();
        assert!(!presence.expired(now + INITIAL_ATTACH_GRACE + RECONNECT_GRACE));
        assert_eq!(presence.intent(), DaemonStartIntent::Standalone);
        assert!(GuiPresence::new(DaemonStartIntent::Standalone, now)
            .deadline()
            .is_none());
    }
}
