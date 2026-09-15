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

use std::{collections::HashMap, io, path::Path, time::Duration};

use chrono::{DateTime, Utc};
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

/// How far back `recent` looks (Issue #4371 AC-4). The spikes #4234 recorded
/// rose and recovered within 5-8 minutes; 30 minutes keeps a recovered spike
/// visible across the gap between two PM status reads.
pub const HISTORY_WINDOW_SECS: u64 = 30 * 60;

/// How often the GUI records its own resident size into the perf log.
const GUI_RSS_SAMPLE_INTERVAL: Duration = Duration::from_secs(60);

/// Perf-log target prefix of a GUI resident-size sample; the pid follows.
const GUI_RSS_TARGET_PREFIX: &str = "gui_rss:pid=";

/// Resident-size samples per GUI pid, oldest first.
pub type RssHistorySamples = HashMap<u32, Vec<(DateTime<Utc>, u64)>>;

/// One gwt GUI process as seen from the OS process table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GwtProcessMemory {
    pub pid: u32,
    pub rss_bytes: u64,
    /// Seconds since the process started, so the size can be read against
    /// uptime the way the Issue #4234 acceptance criteria ask.
    pub uptime_secs: u64,
    /// Peak and average over the last [`HISTORY_WINDOW_SECS`], from samples
    /// the process wrote itself (Issue #4371 AC-4). `rss_bytes` far below
    /// `recent.peak_rss_bytes` is a spike that recovered; both near the peak
    /// is residency that stayed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent: Option<RecentRss>,
}

/// Resident size of one GUI process over the recent window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentRss {
    pub window_secs: u64,
    pub samples: usize,
    pub peak_rss_bytes: u64,
    pub peak_at: DateTime<Utc>,
    pub average_rss_bytes: u64,
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
            recent: None,
        })
        .collect();
    let now = Utc::now();
    evaluate_with_history(processes, &read_rss_history(now), now)
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

/// [`evaluate`] with each process's recent resident-size history attached.
pub fn evaluate_with_history(
    processes: Vec<GwtProcessMemory>,
    history: &RssHistorySamples,
    now: DateTime<Utc>,
) -> MemoryPressureStatus {
    let processes = processes
        .into_iter()
        .map(|process| GwtProcessMemory {
            recent: history
                .get(&process.pid)
                .and_then(|samples| recent_rss(samples, now)),
            ..process
        })
        .collect();
    evaluate(processes)
}

fn recent_rss(samples: &[(DateTime<Utc>, u64)], now: DateTime<Utc>) -> Option<RecentRss> {
    let window_start = now - chrono::Duration::seconds(HISTORY_WINDOW_SECS as i64);
    let in_window = samples
        .iter()
        .filter(|(at, _)| *at >= window_start && *at <= now)
        .collect::<Vec<_>>();
    let (peak_at, peak_rss_bytes) = in_window
        .iter()
        .max_by_key(|(_, rss)| *rss)
        .map(|sample| **sample)?;
    let total: u128 = in_window.iter().map(|(_, rss)| u128::from(*rss)).sum();
    Some(RecentRss {
        window_secs: HISTORY_WINDOW_SECS,
        samples: in_window.len(),
        peak_rss_bytes,
        peak_at,
        average_rss_bytes: (total / in_window.len() as u128) as u64,
    })
}

/// Persist one resident-size sample of GUI `pid` into the perf log.
pub fn record_rss_sample(
    sink: &mut crate::perf::PerfSink,
    pid: u32,
    rss_bytes: u64,
    at: DateTime<Utc>,
) -> io::Result<()> {
    sink.record_sample(
        at,
        crate::perf::PerfStream::Resource,
        format!("{GUI_RSS_TARGET_PREFIX}{pid}"),
        rss_bytes as f64,
        crate::perf::PerfUnit::Bytes,
    )
}

/// Read the GUI resident-size samples of the recent window back from the perf
/// log. A missing or unreadable log is an empty history, never an error: the
/// probe still reports the live size.
pub fn read_rss_history(now: DateTime<Utc>) -> RssHistorySamples {
    let filter = crate::perf::summary::PerfFilter {
        since: Some(now - chrono::Duration::seconds(HISTORY_WINDOW_SECS as i64)),
        stream: Some("resource".to_string()),
        target: Some(GUI_RSS_TARGET_PREFIX.to_string()),
    };
    let mut history = RssHistorySamples::new();
    for record in crate::perf::summary::read_records(&filter).unwrap_or_default() {
        let Some(pid) = record
            .target
            .strip_prefix(GUI_RSS_TARGET_PREFIX)
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if !record.is_sample() || !record.value.is_finite() || record.value < 0.0 {
            continue;
        }
        history
            .entry(pid)
            .or_default()
            .push((record.timestamp, record.value as u64));
    }
    history
}

