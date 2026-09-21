//! CPU observation of Spotlight's indexing daemon on this host
//! (Issue #4386).
//!
//! On 2026-09-15 `mds_stores` was the single largest CPU consumer on a host
//! running only three agent windows: 138.8% against a load average of 17.84,
//! because Spotlight indexes the `target/` directory of every one of the 462
//! worktrees. Nothing reported it, so the PM only found it by running `ps` by
//! hand and the cost read as "the host is slow" — which is how a verification
//! deadline flake gets misdiagnosed as an agent problem.
//!
//! This module reads the daemon's CPU straight from the process table and
//! turns it into a status field the PM reads anyway
//! (`issue.monitor.status`), next to `memory_pressure` and `disk_space`.
//! Reducing the load itself belongs to #4391 (reclaiming `target/`); this
//! module only makes the load visible.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Warn once the daemon sustains more than one full core (100%). #4298 uses
/// the same bar for `fseventsd`; the observation that opened this Issue was
/// 138.8%.
pub const WARN_ABOVE_PERCENT: f64 = 100.0;

/// Spotlight's indexing daemon only exists on macOS, so every other platform
/// reports no processes and no warning rather than an error.
const HOST_HAS_SPOTLIGHT: bool = cfg!(target_os = "macos");

/// Executable name of Spotlight's store/indexing daemon. `mdworker` and
/// `mds` are separate processes and are not counted.
const INDEXER_PROCESS_NAME: &str = "mds_stores";

/// One `mds_stores` process as seen from the OS process table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpotlightProcess {
    pub pid: u32,
    /// CPU percentage as `ps` reports it, so 138.8 means the daemon is using
    /// more than one core. Values above 100 are expected on multi-core hosts.
    pub cpu_percent: f64,
}

/// CPU verdict over every Spotlight indexing process on the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpotlightPressureStatus {
    pub processes: Vec<SpotlightProcess>,
    pub warn_above_percent: f64,
    /// Present while any process is above the threshold; names the process
    /// and what the load costs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// Read Spotlight's indexing processes from the OS and evaluate the
/// threshold. A no-op that reports nothing on hosts without Spotlight.
pub fn probe() -> SpotlightPressureStatus {
    if !HOST_HAS_SPOTLIGHT {
        return evaluate(Vec::new());
    }
    evaluate(parse_indexer_processes(&read_process_table()))
}

/// Evaluate the threshold over `processes` (sorted by pid for stable output).
pub fn evaluate(mut processes: Vec<SpotlightProcess>) -> SpotlightPressureStatus {
    processes.sort_by_key(|process| process.pid);
    let warnings = processes
        .iter()
        .filter(|process| {
            process.cpu_percent.is_finite() && process.cpu_percent > WARN_ABOVE_PERCENT
        })
        .map(|process| {
            format!(
                "mds_stores pid {} is using {:.1}% CPU (above {:.0}%): Spotlight is indexing the \
                 target directory of every worktree and competes with cargo, which reads as \
                 verify deadline flakes and host saturation (Issue #4386)",
                process.pid, process.cpu_percent, WARN_ABOVE_PERCENT,
            )
        })
        .collect::<Vec<_>>();
    SpotlightPressureStatus {
        processes,
        warn_above_percent: WARN_ABOVE_PERCENT,
        warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
    }
}

fn read_process_table() -> String {
    let Ok(output) = gwt_core::process::hidden_command("ps")
        .args(["axww", "-o", "pid=,%cpu=,command="])
        .output()
    else {
        return String::new();
    };
    if !output.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Keep the `mds_stores` lines of a `pid %cpu command` process table. The
/// executable is the first token of the command, the way #4298 matches
/// `fseventsd`, so a line that merely mentions the daemon in its arguments
/// (a `grep`, another agent's `ps`) is not one of its processes.
fn parse_indexer_processes(table: &str) -> Vec<SpotlightProcess> {
    table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let cpu_percent = fields.next()?.parse().ok()?;
            let executable = fields.next()?;
            Path::new(executable)
                .file_name()
                .is_some_and(|name| name == INDEXER_PROCESS_NAME)
                .then_some(SpotlightProcess { pid, cpu_percent })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, cpu_percent: f64) -> SpotlightProcess {
        SpotlightProcess { pid, cpu_percent }
    }

    #[test]
    fn spotlight_below_the_threshold_carries_no_warning() {
        let status = evaluate(vec![process(402, 100.0), process(11, 3.4)]);

        assert_eq!(status.warning, None);
        assert_eq!(status.warn_above_percent, WARN_ABOVE_PERCENT);
        assert_eq!(
            status.processes.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![11, 402],
            "processes are reported in pid order"
        );
    }

    #[test]
    fn spotlight_above_the_threshold_is_named_with_its_cpu() {
        let status = evaluate(vec![process(402, 138.8), process(11, 3.4)]);

        let warning = status.warning.expect("warning above 100% CPU");
        assert!(warning.contains("mds_stores pid 402"), "{warning}");
        assert!(warning.contains("138.8%"), "{warning}");
        assert!(warning.contains("above 100%"), "{warning}");
        assert!(!warning.contains("pid 11"), "{warning}");
    }

    #[test]
    fn spotlight_probe_is_a_no_op_on_hosts_without_spotlight() {
        let status = probe();

        assert_eq!(status.warn_above_percent, WARN_ABOVE_PERCENT);
        if !HOST_HAS_SPOTLIGHT {
            assert_eq!(
                status,
                evaluate(Vec::new()),
                "hosts without Spotlight report no processes and no warning"
            );
        }
    }

    #[test]
    fn only_the_indexing_daemon_counts() {
        let table = "\
  402   138.8 /usr/libexec/mds_stores
  401     0.1 /usr/libexec/mds
  403     7.2 mdworker_shared
  404     0.0 /usr/bin/grep mds_stores
";

        assert_eq!(
            parse_indexer_processes(table),
            vec![process(402, 138.8)],
            "only the mds_stores processes themselves are reported"
        );
        assert!(parse_indexer_processes("").is_empty());
    }
}
