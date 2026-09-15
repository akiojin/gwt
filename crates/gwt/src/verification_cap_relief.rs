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
//!
//! The loop runs on its own thread, never on the GUI event loop or a tokio
//! worker. Each tick is one lease probe (a kernel lock test plus the lease
//! ticket) and one `IsProcessInJob` per pane; no git work. A holder pid that
//! exited or was recycled simply is not found in any Job, and nothing is
//! retried until the next tick. An uncapped Job cannot outlive gwt: every
//! agent Job is kill-on-close, so gwt's exit or crash terminates the tree.

use std::time::{Duration, Instant};

use gwt_core::index_coordinator::{HeavyHolderKind, HeavyLeaseStatus, IndexCoordinator};

use crate::PtyWriterRegistry;

const TICK: Duration = Duration::from_secs(5);
/// A tick slower than this is logged, so its cost is never unmeasured.
const SLOW_TICK: Duration = Duration::from_millis(50);

/// Spawn the relief loop on a dedicated thread.
pub fn spawn(pty_writers: PtyWriterRegistry) {
    let spawned = std::thread::Builder::new()
        .name("gwt-verification-cap-relief".to_string())
        .spawn(move || loop {
            std::thread::sleep(TICK);
            let started = Instant::now();
            let holder = IndexCoordinator::open_default()
                .ok()
                .and_then(|coordinator| coordinator.heavy_lease_status().ok())
                .and_then(|status| verification_holder_pid(&status));
            relieve(&pty_writers, holder);
            let elapsed = started.elapsed();
            if elapsed > SLOW_TICK {
                tracing::warn!(
                    target: "gwt_verification",
                    elapsed_ms = elapsed.as_millis() as u64,
                    "verification cap relief tick was slow"
                );
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(
            target: "gwt_verification",
            %error,
            "verification cap relief thread failed to start"
        );
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
