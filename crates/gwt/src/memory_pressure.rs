//! Resident-size observation of the gwt GUI processes on this host
//! (Issue #4234 AC-5).
//!
//! On 2026-09-10 a gwt GUI process grew to 14.7 GB and its pane WebSocket
//! stopped answering; the PM only learned about it from
//! `pane_backend_unresponsive`, after every pane had already become
//! unobservable. This module reads each GUI process's resident size straight
//! from the OS, so the number is available even when the process itself no
//! longer answers, and turns it into a status field the PM reads anyway
//! (`issue.monitor.status`).

use std::path::Path;

use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// Warn once a single gwt GUI process holds more than this resident size
/// (4 GiB). While diagnosing Issue #4234 a healthy instance stayed near
/// 0.5 GB after 19 hours; the instances whose pane WebSocket stopped
/// answering were past 9 GB.
pub const WARN_ABOVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Executable stem of the GUI binary. `gwtd` is a different name, so the
/// JSON-operation processes never count.
const GUI_PROCESS_STEM: &str = "gwt";

/// One gwt GUI process as seen from the OS process table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GwtProcessMemory {
    pub pid: u32,
    pub rss_bytes: u64,
    /// Seconds since the process started, so the size can be read against
    /// uptime the way the Issue #4234 acceptance criteria ask.
    pub uptime_secs: u64,
}

/// Resident-size verdict over every gwt GUI process on the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPressureStatus {
    pub processes: Vec<GwtProcessMemory>,
    pub warn_above_bytes: u64,
    /// Present while any process is above the threshold; names the process
    /// and the surface that stops answering next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// Read every gwt GUI process from the OS and evaluate the threshold.
pub fn probe() -> MemoryPressureStatus {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    let processes = system
        .processes()
        .values()
        .filter(|process| is_gui_process(Path::new(process.name())))
        .map(|process| GwtProcessMemory {
            pid: process.pid().as_u32(),
            rss_bytes: process.memory(),
            uptime_secs: process.run_time(),
        })
        .collect();
    evaluate(processes)
}

fn is_gui_process(name: &Path) -> bool {
    name.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case(GUI_PROCESS_STEM))
}

/// Evaluate the threshold over `processes` (sorted by pid for stable output).
pub fn evaluate(mut processes: Vec<GwtProcessMemory>) -> MemoryPressureStatus {
    processes.sort_by_key(|process| process.pid);
    let warnings = processes
        .iter()
        .filter(|process| process.rss_bytes > WARN_ABOVE_BYTES)
        .map(|process| {
            format!(
                "gwt pid {} holds {} resident after {} (above {}): pane.list / pane.read time out \
                 with pane_backend_unresponsive once this instance saturates (Issue #4234)",
                process.pid,
                format_gib(process.rss_bytes),
                format_uptime(process.uptime_secs),
                format_gib(WARN_ABOVE_BYTES),
            )
        })
        .collect::<Vec<_>>();
    MemoryPressureStatus {
        processes,
        warn_above_bytes: WARN_ABOVE_BYTES,
        warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
    }
}

fn format_gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

fn format_uptime(secs: u64) -> String {
    format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, rss_bytes: u64, uptime_secs: u64) -> GwtProcessMemory {
        GwtProcessMemory {
            pid,
            rss_bytes,
            uptime_secs,
        }
    }

    #[test]
    fn healthy_processes_carry_no_warning() {
        let status = evaluate(vec![
            process(2, 512 * 1024 * 1024, 70_000),
            process(1, 0, 5),
        ]);
        assert_eq!(status.warning, None);
        assert_eq!(status.warn_above_bytes, WARN_ABOVE_BYTES);
        assert_eq!(
            status.processes.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![1, 2],
            "processes are reported in pid order"
        );
    }

    #[test]
    fn process_above_threshold_is_named_with_size_and_uptime() {
        let status = evaluate(vec![
            process(51227, 14_774_304 * 1024, 20 * 3600 + 35 * 60),
            process(7, 100, 1),
        ]);
        let warning = status.warning.expect("warning above 4 GiB");
        assert!(warning.contains("gwt pid 51227"), "{warning}");
        assert!(warning.contains("14.1 GiB"), "{warning}");
        assert!(warning.contains("20h35m"), "{warning}");
        assert!(warning.contains("pane_backend_unresponsive"), "{warning}");
    }

    #[test]
    fn only_the_gui_binary_counts() {
        assert!(is_gui_process(Path::new("gwt")));
        assert!(is_gui_process(Path::new("gwt.exe")));
        assert!(is_gui_process(Path::new("GWT")));
        assert!(!is_gui_process(Path::new("gwtd")));
        assert!(!is_gui_process(Path::new("gwtd.exe")));
        assert!(!is_gui_process(Path::new("gwt-helper")));
    }
}
