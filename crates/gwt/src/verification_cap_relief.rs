//! Issue #4405 AC-2: keep the verification lease holder off the per-agent
//! CPU share.
//!
//! Windows caps every agent Job at `100 / max_active` percent of the machine
//! (SPEC #1921 Phase 86), and `verify.run` runs inside that Job. The lease
//! serializes heavy verification host-wide, so the one run holding it must
//! not crawl at an agent's share while every other window waits behind it.
//! gwt owns the Jobs, so it lifts the cap on the Job that contains the lease
//! holder, only while the lease is held. Nothing ever leaves the agent Job,
//! so pane-close containment is unchanged.

use std::time::Duration;

use gwt_core::index_coordinator::{HeavyHolderKind, HeavyLeaseStatus, IndexCoordinator};
use tokio::time::{interval, MissedTickBehavior};

use crate::PtyWriterRegistry;

const TICK_SECS: u64 = 5;

/// Spawn the relief loop onto the shared tokio runtime.
pub fn spawn(runtime: &tokio::runtime::Runtime, pty_writers: PtyWriterRegistry) {
    drop(runtime.handle().spawn(run(pty_writers)));
}

async fn run(pty_writers: PtyWriterRegistry) {
    let mut ticker = interval(Duration::from_secs(TICK_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let holder = IndexCoordinator::open_default()
            .ok()
            .and_then(|coordinator| coordinator.heavy_lease_status().ok())
            .and_then(|status| verification_holder_pid(&status));
        relieve(&pty_writers, holder);
    }
}

/// The process holding the verification lease, if one does. Index jobs are
/// spawned by gwt itself outside any agent Job, so only verification counts.
fn verification_holder_pid(status: &HeavyLeaseStatus) -> Option<u32> {
    if !status.held || status.expired || status.holder_kind != Some(HeavyHolderKind::Verification) {
        return None;
    }
    status.owner.as_ref().map(|owner| owner.pid)
}

fn relieve(pty_writers: &PtyWriterRegistry, holder: Option<u32>) {
    let Ok(ptys) = pty_writers.read() else {
        return;
    };
    for (window_id, pty) in ptys.iter() {
        match pty.relieve_cap_for_lease_holder(holder) {
            Ok(true) => tracing::info!(
                target: "gwt_verification",
                window_id = %window_id,
                holder_pid = ?holder,
                "verification lease holder CPU cap {}",
                if holder.is_some() { "changed" } else { "restored" }
            ),
            Ok(false) => {}
            Err(error) => tracing::warn!(
                target: "gwt_verification",
                window_id = %window_id,
                %error,
                "verification lease holder CPU cap update failed"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use gwt_core::index_coordinator::OwnerIdentity;

    use super::*;

    #[test]
    fn only_a_live_verification_lease_names_a_holder() {
        let held = HeavyLeaseStatus {
            held: true,
            holder_kind: Some(HeavyHolderKind::Verification),
            owner: Some(OwnerIdentity {
                pid: 21468,
                start_id: "start".to_string(),
            }),
            ..HeavyLeaseStatus::default()
        };
        assert_eq!(verification_holder_pid(&held), Some(21468));

        let index = HeavyLeaseStatus {
            holder_kind: Some(HeavyHolderKind::Index),
            ..held.clone()
        };
        assert_eq!(verification_holder_pid(&index), None);

        let expired = HeavyLeaseStatus {
            expired: true,
            ..held.clone()
        };
        assert_eq!(verification_holder_pid(&expired), None);

        assert_eq!(verification_holder_pid(&HeavyLeaseStatus::default()), None);
    }
}
