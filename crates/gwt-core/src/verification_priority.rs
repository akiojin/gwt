//! Where a `verify.run` child process is launched from, and why (Issue #4409).
//!
//! `gwtd` runs inside the agent's PTY process tree, so every verification
//! command it spawns inherits the agent launch policy's degraded priority
//! (SPEC #1921 Phase 86 gives the agent tree `nice 10`). A non-privileged
//! process cannot lower its own nice value, so the inherited penalty cannot be
//! undone from inside the tree — #4405 measured a 54x slowdown and could only
//! report the problem. The fix is to stop inheriting: launch the workload from
//! a process that is already at baseline priority.
//!
//! The only such process gwt already owns is the daemon under the GUI. This
//! module holds the placement decision itself, kept free of I/O so the policy
//! is decided in one readable place instead of being re-derived at each call
//! site.

/// Priority of the process that is about to launch a verification command.
///
/// Agent-tree membership is observed through the *inherited nice value* rather
/// than an environment marker. A marker would be inherited by the verification
/// child too, so a nested `verify.run` would keep reporting itself as trapped
/// long after it escaped. The nice value is the thing that actually harms the
/// workload, and it stops being degraded exactly when the escape worked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LauncherPriority {
    /// Current nice value, or `None` on platforms with no nice concept
    /// (Windows, where #4405 solves the same problem through job objects).
    pub nice: Option<i32>,
}

/// Nice value a verification workload is entitled to.
pub const BASELINE_NICE: i32 = 0;

impl LauncherPriority {
    /// Observe the calling process.
    pub fn current() -> Self {
        Self {
            nice: current_nice(),
        }
    }

    /// Whether this launcher would pass a degraded priority to its children.
    pub fn is_degraded(&self) -> bool {
        self.nice.is_some_and(|nice| nice > BASELINE_NICE)
    }
}

/// Whether a daemon able to launch verification outside the agent tree can be
/// reached right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonAvailability {
    /// A daemon is running and speaks the verification-spawn protocol.
    Available,
    /// No daemon endpoint could be resolved.
    Absent,
    /// A daemon answered but predates the verification-spawn frames.
    Incompatible { protocol_version: u32 },
}

/// How the verification command should be launched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnPlacement {
    /// Launch it directly from this process, and say why the escape did not
    /// happen so the run's own record carries the answer.
    Inherit { reason: String },
    /// Hand it to the daemon, which launches it outside the agent tree.
    Delegate,
    /// A degraded launcher has no compatible daemon to escape to.
    Refuse { reason: String },
}

/// Decide where one verification command is launched from.
///
/// A missing or incompatible daemon is a configuration failure: refusing here
/// prevents an implicit fallback to the degraded agent tree (Issue #4473).
/// This is separate from a daemon that can spawn but cannot restore nice 0;
/// that workload still runs and records its observed priority (Issue #4409 AC-6).
pub fn decide_placement(launcher: LauncherPriority, daemon: &DaemonAvailability) -> SpawnPlacement {
    if !launcher.is_degraded() {
        return SpawnPlacement::Inherit {
            reason: match launcher.nice {
                Some(nice) => {
                    format!("launcher already runs at baseline priority (nice {nice})")
                }
                None => "platform has no nice value to inherit".to_string(),
            },
        };
    }
    match daemon {
        DaemonAvailability::Available => SpawnPlacement::Delegate,
        DaemonAvailability::Absent | DaemonAvailability::Incompatible { .. } => {
            SpawnPlacement::Refuse {
                reason: unescaped_reason(launcher, daemon),
            }
        }
    }
}

