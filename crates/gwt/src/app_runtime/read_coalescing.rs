//! Issue #7869 AC-2: one in-flight background read per key.
//!
//! A frontend that resends the same read while the previous one is still
//! running must not queue another worker per resend. Repeats join the read in
//! flight and ask for exactly one rerun, so the work per key stays bounded no
//! matter how many duplicates arrive.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Clone, Debug, Default)]
pub(crate) struct CoalescedReads(Arc<Mutex<HashMap<String, bool>>>);

impl CoalescedReads {
    /// `true` when the caller must start the read. `false` when a read for
    /// `key` is already running; it will run once more after it finishes.
    pub(crate) fn begin(&self, key: &str) -> bool {
        let mut reads = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match reads.get_mut(key) {
            Some(rerun) => {
                *rerun = true;
                false
            }
            None => {
                reads.insert(key.to_string(), false);
                true
            }
        }
    }

    /// Called by the worker after it delivered a result. `true` asks the
    /// worker to read again because a duplicate arrived meanwhile.
    pub(crate) fn finish(&self, key: &str) -> bool {
        let mut reads = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match reads.get_mut(key) {
            Some(rerun) if *rerun => {
                *rerun = false;
                true
            }
            _ => {
                reads.remove(key);
                false
            }
        }
    }

    /// The worker could not start: release the key so a later request reads.
    pub(crate) fn abandon(&self, key: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }

    /// Run `read` until no duplicate arrived during the last run.
    pub(crate) fn run(&self, key: &str, mut read: impl FnMut()) {
        loop {
            read();
            if !self.finish(key) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_during_a_read_collapse_into_one_rerun() {
        let reads = CoalescedReads::default();
        assert!(reads.begin("a"));
        for _ in 0..1_000 {
            assert!(!reads.begin("a"), "a duplicate joins the read in flight");
        }
        assert!(reads.begin("b"), "another key reads independently");
        assert!(reads.finish("a"), "duplicates ask for exactly one rerun");
        assert!(!reads.finish("a"), "no duplicate arrived during the rerun");
        assert!(reads.begin("a"), "a finished key starts a new read");
        reads.abandon("a");
        assert!(reads.begin("a"), "an abandoned key can start again");
    }
}
