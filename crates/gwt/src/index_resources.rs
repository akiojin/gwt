//! Index resource diagnostics (Issue #4527 AC-1, SPEC #1939 AS-33 / FR-425).
//!
//! Measures the logical runner tree from the OS through `sysinfo`, so Windows
//! reports the same CPU / RSS / private-memory fields as POSIX without
//! depending on `ps`. The heavy lease and refresh broker counters come from
//! their existing read-only projections; nothing here redesigns scheduling.

use std::collections::HashSet;

use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// Measured usage of one process and every descendant.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessTreeUsage {
    pub process_count: usize,
    /// Sum of per-process CPU, where 100 is one fully used logical core.
    pub cpu_percent: f64,
    pub rss_bytes: u64,
    /// Private commit. Only Windows exposes it (`PrivateUsage`); elsewhere it
    /// is `None` — unsupported, never reported as zero.
    pub private_bytes: Option<u64>,
}

/// Measure `root_pid` and its descendants over one CPU sampling interval.
/// Returns `None` when the root does not exist.
pub fn measure_process_tree(root_pid: u32) -> Option<ProcessTreeUsage> {
    let root = sysinfo::Pid::from_u32(root_pid);
    let refresh = ProcessRefreshKind::nothing().with_cpu().with_memory();
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    system.process(root)?;
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    system.process(root)?;

    let mut tree = HashSet::from([root]);
    loop {
        let before = tree.len();
        for (pid, process) in system.processes() {
            if process
                .parent()
                .is_some_and(|parent| tree.contains(&parent))
            {
                tree.insert(*pid);
            }
        }
        if tree.len() == before {
            break;
        }
    }
    let processes: Vec<_> = tree.iter().filter_map(|pid| system.process(*pid)).collect();
    Some(ProcessTreeUsage {
        process_count: processes.len(),
        cpu_percent: processes
            .iter()
            .map(|process| f64::from(process.cpu_usage()))
            .sum(),
        rss_bytes: processes.iter().map(|process| process.memory()).sum(),
        private_bytes: cfg!(windows).then(|| {
            processes
                .iter()
                .map(|process| process.virtual_memory())
                .sum()
        }),
    })
}

/// Host-wide index resource snapshot for `index.status` and
/// `diagnostics cpu --json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct IndexResourceDiagnostics {
    pub heavy_held: bool,
    pub heavy_owner_pid: Option<u32>,
    pub heavy_holder_kind: Option<String>,
    pub heavy_pending: usize,
    pub broker_targets: usize,
    pub broker_queue_depth: usize,
    pub broker_running: usize,
    /// Targets holding a coalesced follow-up refresh behind a running one.
    pub broker_follow_ups: usize,
    /// The heavy owner's logical tree; `None` while no live owner holds it.
    pub runner_tree: Option<ProcessTreeUsage>,
}

/// Collect the snapshot without creating coordinator or broker state on a
/// host that has none yet.
pub fn collect_index_resources() -> IndexResourceDiagnostics {
    let mut diagnostics = IndexResourceDiagnostics::default();
    let coordinator_root = gwt_core::index_coordinator::coordinator_root();
    if coordinator_root.is_dir() {
        if let Ok(status) = gwt_core::index_coordinator::IndexCoordinator::open(coordinator_root)
            .and_then(|coordinator| coordinator.heavy_lease_status())
        {
            diagnostics.heavy_held = status.held;
            diagnostics.heavy_pending = status.pending;
            if status.held {
                diagnostics.heavy_owner_pid = status.owner.as_ref().map(|owner| owner.pid);
                diagnostics.heavy_holder_kind =
                    status.holder_kind.map(|kind| kind.as_str().to_string());
            }
        }
    }
    let broker_root = gwt_core::index::broker::refresh_broker_root();
    if broker_root.is_dir() {
        if let Ok(snapshot) = gwt_core::index::broker::RefreshBroker::open(
            broker_root,
            gwt_core::index::broker::DEFAULT_REFRESH_QUIET_PERIOD,
        )
        .and_then(|broker| broker.inspect())
        {
            diagnostics.broker_targets = snapshot.target_count();
            diagnostics.broker_queue_depth = snapshot.queue_depth();
            diagnostics.broker_running = snapshot.running_count();
            diagnostics.broker_follow_ups = snapshot
                .targets()
                .iter()
                .filter(|target| target.follow_up_count() > 0)
                .count();
        }
    }
    diagnostics.runner_tree = diagnostics.heavy_owner_pid.and_then(measure_process_tree);
    diagnostics
}

/// Render the snapshot as `index.status` lines.
pub fn render_index_resources(out: &mut String, diagnostics: &IndexResourceDiagnostics) {
    out.push_str(&format!(
        "resources: heavy_held={} heavy_owner={} heavy_kind={} heavy_pending={} broker_targets={} queue_depth={} running={} follow_ups={}\n",
        diagnostics.heavy_held,
        diagnostics
            .heavy_owner_pid
            .map_or_else(|| "none".to_string(), |pid| pid.to_string()),
        diagnostics.heavy_holder_kind.as_deref().unwrap_or("none"),
        diagnostics.heavy_pending,
        diagnostics.broker_targets,
        diagnostics.broker_queue_depth,
        diagnostics.broker_running,
        diagnostics.broker_follow_ups,
    ));
    match &diagnostics.runner_tree {
        Some(tree) => out.push_str(&format!(
            "resources: runner_tree processes={} cpu_percent={:.1} rss_bytes={} private_bytes={}\n",
            tree.process_count,
            tree.cpu_percent,
            tree.rss_bytes,
            tree.private_bytes
                .map_or_else(|| "unsupported".to_string(), |bytes| bytes.to_string()),
        )),
        None => out.push_str("resources: runner_tree=idle\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_idle_and_unsupported_explicitly() {
        let mut out = String::new();
        render_index_resources(&mut out, &IndexResourceDiagnostics::default());
        assert_eq!(
            out,
            "resources: heavy_held=false heavy_owner=none heavy_kind=none heavy_pending=0 broker_targets=0 queue_depth=0 running=0 follow_ups=0\nresources: runner_tree=idle\n"
        );

        let mut out = String::new();
        render_index_resources(
            &mut out,
            &IndexResourceDiagnostics {
                heavy_held: true,
                heavy_owner_pid: Some(42),
                heavy_holder_kind: Some("index".into()),
                runner_tree: Some(ProcessTreeUsage {
                    process_count: 2,
                    cpu_percent: 12.34,
                    rss_bytes: 100,
                    private_bytes: None,
                }),
                ..IndexResourceDiagnostics::default()
            },
        );
        assert!(out.contains("heavy_owner=42 heavy_kind=index"), "{out}");
        assert!(
            out.contains(
                "runner_tree processes=2 cpu_percent=12.3 rss_bytes=100 private_bytes=unsupported"
            ),
            "{out}"
        );
    }
}
