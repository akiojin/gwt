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
    /// Launch it directly from this process: nothing would be gained by
    /// escaping, because this process is already at baseline priority.
    Inherit { reason: String },
    /// Hand it to the daemon, which launches it outside the agent tree.
    Delegate,
    /// Refuse to run. Escaping is required and no host can do it.
    Reject { message: String },
}

/// Decide where one verification command is launched from.
///
/// The two refusal-shaped conditions are deliberately *not* symmetric
/// (Issue #4409 AC-5 / AC-6):
///
/// - a missing daemon is a **configuration** problem the caller can fix, so it
///   is refused rather than silently downgraded back into the agent tree;
/// - a nice value that cannot be lowered is an **environment** problem the
///   caller cannot fix, so the run proceeds and records what it got. That case
///   is decided by the launching host, not here — see
///   [`crate::daemon::VerificationSpawnAccepted`].
pub fn decide_placement(
    launcher: LauncherPriority,
    daemon: &DaemonAvailability,
) -> SpawnPlacement {
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
            SpawnPlacement::Reject {
                message: rejection_message(launcher, daemon),
            }
        }
    }
}

/// Explain a refusal in the terms the caller needs: why baseline priority
/// cannot be guaranteed here, and what to do next (Issue #4409 AC-5).
fn rejection_message(launcher: LauncherPriority, daemon: &DaemonAvailability) -> String {
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
        DaemonAvailability::Available => unreachable!("Available is never refused"),
    };
    format!(
        "verify.run refuses to launch verification from this process: it runs at nice {nice}, \
         and a non-privileged process cannot lower its own nice value, so every command it \
         spawns would inherit the agent launch policy's degraded priority (SPEC #1921 Phase 86; \
         measured as a 54x slowdown in Issue #4405). Baseline priority can only be guaranteed by \
         launching from the gwt daemon, and {cause}. Falling back to an in-tree spawn is not \
         offered, because a silent fallback reproduces the starvation this check exists to \
         prevent. Next: start the gwt GUI for this project (it owns the daemon), or run \
         `gwtd daemon status` to see why the daemon is not reachable, then retry verify.run."
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

    /// AC-5: a missing daemon is refused outright. The whole point of the
    /// refusal is that an implicit in-tree fallback would silently reproduce
    /// the starvation, so `Inherit` must never be the answer here.
    #[test]
    fn a_degraded_launcher_without_a_daemon_is_refused_not_downgraded() {
        let placement = decide_placement(degraded(), &DaemonAvailability::Absent);
        let SpawnPlacement::Reject { message } = placement else {
            panic!("an in-tree fallback reproduces the starvation: {placement:?}");
        };
        assert!(message.contains("nice 10"), "{message}");
        assert!(
            message.contains("no gwt daemon is reachable"),
            "the refusal must say why baseline priority cannot be guaranteed: {message}"
        );
        assert!(
            message.contains("retry verify.run"),
            "the refusal must name the next operation: {message}"
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
        let SpawnPlacement::Reject { message } = placement else {
            panic!("an daemon that cannot spawn is as good as absent: {placement:?}");
        };
        assert!(message.contains("protocol 3"), "{message}");
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