/// Explain why the workload cannot receive baseline priority and how to retry.
fn unescaped_reason(launcher: LauncherPriority, daemon: &DaemonAvailability) -> String {
    let nice = launcher
        .nice
        .map(|nice| nice.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let cause = match daemon {
        DaemonAvailability::Absent => {
            "no gwt daemon is reachable for this project scope".to_string()
        }
        DaemonAvailability::Incompatible { protocol_version } => format!(
            "the running gwt daemon speaks protocol {protocol_version}, which predates the \
             verification spawn frames"
        ),
        DaemonAvailability::Available => unreachable!("Available never lands here"),
    };
    format!(
        "verification spawn refused: launcher runs at nice {nice}, and {cause}. It cannot \
         guarantee nice 0 because an unprivileged process cannot lower its inherited nice \
         value from inside the agent process tree. No verification command was started. \
         Start or upgrade the gwt GUI so its daemon supports verification spawns, then \
         retry verify.run"
    )
}

/// Read the calling thread's nice value.
///
/// `who = 0` with `PRIO_PROCESS` is the one `getpriority` form that cannot
/// fail: `ESRCH` needs a process that does not exist and `EINVAL` needs a
/// `which` outside the three documented constants. So the usual `-1`/`errno`
/// disambiguation dance is unnecessary here, and `-1` can be returned as what
/// it is — a process someone renice'd *up*, which
/// [`LauncherPriority::is_degraded`] correctly treats as not degraded.
///
/// `PRIO_PGRP` would be the wrong reading: it reports the *lowest* nice in the
/// group, so one baseline-priority sibling would mask the degraded value this
/// process is actually about to hand to its children.
#[cfg(unix)]
fn current_nice() -> Option<i32> {
    // SAFETY: `getpriority` has no memory-safety preconditions.
    Some(unsafe { libc::getpriority(libc::PRIO_PROCESS as _, 0) })
}

#[cfg(not(unix))]
fn current_nice() -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn degraded() -> LauncherPriority {
        LauncherPriority { nice: Some(10) }
    }

    fn baseline() -> LauncherPriority {
        LauncherPriority { nice: Some(0) }
    }

    #[test]
    fn a_baseline_launcher_spawns_in_place_even_without_a_daemon() {
        let placement = decide_placement(baseline(), &DaemonAvailability::Absent);
        let SpawnPlacement::Inherit { reason } = placement else {
            panic!("a launcher at nice 0 has nothing to escape from: {placement:?}");
        };
        assert!(reason.contains("nice 0"), "{reason}");
    }

    #[test]
    fn a_platform_without_nice_spawns_in_place() {
        let placement =
            decide_placement(LauncherPriority { nice: None }, &DaemonAvailability::Absent);
        assert!(matches!(placement, SpawnPlacement::Inherit { .. }));
    }

    #[test]
    fn a_degraded_launcher_delegates_when_the_daemon_can_take_it() {
        assert_eq!(
            decide_placement(degraded(), &DaemonAvailability::Available),
            SpawnPlacement::Delegate
        );
    }

    /// Issue #4473: configuration failures refuse rather than inherit nice.
    #[test]
    fn a_degraded_launcher_without_a_daemon_is_refused_with_recovery() {
        let placement = decide_placement(degraded(), &DaemonAvailability::Absent);
        let SpawnPlacement::Refuse { reason } = placement else {
            panic!("a launcher with no daemon has no way to delegate: {placement:?}");
        };
        assert!(reason.contains("nice 10"), "{reason}");
        assert!(
            reason.contains("no gwt daemon is reachable"),
            "the record must say why baseline priority could not be given: {reason}"
        );
        assert!(
            reason.contains("nice 0")
                && reason.contains("GUI")
                && reason.contains("retry verify.run"),
            "the refusal must explain the guarantee and recovery: {reason}"
        );
    }

    #[test]
    fn an_outdated_daemon_is_refused_and_names_its_protocol() {
        let placement = decide_placement(
            degraded(),
            &DaemonAvailability::Incompatible {
                protocol_version: 3,
            },
        );
        let SpawnPlacement::Refuse { reason } = placement else {
            panic!("a daemon that cannot spawn is as good as absent: {placement:?}");
        };
        assert!(reason.contains("protocol 3"), "{reason}");
        assert!(
            reason.contains("upgrade") && reason.contains("retry verify.run"),
            "{reason}"
        );
    }

    #[test]
    fn degradation_is_read_from_the_inherited_nice_value() {
        assert!(degraded().is_degraded());
        assert!(!baseline().is_degraded());
        assert!(!LauncherPriority { nice: None }.is_degraded());
        // A launcher that was renice'd *up* is not the defect this guards.
        assert!(!LauncherPriority { nice: Some(-5) }.is_degraded());
    }
}
