//! Process file-descriptor budget (Issue #4142).
//!
//! A gwt GUI process holds three descriptors per live PTY pane — the master
//! `openpty` returns plus the reader and writer clones `portable-pty` dups out
//! of it — so the launchd default soft `RLIMIT_NOFILE` of 256 caps the process
//! at roughly 80 concurrent agents. Past that ceiling every PTY creation,
//! daemon connect, prefs read and tokio runtime build fails with
//! `Too many open files`, which reads as a total runtime outage rather than as
//! a resource limit.
//!
//! [`raise_soft_fd_limit`] moves the ceiling out of the way at startup; the
//! probes below explain the budget when something exhausts it anyway.

/// Soft `RLIMIT_NOFILE` values gwt is willing to run with, most generous
/// first. Each candidate is clamped to the hard limit before it is requested,
/// and the first accepted value wins. macOS refuses a soft limit above
/// `kern.maxfilesperproc` even when the hard limit reports `unlimited`, which
/// is why this is a ladder and not a single request.
pub const SOFT_FD_LIMIT_TARGETS: [u64; 3] = [65_536, 10_240, 8_192];

/// Lowest soft limit that still leaves room for a realistic agent fleet
/// (~2700 concurrent PTY panes at three descriptors each).
pub const MIN_SOFT_FD_LIMIT: u64 = 8_192;

/// Highest descriptor number [`open_fd_count`] and [`open_tty_fd_count`] probe.
/// Descriptors are allocated lowest-first, so a bounded scan stays exact for
/// any process that has not already blown past this many open files, and it
/// keeps the probe cheap when the soft limit is in the millions. Public
/// because it is the bound a caller needs to interpret the counts, and because
/// only the unix implementation consumes it.
pub const FD_PROBE_CEILING: u64 = 65_536;

/// A process resource limit pair, in descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdLimit {
    pub soft: u64,
    pub hard: u64,
}

/// Outcome of [`raise_soft_fd_limit`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FdLimitRaise {
    /// Limit observed before the raise. `None` where the platform has no
    /// `RLIMIT_NOFILE` (Windows).
    pub before: Option<FdLimit>,
    /// Limit observed after the raise, read back from the OS rather than
    /// assumed from the request.
    pub after: Option<FdLimit>,
    /// Soft limit that was accepted, if any request was made and succeeded.
    pub requested: Option<u64>,
    /// Why no request succeeded, when one was attempted.
    pub error: Option<String>,
}

impl FdLimitRaise {
    /// Soft limit the process actually runs under after the attempt.
    pub fn effective_soft(&self) -> Option<u64> {
        self.after.map(|limit| limit.soft)
    }

    /// Whether the process ended up with at least [`MIN_SOFT_FD_LIMIT`].
    pub fn meets_minimum(&self) -> bool {
        self.effective_soft()
            .is_some_and(|soft| soft >= MIN_SOFT_FD_LIMIT)
    }
}

/// Soft limits worth requesting for `current`, most generous first.
///
/// Each target is clamped to the hard limit, candidates that would not raise
/// the current soft limit are dropped, and duplicates produced by the clamp
/// are collapsed so a low hard limit is not requested three times.
pub fn soft_limit_request_ladder(current: FdLimit) -> Vec<u64> {
    let mut ladder: Vec<u64> = Vec::with_capacity(SOFT_FD_LIMIT_TARGETS.len());
    for target in SOFT_FD_LIMIT_TARGETS {
        let candidate = target.min(current.hard);
        if candidate > current.soft && !ladder.contains(&candidate) {
            ladder.push(candidate);
        }
    }
    ladder
}

/// One-line description of the current descriptor budget, for diagnostics that
/// have to explain an `EMFILE` failure. `None` where the platform exposes no
/// `RLIMIT_NOFILE`.
pub fn describe_fd_budget() -> Option<String> {
    let limit = current_fd_limit()?;
    let hard = if limit.hard == u64::MAX {
        "unlimited".to_string()
    } else {
        limit.hard.to_string()
    };
    let open = match open_fd_count() {
        Some(open) => open.to_string(),
        None => "unknown".to_string(),
    };
    Some(format!(
        "open fds {open}, soft RLIMIT_NOFILE {}, hard {hard}",
        limit.soft
    ))
}

#[cfg(unix)]
mod platform {
    use super::{FdLimit, FdLimitRaise, FD_PROBE_CEILING};

