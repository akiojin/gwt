//! Issues #4405 / #5816: keep every verification lease holder off the per-agent
//! CPU share.
//!
//! Windows caps every agent Job at `100 / max_active` percent of the machine
//! (SPEC #1921 Phase 86), and `verify.run` runs inside that Job. The lease
//! admits heavy verification host-wide, so its holders must not crawl at an
//! agent's share. gwt owns the Jobs, so it disables rate control on every Job
//! containing an active holder, only while a lease is held. Nested ancestor
//! Jobs also contain the PID. Nothing ever leaves the agent Job,
//! so pane-close containment is unchanged.
//!
//! The loop runs on its own thread, never on the GUI event loop or a tokio
//! worker. Each tick probes the canonical pool's full slot set and the legacy
//! model lane, then checks each holder's membership per pane; no git work. A
//! holder PID that exited or was recycled is not found in any Job, and nothing is
//! retried until the next tick. An uncapped Job cannot outlive gwt: every
//! agent Job is kill-on-close, so gwt's exit or crash terminates the tree.

use std::time::{Duration, Instant};

use gwt_core::index_coordinator::{HeavyHolderKind, HeavyLeaseStatus, IndexCoordinator};

use crate::project_runtime::agent_dispatch::PtyWriterRegistry;

const TICK: Duration = Duration::from_secs(5);
/// A tick slower than this is logged, so its cost is never unmeasured.
const SLOW_TICK: Duration = Duration::from_millis(50);

pub(crate) struct VerificationCapRelief {
    stop: std::sync::mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for VerificationCapRelief {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::warn!("verification cap relief worker panicked during owner shutdown");
            }
        }
    }
}

/// The fixed owner retains the existing relief loop until its physical drain.
pub(crate) fn spawn(pty_writers: PtyWriterRegistry) -> Option<VerificationCapRelief> {
    let (stop, stopped) = std::sync::mpsc::channel();
    #[cfg(any(test, feature = "test-gh-guard"))]
    let home = gwt_core::test_support::gwt_home_override();
    let spawned = std::thread::Builder::new()
        .name("gwt-verification-cap-relief".to_string())
        .spawn(move || {
            #[cfg(any(test, feature = "test-gh-guard"))]
            let _home = home
                .as_ref()
                .map(gwt_core::test_support::ScopedGwtHome::set);
            loop {
                match stopped.recv_timeout(TICK) {
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
                let started = Instant::now();
                let holders = verification_holder_pids(verification_statuses());
                relieve(&pty_writers, &holders);
                let elapsed = started.elapsed();
                if elapsed > SLOW_TICK {
                    tracing::warn!(
                        target: "gwt_verification",
                        elapsed_ms = elapsed.as_millis() as u64,
                        "verification cap relief tick was slow"
                    );
                }
            }
        });
    match spawned {
        Ok(worker) => Some(VerificationCapRelief {
            stop,
            worker: Some(worker),
        }),
        Err(error) => {
            tracing::warn!(target: "gwt_verification", %error,
                "verification cap relief thread failed to start");
            None
        }
    }
}

fn verification_statuses() -> Vec<HeavyLeaseStatus> {
    [
        IndexCoordinator::open_default_verification(),
        IndexCoordinator::open_default(),
    ]
    .into_iter()
    .filter_map(Result::ok)
    .filter_map(|coordinator| coordinator.heavy_pool_status().ok())
    .flat_map(|pool| pool.slots.into_iter().map(|slot| slot.status))
    .collect()
}

/// Both lanes can hold verification during upgrades, and every numbered slot
/// can have its own holder. Index jobs never qualify for pane relief.
fn verification_holder_pids(statuses: impl IntoIterator<Item = HeavyLeaseStatus>) -> Vec<u32> {
    let mut pids: Vec<_> = statuses
        .into_iter()
        .filter(|status| {
            status.held
                && !status.expired
                && !status.holder_stale
                && status.holder_kind == Some(HeavyHolderKind::Verification)
        })
        .filter_map(|status| status.owner.map(|owner| owner.pid))
        .collect();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn relieve(pty_writers: &PtyWriterRegistry, holders: &[u32]) {
    let Ok(ptys) = pty_writers.read() else {
        return;
    };
    for (window_id, pty) in ptys.iter() {
        match pty.handle.relieve_cap_for_lease_holders(holders) {
            Ok(true) => tracing::info!(
                target: "gwt_verification",
                window_id = %window_id,
                holder_pids = ?holders,
                "verification lease holder CPU cap {}",
                if holders.is_empty() { "restored" } else { "changed" }
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
    use gwt_core::{
        index_coordinator::{
            verification_coordinator_root, JobAdmission, JobPriority, OwnerIdentity, TargetKey,
        },
        test_support::ScopedGwtHome,
    };

    use super::*;

    #[test]
    fn default_read_observes_canonical_slots_and_the_legacy_root() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let canonical =
            IndexCoordinator::open_verification(verification_coordinator_root(), 2).unwrap();
        let legacy = IndexCoordinator::open_default().unwrap();
        let own = |coordinator: &IndexCoordinator, name: &str| match coordinator
            .request_job(
                &TargetKey::verification("cap-relief", name),
                JobPriority::ManualRebuild,
                Duration::ZERO,
            )
            .unwrap()
        {
            JobAdmission::Owner(owner) => owner,
            JobAdmission::Joined(_) => panic!("the isolated target is not owned"),
        };
        let first_owner = own(&canonical, "slot-a");
        let second_owner = own(&canonical, "slot-b");
        let old_owner = own(&legacy, "legacy");
        let ttl = gwt_core::deadline_budget::HANG_GUARD;
        let first = first_owner
            .acquire_heavy_with_disk_budgets(Duration::ZERO, ttl, &[])
            .unwrap();
        let second = second_owner
            .acquire_heavy_with_disk_budgets(Duration::ZERO, ttl, &[])
            .unwrap();
        let old = old_owner
            .acquire_heavy_with_ttl(Duration::ZERO, ttl)
            .unwrap();
        let mut actual: Vec<_> = verification_statuses()
            .into_iter()
            .filter(|status| status.held)
            .map(|status| status.lease_id.unwrap())
            .collect();
        let mut expected = vec![first.id(), second.id(), old.id()];
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected);
    }

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
        assert_eq!(verification_holder_pids([held.clone()]), vec![21468]);

        let index = HeavyLeaseStatus {
            holder_kind: Some(HeavyHolderKind::Index),
            ..held.clone()
        };
        assert!(verification_holder_pids([index]).is_empty());

        let expired = HeavyLeaseStatus {
            expired: true,
            ..held.clone()
        };
        assert!(verification_holder_pids([expired]).is_empty());

        assert!(verification_holder_pids([HeavyLeaseStatus::default()]).is_empty());
        let another_slot = HeavyLeaseStatus {
            owner: Some(OwnerIdentity {
                pid: 21469,
                start_id: "other".to_string(),
            }),
            ..held.clone()
        };
        assert_eq!(
            verification_holder_pids([held.clone(), another_slot, held]),
            vec![21468, 21469],
            "all slot holders count, even when the representative changes"
        );
    }
}
