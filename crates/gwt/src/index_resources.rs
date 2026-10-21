//! Index resource diagnostics (Issue #4527 AC-1, SPEC #1939 AS-33 / FR-425).
//!
//! Measures actual runner process trees from the OS through `sysinfo`, so Windows
//! reports the same CPU / RSS / private-memory fields as POSIX without
//! depending on `ps`. The heavy lease and refresh broker counters come from
//! their existing read-only projections; nothing here redesigns scheduling.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// Combined usage of selected processes and their descendants, counted once.
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

/// The live model-lane holder. Optional runner fields stay unknown for
/// legacy tickets; the querying project's root is never substituted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexRunnerHolder {
    pub repo_hash: String,
    pub owner_pid: Option<u32>,
    pub project_root: Option<PathBuf>,
    pub action: Option<String>,
    pub qos: Option<String>,
    pub model_state: Option<String>,
    pub model_load_ms: Option<u64>,
}

impl std::fmt::Display for IndexRunnerHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "repo_hash={} project_root={} action={} qos={} model_state={} model_load_ms={}",
            self.repo_hash,
            self.project_root
                .as_ref()
                .map_or_else(|| "unknown".to_string(), |root| root.display().to_string(),),
            self.action.as_deref().unwrap_or("unknown"),
            self.qos.as_deref().unwrap_or("unknown"),
            self.model_state.as_deref().unwrap_or("unknown"),
            self.model_load_ms
                .map_or_else(|| "unknown".to_string(), |ms| ms.to_string()),
        )
    }
}

/// Project the existing kernel-backed lease status for both index.status
/// and a refused search. Stale tickets never become live runner holders.
pub(crate) fn index_runner_holder(
    status: &gwt_core::index_coordinator::HeavyLeaseStatus,
) -> Option<IndexRunnerHolder> {
    if !status.held {
        return None;
    }
    let repo_hash = status.target.as_deref()?.split("--").next()?.to_string();
    Some(IndexRunnerHolder {
        repo_hash,
        owner_pid: status.owner.as_ref().map(|owner| owner.pid),
        project_root: status
            .holder_context
            .as_ref()
            .map(|context| context.project_root.clone()),
        action: status
            .holder_context
            .as_ref()
            .map(|context| context.action.clone()),
        qos: status
            .holder_context
            .as_ref()
            .map(|context| context.qos.clone()),
        model_state: status.model_state.clone(),
        model_load_ms: status.model_load_ms,
    })
}

/// Measure `root_pid` and its descendants over one CPU sampling interval.
/// Returns `None` when the root does not exist.
pub fn measure_process_tree(root_pid: u32) -> Option<ProcessTreeUsage> {
    measure_process_trees(Some(root_pid))
}

// All launch paths pass the Python script as argv[1]. Match that argument's
// basename, not mentions of the runner in an agent prompt or shell command.
fn is_index_runner(command: &[OsString]) -> bool {
    let python = command
        .first()
        .and_then(|arg| Path::new(arg).file_stem())
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("python"));
    python
        && command
            .get(1)
            .and_then(|arg| Path::new(arg).file_name())
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name == "chroma_index_runner.py"
                    || name
                        .strip_prefix("chroma_index_runner-")
                        .and_then(|suffix| suffix.strip_suffix(".py"))
                        .is_some_and(|hash| !hash.is_empty())
            })
}

fn runner_tree_pids(processes: &[(Pid, Option<Pid>, bool)]) -> HashSet<Pid> {
    descendant_pids(
        processes
            .iter()
            .filter(|(_, _, runner)| *runner)
            .map(|(pid, _, _)| *pid)
            .collect(),
        processes,
    )
}

fn descendant_pids(mut tree: HashSet<Pid>, processes: &[(Pid, Option<Pid>, bool)]) -> HashSet<Pid> {
    loop {
        let before = tree.len();
        for (pid, parent, _) in processes {
            if parent.is_some_and(|parent| tree.contains(&parent)) {
                tree.insert(*pid);
            }
        }
        if tree.len() == before {
            return tree;
        }
    }
}