/// GUI only: record this process's resident size into the perf log every
/// minute, so the one-shot `gwtd` probe can tell a recovered spike from
/// residency that stayed (Issue #4371 AC-4). Fail-open like every perf write:
/// with the kill switch off each tick is a no-op.
pub fn spawn_gui_rss_sampler() {
    let spawned = std::thread::Builder::new()
        .name("gwt-rss-sampler".to_string())
        .spawn(|| {
            let pid = std::process::id();
            let sys_pid = sysinfo::Pid::from_u32(pid);
            let mut system = System::new();
            loop {
                system.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&[sys_pid]),
                    true,
                    ProcessRefreshKind::nothing().with_memory(),
                );
                if let Some(process) = system.process(sys_pid) {
                    let rss_bytes = process.memory();
                    crate::perf::global::with_sink(|sink| {
                        let _ = record_rss_sample(sink, pid, rss_bytes, Utc::now());
                    });
                }
                std::thread::sleep(GUI_RSS_SAMPLE_INTERVAL);
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the GUI resident-size sampler");
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
    use chrono::TimeZone as _;

    use super::*;

    fn process(pid: u32, rss_bytes: u64, uptime_secs: u64) -> GwtProcessMemory {
        GwtProcessMemory {
            pid,
            rss_bytes,
            uptime_secs,
            recent: None,
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

    const MIB: u64 = 1024 * 1024;

    /// Issue #4371 AC-4: the #4234 samples show 04:21-04:28 rising to
    /// 1,752 MB and 04:29 back at 546 MB. A one-shot probe at 04:29 sees only
    /// the 546 MB; the recent peak, when it happened and the window average
    /// must travel with it so a recovered spike is not read as retention.
    #[test]
    fn recovered_spike_is_reported_next_to_the_current_residency() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 9, 15, 4, 29, 0)
            .single()
            .expect("valid time");
        let minutes_ago = |minutes: i64| now - chrono::Duration::minutes(minutes);
        let mut history = RssHistorySamples::new();
        history.insert(
            7,
            vec![
                (minutes_ago(8), 1_031 * MIB),
                (minutes_ago(2), 1_752 * MIB),
                (minutes_ago(0), 546 * MIB),
            ],
        );
        history.insert(8, vec![(minutes_ago(45), 9_000 * MIB)]);

        let status = evaluate_with_history(
            vec![process(7, 546 * MIB, 21 * 3600), process(8, 500 * MIB, 60)],
            &history,
            now,
        );

        let recent = status.processes[0]
            .recent
            .as_ref()
            .expect("pid 7 carries its recent history");
        assert_eq!(recent.peak_rss_bytes, 1_752 * MIB);
        assert_eq!(recent.peak_at, minutes_ago(2));
        assert_eq!(recent.average_rss_bytes, (1_031 + 1_752 + 546) * MIB / 3);
        assert_eq!(recent.samples, 3);
        assert_eq!(recent.window_secs, HISTORY_WINDOW_SECS);
        assert_eq!(
            status.processes[1].recent, None,
            "samples older than the window are not history"
        );
        assert_eq!(
            status.warning, None,
            "a recovered spike is not a sustained-residency warning"
        );
    }

    /// Issue #4371 AC-4: the GUI writes its resident size into the perf log it
    /// already establishes, and the short-lived `gwtd` probe reads it back per
    /// process without creating anything under the HOME.
    #[test]
    fn gui_rss_samples_round_trip_through_the_perf_log_per_process() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let mut sink = crate::perf::PerfSink::from_config(&gwt_config::PerfConfig {
            enabled: true,
            ..gwt_config::PerfConfig::default()
        })
        .expect("perf sink");
        let now = chrono::Utc::now();
        let earlier = now - chrono::Duration::minutes(3);
        record_rss_sample(&mut sink, 7, 1_200 * MIB, earlier).expect("record pid 7");
        record_rss_sample(&mut sink, 9, 80 * MIB, now).expect("record pid 9");

        let history = read_rss_history(now);

        assert_eq!(history.get(&7), Some(&vec![(earlier, 1_200 * MIB)]));
        assert_eq!(history.get(&9), Some(&vec![(now, 80 * MIB)]));
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