    /// Read the process `RLIMIT_NOFILE` pair.
    #[allow(
        clippy::unnecessary_cast,
        reason = "libc::rlim_t is u64 on every target gwt builds for, but POSIX only guarantees an unsigned integer type"
    )]
    pub fn current_fd_limit() -> Option<FdLimit> {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `limit` is a live, correctly sized `rlimit` the kernel only
        // writes into.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) } != 0 {
            return None;
        }
        Some(FdLimit {
            soft: limit.rlim_cur as u64,
            hard: limit.rlim_max as u64,
        })
    }

    fn set_soft_fd_limit(soft: u64, hard: u64) -> std::io::Result<()> {
        let limit = libc::rlimit {
            rlim_cur: soft as libc::rlim_t,
            rlim_max: hard as libc::rlim_t,
        };
        // SAFETY: `limit` is a live, correctly sized `rlimit` the kernel only
        // reads from.
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    pub fn raise_soft_fd_limit() -> FdLimitRaise {
        let Some(before) = current_fd_limit() else {
            return FdLimitRaise {
                error: Some("getrlimit(RLIMIT_NOFILE) failed".to_string()),
                ..FdLimitRaise::default()
            };
        };
        let mut raise = FdLimitRaise {
            before: Some(before),
            after: Some(before),
            ..FdLimitRaise::default()
        };
        let mut last_error = None;
        for candidate in super::soft_limit_request_ladder(before) {
            match set_soft_fd_limit(candidate, before.hard) {
                Ok(()) => {
                    raise.requested = Some(candidate);
                    raise.after = current_fd_limit().or(Some(FdLimit {
                        soft: candidate,
                        hard: before.hard,
                    }));
                    return raise;
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        raise.error = last_error;
        raise
    }

    fn probe_ceiling() -> i32 {
        let soft = current_fd_limit().map_or(FD_PROBE_CEILING, |limit| limit.soft);
        i32::try_from(soft.min(FD_PROBE_CEILING)).unwrap_or(i32::MAX)
    }

    fn fd_is_open(fd: i32) -> bool {
        // SAFETY: `F_GETFD` only reads the descriptor flags and reports EBADF
        // for a closed descriptor.
        unsafe { libc::fcntl(fd, libc::F_GETFD) != -1 }
    }

    pub fn open_fd_count() -> Option<usize> {
        Some((0..probe_ceiling()).filter(|fd| fd_is_open(*fd)).count())
    }

    pub fn open_tty_fd_count() -> Option<usize> {
        Some(
            (0..probe_ceiling())
                .filter(|fd| {
                    // SAFETY: `isatty` only inspects the descriptor.
                    fd_is_open(*fd) && unsafe { libc::isatty(*fd) } == 1
                })
                .count(),
        )
    }
}

#[cfg(not(unix))]
mod platform {
    use super::{FdLimit, FdLimitRaise};

    /// Windows has no `RLIMIT_NOFILE`: handle counts are bounded by the
    /// system-wide handle table, not by a per-process soft limit gwt could
    /// raise.
    pub fn current_fd_limit() -> Option<FdLimit> {
        None
    }

    pub fn raise_soft_fd_limit() -> FdLimitRaise {
        FdLimitRaise::default()
    }

    pub fn open_fd_count() -> Option<usize> {
        None
    }

    pub fn open_tty_fd_count() -> Option<usize> {
        None
    }
}

/// Read the process `RLIMIT_NOFILE` pair. `None` on platforms without one.
pub fn current_fd_limit() -> Option<FdLimit> {
    platform::current_fd_limit()
}

/// Raise the soft `RLIMIT_NOFILE` toward the hard limit, capped by
/// [`SOFT_FD_LIMIT_TARGETS`]. Never lowers an already generous limit, and is
/// a no-op on platforms without `RLIMIT_NOFILE`.
pub fn raise_soft_fd_limit() -> FdLimitRaise {
    platform::raise_soft_fd_limit()
}

/// Number of open descriptors below [`FD_PROBE_CEILING`].
pub fn open_fd_count() -> Option<usize> {
    platform::open_fd_count()
}

/// Number of open descriptors that are terminals. PTY masters are the only
/// terminals a gwt process opens beyond its own stdio, so a delta of this
/// count is a direct measure of PTY descriptor lifetime.
pub fn open_tty_fd_count() -> Option<usize> {
    platform::open_tty_fd_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_stops_at_the_hard_limit() {
        let ladder = soft_limit_request_ladder(FdLimit {
            soft: 256,
            hard: 10_240,
        });
        assert_eq!(ladder, vec![10_240, 8_192]);
    }

    #[test]
    fn ladder_collapses_candidates_clamped_to_the_same_hard_limit() {
        let ladder = soft_limit_request_ladder(FdLimit {
            soft: 256,
            hard: 4_096,
        });
        assert_eq!(ladder, vec![4_096]);
    }

    #[test]
    fn ladder_is_empty_when_the_soft_limit_already_exceeds_every_target() {
        let ladder = soft_limit_request_ladder(FdLimit {
            soft: 1_048_576,
            hard: u64::MAX,
        });
        assert!(ladder.is_empty(), "must never lower an existing soft limit");
    }

    #[test]
    fn ladder_from_the_launchd_default_reaches_the_most_generous_target() {
        let ladder = soft_limit_request_ladder(FdLimit {
            soft: 256,
            hard: u64::MAX,
        });
        assert_eq!(ladder.first().copied(), Some(SOFT_FD_LIMIT_TARGETS[0]));
        assert!(ladder
            .iter()
            .all(|candidate| *candidate >= MIN_SOFT_FD_LIMIT));
    }

    #[cfg(unix)]
    #[test]
    fn raising_leaves_at_least_the_minimum_soft_limit() {
        let raise = raise_soft_fd_limit();
        assert!(
            raise.meets_minimum(),
            "effective soft limit {:?} is below {MIN_SOFT_FD_LIMIT}: {raise:?}",
            raise.effective_soft()
        );
    }

    #[cfg(unix)]
    #[test]
    fn budget_description_names_the_open_count_and_the_soft_limit() {
        let described = describe_fd_budget().expect("unix exposes RLIMIT_NOFILE");
        assert!(described.contains("open fds"), "{described}");
        assert!(described.contains("soft RLIMIT_NOFILE"), "{described}");
    }

    #[cfg(unix)]
    #[test]
    fn open_fd_count_observes_newly_opened_descriptors() {
        // Opening a batch keeps the assertion immune to sibling tests in this
        // binary closing a descriptor of their own between the two probes.
        const BATCH: usize = 64;

        let before = open_fd_count().expect("unix exposes an fd table");
        let files: Vec<tempfile::NamedTempFile> = (0..BATCH)
            .map(|_| tempfile::NamedTempFile::new().expect("temp file"))
            .collect();
        let during = open_fd_count().expect("unix exposes an fd table");
        drop(files);

        assert!(
            during >= before + BATCH / 2,
            "open fd count did not observe {BATCH} new descriptors: {before} -> {during}"
        );
    }
}