/// Without an explicit root, measure all actual runners, including reused
/// search workers whose original lease owner has already exited.
fn measure_process_trees(root_pid: Option<u32>) -> Option<ProcessTreeUsage> {
    let root = root_pid.map(Pid::from_u32);
    let refresh = ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_cmd(UpdateKind::OnlyIfNotSet)
        // Linux otherwise exposes threads as children with duplicate RSS.
        .without_tasks();
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    if let Some(root) = root {
        system.process(root)?;
    }
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    if let Some(root) = root {
        system.process(root)?;
    }

    let snapshot: Vec<_> = system
        .processes()
        .iter()
        .map(|(pid, process)| (*pid, process.parent(), is_index_runner(process.cmd())))
        .collect();
    let tree = match root {
        Some(root) => descendant_pids(HashSet::from([root]), &snapshot),
        None => runner_tree_pids(&snapshot),
    };
    if tree.is_empty() {
        return None;
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

/// Readiness of the resident worker selected by the current search runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryWorkerStatus {
    pub model_state: &'static str,
    pub worker_pid: Option<u32>,
    pub model_load_ms: Option<u64>,
}

impl Default for QueryWorkerStatus {
    fn default() -> Self {
        Self {
            model_state: "cold",
            worker_pid: None,
            model_load_ms: None,
        }
    }
}

fn query_worker_status_at(worker_dir: &Path) -> QueryWorkerStatus {
    let read = || -> Option<QueryWorkerStatus> {
        let endpoint: serde_json::Value =
            serde_json::from_slice(&std::fs::read(worker_dir.join("endpoint.json")).ok()?).ok()?;
        let pid = u32::try_from(endpoint.get("pid")?.as_u64()?).ok()?;
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        );
        system.process(Pid::from_u32(pid))?;
        let model_state = match endpoint.get("model_state")?.as_str()? {
            "cold" => "cold",
            "warming" => "warming",
            "warm" => "warm",
            _ => return None,
        };
        Some(QueryWorkerStatus {
            model_state,
            worker_pid: Some(pid),
            model_load_ms: endpoint
                .get("model_load_ms")
                .and_then(serde_json::Value::as_u64),
        })
    };
    read().unwrap_or_default()
}

/// Host-wide snapshot for `index.status` and `diagnostics cpu --json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct IndexResourceDiagnostics {
    pub query_worker: QueryWorkerStatus,
    pub heavy_held: bool,
    pub heavy_owner_pid: Option<u32>,
    pub heavy_holder_kind: Option<String>,
    pub heavy_holder: Option<IndexRunnerHolder>,
    pub heavy_pending: usize,
    pub broker_targets: usize,
    pub broker_queue_depth: usize,
    pub broker_running: usize,
    /// Targets holding a coalesced follow-up refresh behind a running one.
    pub broker_follow_ups: usize,
    /// Actual index runners and their descendants across the host, independent
    /// of the lease owner's GUI/agent tree. `None` when no runner is visible.
    pub runner_tree: Option<ProcessTreeUsage>,
}

/// Collect the snapshot without creating coordinator or broker state on a
/// host that has none yet.
pub fn collect_index_resources() -> IndexResourceDiagnostics {
    let mut diagnostics = IndexResourceDiagnostics {
        query_worker: query_worker_status_at(&crate::index_search::search_worker_dir()),
        ..Default::default()
    };
    let coordinator_root = gwt_core::index_coordinator::coordinator_root();
    if coordinator_root.is_dir() {
        if let Ok(status) = gwt_core::index_coordinator::IndexCoordinator::open(coordinator_root)
            .and_then(|coordinator| coordinator.heavy_lease_status())
        {
            diagnostics.heavy_held = status.held;
            diagnostics.heavy_pending = status.pending;
            diagnostics.heavy_holder = index_runner_holder(&status);
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
        .and_then(|broker| broker.scan())
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
    diagnostics.runner_tree = measure_process_trees(None);
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
    if let Some(holder) = &diagnostics.heavy_holder {
        out.push_str(&format!("resources: runner_holder {holder}\n"));
    }
    let worker = &diagnostics.query_worker;
    out.push_str(&format!(
        "resources: query_worker model_state={} worker_pid={} model_load_ms={}\n",
        worker.model_state,
        worker
            .worker_pid
            .map_or_else(|| "none".into(), |pid| pid.to_string()),
        worker
            .model_load_ms
            .map_or_else(|| "unknown".into(), |ms| ms.to_string()),
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
    fn query_worker_readiness_is_live_and_rendered() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(query_worker_status_at(temp.path()).model_state, "cold");
        let endpoint = temp.path().join("endpoint.json");
        for state in ["cold", "warming", "warm"] {
            std::fs::write(
                &endpoint,
                serde_json::json!({
                    "pid": std::process::id(), "model_state": state, "model_load_ms": 1234,
                })
                .to_string(),
            )
            .unwrap();
            let status = query_worker_status_at(temp.path());
            assert_eq!(status.model_state, state);
            assert_eq!(status.worker_pid, Some(std::process::id()));
            assert_eq!(status.model_load_ms, Some(1234));
            let mut out = String::new();
            let diagnostics = IndexResourceDiagnostics {
                query_worker: status,
                ..Default::default()
            };
            render_index_resources(&mut out, &diagnostics);
            assert!(
                out.contains(&format!("query_worker model_state={state}")),
                "{out}"
            );
            assert_eq!(
                serde_json::to_value(diagnostics).unwrap()["query_worker"]["model_state"],
                state
            );
        }
        std::fs::write(&endpoint, r#"{"pid":4294967295,"model_state":"warm"}"#).unwrap();
        assert_eq!(query_worker_status_at(temp.path()).model_state, "cold");
        std::fs::remove_file(&endpoint).unwrap();
        assert_eq!(query_worker_status_at(temp.path()).model_state, "cold");
    }

    #[test]
    fn runner_selection_excludes_owner_and_agent_trees_but_includes_reused_workers() {
        let pid = sysinfo::Pid::from_u32;
        let processes = [
            (pid(1), None, false),         // GUI / lease owner
            (pid(2), Some(pid(1)), false), // agent
            (pid(3), Some(pid(2)), false), // cargo
            (pid(4), Some(pid(1)), true),  // runner
            (pid(5), Some(pid(4)), false), // runner child
            (pid(6), None, true),          // reused worker, no longer under owner
            (pid(7), Some(pid(6)), false),
            (pid(8), Some(pid(4)), true), // nested runner must not be counted twice
        ];
        assert_eq!(
            runner_tree_pids(&processes),
            HashSet::from([pid(4), pid(5), pid(6), pid(7), pid(8)])
        );
        assert!(runner_tree_pids(&processes[..3]).is_empty());
    }

    #[test]
    fn runner_identity_uses_the_python_script_argument() {
        let command = |args: &[&str]| {
            args.iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
        };
        for script in [
            "/runtime/chroma_index_runner.py",
            "/runtime/runners/chroma_index_runner-abc123.py",
            "/Users/日本語 user/runtime/chroma_index_runner.py",
        ] {
            assert!(is_index_runner(&command(&[
                "python.exe",
                script,
                "--action",
                "index-files"
            ])));
        }
        for args in [
            vec!["codex", "explain chroma_index_runner.py"],
            vec!["python", "unrelated.py", "chroma_index_runner.py"],
            vec!["python", "chroma_index_runner.py.bak"],
            vec!["codex", "chroma_index_runner.py"],
            vec!["python"],
            vec![],
        ] {
            assert!(!is_index_runner(&command(&args)));
        }
    }

    #[test]
    fn resources_report_the_actual_cross_project_holder_before_runner_spawn() {
        use gwt_core::{
            index_coordinator::{IndexCoordinator, JobAdmission, JobPriority, TargetKey},
            test_support::{env_lock, ScopedEnvVar},
        };
        use std::time::Duration;

        let _env = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("isolated home");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let coordinator = IndexCoordinator::open_default().expect("coordinator");
        let key = TargetKey::repo_shared("other-repo", "issues");
        let JobAdmission::Owner(job) = coordinator
            .request_job(&key, JobPriority::Background, Duration::from_secs(1))
            .expect("target admission")
        else {
            panic!("new target must be owned")
        };
        let mut heavy = job
            .acquire_heavy(Duration::from_secs(1))
            .expect("model lease");
        let legacy = serde_json::to_value(index_runner_holder(
            &coordinator.heavy_lease_status().unwrap(),
        ))
        .unwrap();
        assert_eq!(legacy["repo_hash"], "other-repo");
        assert!(legacy["project_root"].is_null());
        assert!(legacy["action"].is_null());
        assert!(legacy["qos"].is_null());
        // No process is spawned: diagnostics must also cover model startup.
        let project_root = temp.path().join("other-project");
        let lease_id = heavy.id().to_string();
        heavy.record_runner_context(&project_root, "index-issues", "background");
        assert_eq!(
            heavy.id(),
            lease_id,
            "diagnostics must preserve lease identity"
        );
        coordinator
            .write_heavy_progress(&gwt_core::index_coordinator::HeavyProgress {
                target: key.file_stem(),
                done: 0,
                total: 40,
                batch_size: 1,
                batch_ms: 0,
                updated_at_ms: coordinator
                    .heavy_lease_status()
                    .unwrap()
                    .acquired_at_ms
                    .unwrap(),
                model_state: Some("warming".into()),
                model_load_ms: Some(0),
            })
            .unwrap();

        let diagnostics = collect_index_resources();
        let payload = serde_json::to_value(&diagnostics).expect("diagnostics json");
        assert_eq!(payload["heavy_holder"]["repo_hash"], "other-repo");
        assert_eq!(
            payload["heavy_holder"]["project_root"],
            project_root.to_string_lossy().as_ref()
        );
        assert_eq!(payload["heavy_holder"]["action"], "index-issues");
        assert_eq!(payload["heavy_holder"]["qos"], "background");
        assert_eq!(payload["heavy_holder"]["model_state"], "warming");
        assert_eq!(payload["heavy_holder"]["model_load_ms"], 0);
        let mut out = String::new();
        render_index_resources(&mut out, &diagnostics);
        assert!(out.contains("repo_hash=other-repo"), "{out}");
        assert!(out.contains("action=index-issues qos=background"), "{out}");
        assert!(out.contains("model_state=warming model_load_ms=0"), "{out}");
    }

    #[test]
    fn renders_idle_and_unsupported_explicitly() {
        let mut out = String::new();
        render_index_resources(&mut out, &IndexResourceDiagnostics::default());
        assert_eq!(
            out,
            "resources: heavy_held=false heavy_owner=none heavy_kind=none heavy_pending=0 broker_targets=0 queue_depth=0 running=0 follow_ups=0\nresources: query_worker model_state=cold worker_pid=none model_load_ms=unknown\nresources: runner_tree=idle\n"
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
