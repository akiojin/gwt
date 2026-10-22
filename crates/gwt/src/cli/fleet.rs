//! Read-only, project-scoped fleet inventory. Observations never grant authority.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use gwt_agent::Session;
use gwt_github::{ApiError, SpecOpsError};
use serde::Serialize;
use serde_json::{json, Value};

use super::{execution_state, CliEnv};

#[derive(Debug, Clone, Serialize)]
pub(super) struct ProcessObservation {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    /// Native OS process-start identity, paired with PID (not an epoch time).
    pub started_at: Option<u64>,
    pub executable: String,
    pub cwd: Option<PathBuf>,
    pub worktree_path: Option<PathBuf>,
    pub cpu_percent: Option<f32>,
    pub cpu_time_ms: Option<u64>,
    pub elapsed_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SourceStatus {
    pub name: String,
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FleetEntry {
    pub id: String,
    pub correlation: String,
    pub process: Option<ProcessObservation>,
    pub pane: Option<Value>,
    pub monitor: Option<Value>,
    pub session_id: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub branch: Option<String>,
    pub owner_number: Option<u64>,
    pub prompt_kind: String,
    pub duplicate_count: usize,
    pub claim_holder: Option<bool>,
    pub claim: Value,
    pub delivery_state: String,
    pub delivery_evidence: Value,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FleetReport {
    pub project_root: PathBuf,
    pub observed_at: String,
    pub sources: Vec<SourceStatus>,
    pub entries: Vec<FleetEntry>,
}

pub(super) fn run<E: CliEnv>(env: &mut E, out: &mut String) -> Result<i32, SpecOpsError> {
    let report = collect(env.repo_path());
    let output = serde_json::to_string_pretty(&report).map_err(|error| {
        SpecOpsError::Api(ApiError::Parse {
            operation: "fleet.inventory".into(),
            message: error.to_string(),
        })
    })?;
    out.push_str(&output);
    out.push('\n');
    Ok(0)
}

fn source(name: &str, status: &str, detail: Option<String>) -> SourceStatus {
    SourceStatus {
        name: name.into(),
        status: status.into(),
        detail,
    }
}

fn normalized(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn agent_executable(path: &Path) -> Option<&'static str> {
    match path.file_name()?.to_str()?.to_ascii_lowercase().as_str() {
        "codex" | "codex.exe" => Some("codex"),
        "claude" | "claude.exe" => Some("claude"),
        _ => None,
    }
}

fn scoped_worktree(cwd: &Path, worktrees: &[PathBuf]) -> Option<PathBuf> {
    let cwd = normalized(cwd);
    worktrees
        .iter()
        .filter(|root| cwd.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

fn exact_process(process: &ProcessObservation, session: &Value) -> bool {
    let Some(started_at) = process.started_at.filter(|start| *start != 0) else {
        return false;
    };
    (session["child_pid"].as_u64() == Some(process.pid.into())
        && session["child_started_at"].as_u64() == Some(started_at))
        || session["descendant_processes"]
            .as_array()
            .is_some_and(|children| {
                children.iter().any(|child| {
                    child["pid"].as_u64() == Some(process.pid.into())
                        && child["started_at"].as_u64() == Some(started_at)
                })
            })
}

fn same_pane(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    match (left.split_once("::"), right.split_once("::")) {
        (Some(_), Some(_)) => false,
        (Some((_, raw)), None) => raw == right,
        (None, Some((_, raw))) => left == raw,
        _ => false,
    }
}

fn monitor_rows(monitor: &Value) -> Vec<Value> {
    let mut rows = BTreeMap::new();
    for (window, owner) in monitor["launch_bindings"].as_object().into_iter().flatten() {
        if let Some(owner) = owner.as_u64() {
            rows.insert(
                (owner, window.clone()),
                json!({"issue_number":owner,"window_id":window,"source":"persisted_monitor"}),
            );
        }
    }
    for field in [
        "launched_issues",
        "launching_issues",
        "autonomous_records",
        "failed_issues",
        "pending_launch_deliveries",
    ] {
        for row in monitor[field].as_array().into_iter().flatten() {
            let Some(number) = row["issue_number"]
                .as_u64()
                .or_else(|| row["number"].as_u64())
            else {
                continue;
            };
            let window = row["window_id"]
                .as_str()
                .or_else(|| row["active_launch_id"].as_str());
            let key = (number, window.unwrap_or_default().to_string());
            rows.entry(key).or_insert_with(
                || json!({"issue_number":number,"window_id":window,"source":"persisted_monitor"}),
            );
        }
    }
    // A launch and its autonomous state usually describe the same row.
    let with_window: BTreeSet<u64> = rows
        .keys()
        .filter(|(_, window)| !window.is_empty())
        .map(|(owner, _)| *owner)
        .collect();
    rows.retain(|(owner, window), _| !window.is_empty() || !with_window.contains(owner));
    rows.into_values().collect()
}

fn attach_monitor(entry: &mut FleetEntry, rows: &[Value], monitor: &Value) {
    let pane_id = entry.pane.as_ref().and_then(|pane| pane["id"].as_str());
    let matching: Vec<_> = rows
        .iter()
        .filter(|row| {
            pane_id
                .zip(row["window_id"].as_str())
                .is_some_and(|(pane, window)| same_pane(pane, window))
                && entry
                    .owner_number
                    .is_none_or(|owner| row["issue_number"].as_u64() == Some(owner))
        })
        .collect();
    if matching.len() == 1 {
        entry.monitor = Some(matching[0].clone());
        entry.owner_number = entry
            .owner_number
            .or_else(|| matching[0]["issue_number"].as_u64());
    }
    let Some(owner) = entry.owner_number else {
        return;
    };
    let key = owner.to_string();
    let confirmation = &monitor["launch_confirmations"][&key];
    let claim_id = monitor["launched_claims"][&key].as_str();
    let holder = confirmation["window_id"].as_str();
    if let (Some(pane_id), Some(claim_id), Some(holder)) = (pane_id, claim_id, holder) {
        if confirmation["claim_id"].as_str() == Some(claim_id)
            && rows.iter().any(|row| {
                row["issue_number"].as_u64() == Some(owner)
                    && row["window_id"]
                        .as_str()
                        .is_some_and(|window| same_pane(holder, window))
            })
        {
            let is_holder = same_pane(pane_id, holder);
            entry.claim_holder = Some(is_holder);
            entry.claim = json!({"role":if is_holder {"holder"} else {"non_holder"},"claim_id":claim_id,
                "holder_pane_id":holder,"source":"persisted_monitor","authority":false});
        }
    }
}

fn entry(
    process: Option<ProcessObservation>,
    pane: Option<Value>,
    session: Option<&Value>,
) -> FleetEntry {
    let session_id = pane
        .as_ref()
        .and_then(|pane| pane["session_id"].as_str())
        .or_else(|| session.and_then(|session| session["session_id"].as_str()))
        .map(str::to_string);
    let owner_number = pane
        .as_ref()
        .and_then(|pane| pane["linked_issue_number"].as_u64())
        .or_else(|| session.and_then(|session| session["issue_number"].as_u64()));
    let worktree_path = process
        .as_ref()
        .and_then(|process| process.worktree_path.clone())
        .or_else(|| {
            session
                .and_then(|session| session["worktree_path"].as_str())
                .map(PathBuf::from)
        })
        .or_else(|| {
            pane.as_ref()
                .and_then(|pane| pane["worktree_path"].as_str())
                .map(PathBuf::from)
        });
    let pane_id = pane
        .as_ref()
        .and_then(|pane| pane["id"].as_str())
        .unwrap_or("unknown");
    let (id, correlation) = match &process {
        Some(process) => (
            format!(
                "process:{}:{}",
                process.pid,
                process
                    .started_at
                    .map_or_else(|| "unknown".into(), |start| start.to_string())
            ),
            if pane.is_some() {
                "correlated"
            } else {
                "process_only"
            },
        ),
        None => (format!("pane:{pane_id}"), "pane_only"),
    };
    FleetEntry {
        id,
        correlation: correlation.into(),
        process,
        pane,
        monitor: None,
        session_id,
        worktree_path,
        branch: session
            .and_then(|session| session["branch"].as_str())
            .map(str::to_string),
        owner_number,
        prompt_kind: session
            .and_then(|session| session["prompt_kind"].as_str())
            .unwrap_or("unknown")
            .to_string(),
        duplicate_count: 1,
        claim_holder: None,
        claim: json!({"role":"unknown","authority":false}),
        delivery_state: "unknown".into(),
        delivery_evidence: json!({"state":"unknown","reason":"no_delivery_proof"}),
    }
}

fn correlate_inventory(
    processes: Vec<ProcessObservation>,
    mut panes: Vec<Value>,
    sessions: Vec<Value>,
    monitor: &Value,
) -> Vec<FleetEntry> {
    for pane in &mut panes {
        if pane["id"].is_null() {
            pane["id"] = pane["window_id"].clone();
        }
    }
    let rows = monitor_rows(monitor);
    let mut result = Vec::new();
    let mut consumed_panes = BTreeSet::new();
    for process in processes {
        let candidates: Vec<_> = sessions
            .iter()
            .filter(|session| exact_process(&process, session))
            .collect();
        let mut session = (candidates.len() == 1).then(|| candidates[0]);
        let matches: Vec<_> = panes
            .iter()
            .enumerate()
            .filter(|(_, pane)| {
                if pane.get("child_pid").is_some() || pane.get("child_started_at").is_some() {
                    exact_process(&process, pane)
                } else {
                    session
                        .and_then(|session| session["session_id"].as_str())
                        .is_some_and(|session| pane["session_id"].as_str() == Some(session))
                }
            })
            .collect();
        let pane = if matches.len() == 1 {
            consumed_panes.insert(matches[0].0);
            Some(matches[0].1.clone())
        } else {
            None
        };
        if let Some(pane) = &pane {
            if session.is_some_and(|session| session["session_id"] != pane["session_id"]) {
                session = None;
            }
        }
        let mut row = entry(Some(process), pane, session);
        attach_monitor(&mut row, &rows, monitor);
        result.push(row);
    }
    for (index, pane) in panes.into_iter().enumerate() {
        if consumed_panes.contains(&index) {
            continue;
        }
        let session = sessions.iter().find(|session| {
            session["session_id"].as_str().is_some()
                && session["session_id"].as_str() == pane["session_id"].as_str()
        });
        let mut row = entry(None, Some(pane), session);
        attach_monitor(&mut row, &rows, monitor);
        result.push(row);
    }
    for monitor_row in rows {
        if result
            .iter()
            .any(|row| row.monitor.as_ref() == Some(&monitor_row))
        {
            continue;
        }
        let mut row = entry(None, None, None);
        row.owner_number = monitor_row["issue_number"].as_u64();
        row.id = format!(
            "monitor:{}:{}",
            row.owner_number.unwrap_or_default(),
            monitor_row["window_id"].as_str().unwrap_or("unknown")
        );
        row.correlation = "monitor_only".into();
        row.monitor = Some(monitor_row);
        result.push(row);
    }
    let mut counts = BTreeMap::<PathBuf, usize>::new();
    for row in &result {
        if row.process.is_some() {
            if let Some(path) = &row.worktree_path {
                *counts.entry(normalized(path)).or_default() += 1;
            }
        }
    }
    for row in &mut result {
        row.duplicate_count = row
            .worktree_path
            .as_ref()
            .and_then(|path| counts.get(&normalized(path)))
            .copied()
            .unwrap_or(1);
    }
    result
}

fn classify_delivery(proof: &Value) -> &'static str {
    if proof["execution_error"].as_str().is_some() {
        return "unknown";
    }
    if proof["execution_status"] == "active"
        && proof["entry_session_id"].as_str().is_some()
        && proof["entry_session_id"] == proof["holder_session_id"]
    {
        return "undelivered";
    }
    if proof["merged_pr"].as_u64().is_some()
        && proof["merge_sha"]
            .as_str()
            .is_some_and(|sha| !sha.is_empty())
        && proof["merge_in_base"] == true
        && proof["source_surface"] == "zero"
    {
        return "delivered";
    }
    if proof["owner_state"] == "closed" && proof["source_surface"] == "zero" {
        return "delivered";
    }
    if proof["execution_status"] == "active" && proof["holder_session_id"].as_str().is_some() {
        return if proof["holder_connected"] == true
            && proof["entry_session_id"].as_str().is_some()
            && proof["entry_session_id"] != proof["holder_session_id"]
        {
            "recovery_connected"
        } else {
            "undelivered"
        };
    }
    if proof["execution_status"] == "blocked"
        || (proof["pr_state"] == "OPEN" && proof["pr_number"].as_u64().is_some())
    {
        return "undelivered";
    }
    "unknown"
}

/// Preserve successful independent sources when a GUI endpoint or store is unavailable.
pub(super) fn collect(project_root: &Path) -> FleetReport {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let project_root = normalized(&gwt_core::paths::resolve_main_worktree_root(project_root));
    let mut sources = Vec::new();
    let worktrees = match gwt_git::WorktreeManager::new(&project_root).list() {
        Ok(trees) => {
            sources.push(source("worktrees", "ok", None));
            trees
                .into_iter()
                .map(|tree| normalized(&tree.path))
                .collect::<Vec<_>>()
        }
        Err(error) => {
            sources.push(source("worktrees", "partial", Some(error.to_string())));
            vec![project_root.clone()]
        }
    };
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let inventory = crate::session_inventory::observe_sessions(&project_root, &sessions_dir);
    sources.push(source(
        "sessions",
        if inventory.uncertainties.is_empty() {
            "ok"
        } else {
            "partial"
        },
        (!inventory.uncertainties.is_empty()).then(|| {
            inventory
                .uncertainties
                .iter()
                .map(|row| row.reason.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        }),
    ));
    let mut sessions: Vec<Value> = inventory
        .sessions
        .iter()
        .filter_map(|session| serde_json::to_value(session).ok())
        .collect();
    let mut panes = match super::pane::fleet_windows(&project_root.to_string_lossy()) {
        Ok(panes) => {
            sources.push(source("panes", "ok", None));
            panes
        }
        Err(error) => {
            sources.push(source("panes", "unavailable", Some(error)));
            Vec::new()
        }
    };
    // Durable Session metadata is useful even when a pane has no live child.
    for pane in &panes {
        if let Some(id) = pane["session_id"].as_str() {
            if gwt_agent::session::validate_session_id_path_component(id).is_err() {
                continue;
            }
            if !sessions.iter().any(|session| session["session_id"] == id) {
                if let Ok(session) = Session::load(&sessions_dir.join(format!("{id}.toml"))) {
                    if scoped_worktree(&session.worktree_path, &worktrees).is_some() {
                        sessions.push(json!({"session_id":id,"issue_number":session.linked_issue_number,"worktree_path":session.worktree_path}));
                    }
                }
            }
        }
    }
    for observation in &mut sessions {
        if let Some(id) = observation["session_id"].as_str() {
            if gwt_agent::session::validate_session_id_path_component(id).is_err() {
                continue;
            }
            if let Ok(session) = Session::load(&sessions_dir.join(format!("{id}.toml"))) {
                observation["branch"] = json!(session.branch);
                observation["execution_binding"] = json!(session.execution_binding);
                observation["prompt_kind"] = json!(match session.session_mode {
                    gwt_agent::SessionMode::Continue => "continue",
                    gwt_agent::SessionMode::Resume => "resume",
                    _ => "unknown",
                });
            }
        }
    }
    let prefs_path = crate::issue_monitor::issue_monitor_prefs_path_for_repo_path(&project_root);
    let monitor = match crate::issue_monitor::load_issue_monitor_prefs(&prefs_path) {
        Ok(prefs) => {
            sources.push(source(
                "monitor",
                "ok",
                Some(format!(
                    "persisted canonical store; last_scan_at={}",
                    prefs.last_scan_at.as_deref().unwrap_or("unknown")
                )),
            ));
            serde_json::to_value(prefs).unwrap_or_default()
        }
        Err(error) => {
            sources.push(source("monitor", "unavailable", Some(error.to_string())));
            Value::Null
        }
    };
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_cwd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu(),
    );
    for session in sessions.iter_mut().chain(panes.iter_mut()) {
        let root = session["child_pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok());
        let start = session["child_started_at"].as_u64();
        let Some((root, start)) = root.zip(start) else {
            continue;
        };
        if !system
            .process(sysinfo::Pid::from_u32(root))
            .is_some_and(|process| {
                crate::process::snapshot_process_start_identity(root, process) == start
                    && start != 0
            })
        {
            continue;
        }
        let descendants: Vec<_> = system.processes().iter().filter(|(pid, _)| process_descends_from(&system, pid.as_u32(), root))
            .map(|(pid, process)| json!({"pid":pid.as_u32(),"started_at":crate::process::snapshot_process_start_identity(pid.as_u32(), process)})).collect();
        session["descendant_processes"] = json!(descendants);
    }
    let mut unknown_cwd = 0;
    let mut processes = Vec::new();
    for (pid, process) in system.processes() {
        let executable = match process.exe() {
            Some(exe) => agent_executable(exe),
            None => agent_executable(Path::new(process.name())),
        };
        let Some(executable) = executable else {
            continue;
        };
        let start = crate::process::snapshot_process_start_identity(pid.as_u32(), process);
        let mut observation = ProcessObservation {
            pid: pid.as_u32(),
            parent_pid: process.parent().map(|pid| pid.as_u32()),
            started_at: (start != 0).then_some(start),
            executable: executable.into(),
            cwd: process.cwd().map(normalized),
            worktree_path: process
                .cwd()
                .and_then(|cwd| scoped_worktree(cwd, &worktrees)),
            cpu_percent: Some(process.cpu_usage()),
            cpu_time_ms: Some(process.accumulated_cpu_time()),
            elapsed_seconds: Some(process.run_time()),
        };
        if observation.worktree_path.is_none() {
            observation.worktree_path = sessions
                .iter()
                .find(|session| exact_process(&observation, session))
                .and_then(|session| session["worktree_path"].as_str())
                .map(PathBuf::from);
        }
        if observation.worktree_path.is_none() {
            observation.worktree_path = panes
                .iter()
                .find(|pane| exact_process(&observation, pane))
                .and_then(|pane| pane["worktree_path"].as_str())
                .map(PathBuf::from);
        }
        if observation.worktree_path.is_some() {
            processes.push(observation);
        } else if process.cwd().is_none() {
            unknown_cwd += 1;
        }
    }
    processes.sort_by_key(|process| process.pid);
    sources.push(source(
        "processes",
        if system.processes().is_empty() {
            "unavailable"
        } else if unknown_cwd > 0 {
            "partial"
        } else {
            "ok"
        },
        (unknown_cwd > 0).then(|| {
            format!(
                "{unknown_cwd} agent processes could not be attributed because cwd was unavailable"
            )
        }),
    ));
    let mut entries = correlate_inventory(processes, panes, sessions, &monitor);
    enrich_delivery(&project_root, &monitor, &mut entries);
    sources.push(source(
        "delivery",
        "ok",
        Some(
            "canonical local execution and cached owner/source evidence; no fresh GitHub read"
                .into(),
        ),
    ));
    FleetReport {
        project_root,
        observed_at: chrono::Utc::now().to_rfc3339(),
        sources,
        entries,
    }
}

fn process_descends_from(system: &sysinfo::System, pid: u32, root: u32) -> bool {
    let mut current = pid;
    let mut visited = BTreeSet::new();
    while visited.insert(current) {
        if current == root {
            return true;
        }
        let Some(parent) = system
            .process(sysinfo::Pid::from_u32(current))
            .and_then(sysinfo::Process::parent)
        else {
            return false;
        };
        current = parent.as_u32();
    }
    false
}

fn integration_tip(project_root: &Path) -> Option<(String, String)> {
    for base_ref in ["origin/develop", "origin/main", "origin/HEAD"] {
        let output = gwt_core::process::hidden_command("git")
            .arg("-C")
            .arg(project_root)
            .args(["rev-parse", "--verify", &format!("{base_ref}^{{commit}}")])
            .output()
            .ok()?;
        if output.status.success() {
            let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
            if !sha.is_empty() {
                return Some((base_ref.to_string(), sha));
            }
        }
    }
    None
}

fn merge_in_base(project_root: &Path, merge: &str, base: &str) -> Option<bool> {
    if ![40, 64].contains(&merge.len()) || !merge.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let status = gwt_core::process::hidden_command("git")
        .arg("-C")
        .arg(project_root)
        .args(["merge-base", "--is-ancestor", merge, base])
        .status()
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

fn enrich_delivery(project_root: &Path, monitor: &Value, entries: &mut [FleetEntry]) {
    use super::delivered_owner::{self, OwnerLifecycle};
    let base = integration_tip(project_root);
    let owners: BTreeSet<_> = entries.iter().filter_map(|row| row.owner_number).collect();
    let mut proofs = BTreeMap::new();
    for number in owners {
        let mut proof =
            json!({"owner_number":number,"source":"canonical_local_store","authority":false});
        proof["owner_state"] = json!(
            match delivered_owner::owner_lifecycle(project_root, number) {
                OwnerLifecycle::Open => "open",
                OwnerLifecycle::Closed => "closed",
                OwnerLifecycle::Unknown => "unknown",
            }
        );
        if let Some((base_ref, base_sha)) = &base {
            proof["base_ref"] = json!(base_ref);
            proof["base_sha"] = json!(base_sha);
            if let Some(merged) = monitor["merged_issue_settlements"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|merged| merged["issue_number"].as_u64() == Some(number))
                .max_by_key(|merged| merged["settled_at"].as_str())
            {
                proof["merged_pr"] = merged["pr_number"].clone();
                proof["merge_sha"] = merged["merge_sha"].clone();
                proof["merge_in_base"] = json!(merged["merge_sha"]
                    .as_str()
                    .and_then(|merge| merge_in_base(project_root, merge, base_sha)));
                proof["merge_evidence_source"] =
                    json!("persisted_monitor_merge_settlement_and_local_ancestry");
            }
        }
        for kind in [
            execution_state::ExecutionOwnerKind::Issue,
            execution_state::ExecutionOwnerKind::Spec,
        ] {
            match execution_state::load_owner_generation_ledger(
                project_root,
                execution_state::ExecutionOwnerKey { kind, number },
            ) {
                Ok(Some(ledger)) => {
                    if let Some(generation) = ledger.current_generation() {
                        let latest = ledger
                            .lifecycle_events
                            .iter()
                            .filter(|event| {
                                event.generation_id == generation.identity.generation_id
                            })
                            .map(|event| (event.sequence, event.execution_control_json.as_str()))
                            .chain(
                                ledger
                                    .takeovers
                                    .iter()
                                    .filter(|event| {
                                        event.generation_id == generation.identity.generation_id
                                    })
                                    .map(|event| {
                                        (event.sequence, event.execution_control_json.as_str())
                                    }),
                            )
                            .max_by_key(|(sequence, _)| *sequence)
                            .map_or(
                                generation.execution_control_json.as_str(),
                                |(_, projection)| projection,
                            );
                        if let Ok(record) =
                            serde_json::from_str::<execution_state::ExecutionControlRecord>(latest)
                        {
                            if execution_state::integrity_ok(&record) {
                                proof["execution_error"] = Value::Null;
                                proof["execution_status"] =
                                    json!(ledger.current_effective_status());
                                proof["generation_id"] = json!(generation.identity.generation_id);
                                proof["holder_session_id"] = json!(record.primary_session_id);
                                proof["entrypoint"] = json!(generation.identity.entrypoint);
                                let holder_binding_valid = gwt_agent::session::validate_session_id_path_component(&record.primary_session_id).is_ok()
                                    && Session::load(&gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", record.primary_session_id)))
                                        .ok().and_then(|session| session.execution_binding)
                                        .is_some_and(|binding| binding.session_id == record.primary_session_id
                                            && binding.owner_number == number
                                            && binding.owner_kind == kind.as_str()
                                            && execution_state::current_owner_execution_binding(project_root, ledger.owner).ok().flatten()
                                                .is_some_and(|current| current == binding.identity));
                                proof["holder_binding_valid"] = json!(holder_binding_valid);
                                proof["holder_connected"] = json!(
                                    holder_binding_valid
                                        && entries.iter().any(|row| row.session_id.as_deref()
                                            == Some(&record.primary_session_id)
                                            && row.process.is_some()
                                            && row.pane.is_some())
                                );
                            } else {
                                proof["execution_error"] =
                                    json!("canonical execution projection integrity failed");
                            }
                        } else {
                            proof["execution_error"] =
                                json!("canonical execution projection is unreadable");
                        }
                    }
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    proof["execution_error"] = json!(error.to_string());
                }
            }
        }
        proofs.insert(number, proof);
    }
    let mut surfaces = BTreeMap::new();
    for row in entries {
        let mut proof = row
            .owner_number
            .and_then(|owner| proofs.get(&owner))
            .cloned()
            .unwrap_or_else(|| json!({"reason":"owner_unknown"}));
        proof["entry_session_id"] = json!(row.session_id);
        proof["entry_is_execution_holder"] = json!(
            row.session_id.is_some()
                && row.session_id.as_deref() == proof["holder_session_id"].as_str()
        );
        if proof["owner_state"] == "closed" || proof["merged_pr"].as_u64().is_some() {
            if let Some(worktree) = &row.worktree_path {
                let surface = surfaces.entry(worktree.clone()).or_insert_with(|| {
                    base.as_ref()
                        .ok_or_else(|| "integration base is unavailable".to_string())
                        .and_then(|(_, sha)| {
                            super::verify_derivation::changed_source_paths_since(worktree, sha)
                        })
                });
                match surface {
                    Ok(paths) if paths.is_empty() => {
                        proof["source_surface"] = json!("zero");
                    }
                    Ok(paths) => {
                        proof["source_surface"] = json!("present");
                        proof["changed_source_paths"] = json!(paths);
                    }
                    Err(reason) => {
                        proof["source_surface"] = json!("unknown");
                        proof["source_error"] = json!(reason);
                    }
                }
            }
        }
        row.delivery_state = classify_delivery(&proof).into();
        proof["state"] = json!(row.delivery_state);
        row.delivery_evidence = proof;
        if row.prompt_kind == "unknown"
            && row.session_id.as_deref() == row.delivery_evidence["holder_session_id"].as_str()
        {
            row.prompt_kind = row.delivery_evidence["entrypoint"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn correlate_inventory(
        processes: Vec<ProcessObservation>,
        panes: Vec<Value>,
        sessions: Vec<Value>,
        monitor: &Value,
    ) -> Vec<Value> {
        super::correlate_inventory(processes, panes, sessions, monitor)
            .into_iter()
            .map(|row| {
                let mut value = serde_json::to_value(row).unwrap();
                value["delivery"] = value["delivery_evidence"].clone();
                value
            })
            .collect()
    }

    fn process(pid: u32, start: u64, worktree: &str) -> ProcessObservation {
        ProcessObservation {
            pid,
            parent_pid: None,
            started_at: Some(start),
            executable: "codex".to_string(),
            cwd: Some(worktree.into()),
            worktree_path: Some(worktree.into()),
            cpu_percent: None,
            cpu_time_ms: None,
            elapsed_seconds: None,
        }
    }

    #[test]
    fn fleet_executable_selection_ignores_argument_mentions_and_other_projects() {
        assert_eq!(
            agent_executable(std::path::Path::new("/opt/bin/codex")),
            Some("codex")
        );
        assert_eq!(
            agent_executable(std::path::Path::new("claude.exe")),
            Some("claude")
        );
        assert_eq!(agent_executable(std::path::Path::new("/bin/grep")), None);
        assert_eq!(
            agent_executable(std::path::Path::new("claude-helper")),
            None
        );
        let worktrees = vec![std::path::PathBuf::from("/repo/work/one")];
        assert_eq!(
            scoped_worktree(std::path::Path::new("/repo/work/one/src"), &worktrees),
            Some(worktrees[0].clone())
        );
        assert_eq!(
            scoped_worktree(std::path::Path::new("/repo/work/one-other"), &worktrees),
            None
        );
    }

    #[test]
    fn fleet_inventory_keeps_process_only_pane_only_and_monitor_only_rows() {
        let report = correlate_inventory(
            vec![process(10, 100, "/repo/work/one")],
            vec![json!({"id":"pane-only", "session_id":"unobserved", "linked_issue_number":12})],
            vec![],
            &json!({"launched_issues":[{"issue_number":13,"window_id":"monitor-only"}]}),
        );
        assert!(report
            .iter()
            .any(|row| row["correlation"] == "process_only"));
        assert!(report.iter().any(|row| row["correlation"] == "pane_only"));
        assert!(report
            .iter()
            .any(|row| row["correlation"] == "monitor_only"));
        assert!(report
            .iter()
            .all(|row| row["delivery"]["state"] == "unknown"));
    }

    #[test]
    fn fleet_pid_reuse_does_not_attach_a_process_to_an_old_session() {
        let report = correlate_inventory(
            vec![process(10, 200, "/repo/work/one")],
            vec![json!({"id":"old-pane", "session_id":"old"})],
            vec![
                json!({"session_id":"old", "child_pid":10,"child_started_at":100,"worktree_path":"/repo/work/one"}),
            ],
            &json!({}),
        );
        assert_eq!(report.len(), 2);
        let physical = report
            .iter()
            .find(|row| row["process"]["pid"] == 10)
            .unwrap();
        assert!(physical["pane"].is_null());
        assert!(physical["session_id"].is_null());
    }

    #[test]
    fn fleet_exact_wrapper_descendant_correlates_without_pid_only_fallback() {
        let report = correlate_inventory(
            vec![process(11, 101, "/repo/work/one")],
            vec![
                json!({"window_id":"wrapped", "session_id":"a", "child_pid":10,"child_started_at":100,"descendant_processes":[{"pid":11,"started_at":101}]}),
            ],
            vec![
                json!({"session_id":"a","child_pid":10,"child_started_at":100,"worktree_path":"/repo/work/one","descendant_processes":[{"pid":11,"started_at":101}]}),
            ],
            &json!({}),
        );
        assert_eq!(report.len(), 1);
        assert_eq!(report[0]["pane"]["id"], "wrapped");
        assert_eq!(report[0]["session_id"], "a");
    }

    #[test]
    fn fleet_backend_window_id_correlates_process_and_monitor() {
        let report = correlate_inventory(
            vec![process(10, 100, "/repo/work/one")],
            vec![
                json!({"window_id":"backend-pane", "session_id":"a", "linked_issue_number":42,"child_pid":10,"child_started_at":100}),
            ],
            vec![],
            &json!({"launch_bindings":{"backend-pane":42}}),
        );
        assert_eq!(report.len(), 1);
        assert_eq!(report[0]["pane"]["id"], "backend-pane");
        assert_eq!(report[0]["monitor"]["window_id"], "backend-pane");
    }

    #[test]
    fn fleet_live_pane_generation_overrides_a_stale_matching_session() {
        let report = correlate_inventory(
            vec![process(10, 100, "/repo/work/one")],
            vec![
                json!({"window_id":"restarted", "session_id":"a", "child_pid":11,"child_started_at":101}),
            ],
            vec![
                json!({"session_id":"a","child_pid":10,"child_started_at":100,"worktree_path":"/repo/work/one"}),
            ],
            &json!({}),
        );
        assert_eq!(report.len(), 2);
        assert!(report
            .iter()
            .find(|row| row["process"]["pid"] == 10)
            .unwrap()["pane"]
            .is_null());
    }

    #[test]
    fn fleet_duplicates_show_exact_claim_holder_and_non_holder() {
        let report = correlate_inventory(
            vec![
                process(10, 100, "/repo/work/one"),
                process(11, 101, "/repo/work/one"),
            ],
            vec![
                json!({"id":"holder", "session_id":"a", "linked_issue_number":42}),
                json!({"id":"other", "session_id":"b", "linked_issue_number":42}),
            ],
            vec![
                json!({"session_id":"a","child_pid":10,"child_started_at":100,"worktree_path":"/repo/work/one"}),
                json!({"session_id":"b","child_pid":11,"child_started_at":101,"worktree_path":"/repo/work/one"}),
            ],
            &json!({"launched_issues":[{"issue_number":42,"window_id":"holder"}], "launched_claims":{"42":"claim-42"}, "launch_confirmations":{"42":{"window_id":"holder","claim_id":"claim-42"}}}),
        );
        assert_eq!(report.len(), 2);
        assert!(report.iter().all(|row| row["duplicate_count"] == 2));
        assert_eq!(report[0]["claim"]["role"], "holder");
        assert_eq!(report[1]["claim"]["role"], "non_holder");
        assert_eq!(report[1]["claim"]["holder_pane_id"], "holder");
    }

    #[test]
    fn fleet_delivery_requires_base_proof_and_recovery_requires_connection() {
        assert_eq!(
            classify_delivery(&json!({"execution_status":"completed"})),
            "unknown"
        );
        assert_eq!(
            classify_delivery(&json!({"merged_pr":7,"merge_sha":"abc","merge_in_base":false})),
            "unknown"
        );
        assert_eq!(
            classify_delivery(
                &json!({"merged_pr":7,"merge_sha":"abc","merge_in_base":true,"source_surface":"zero","owner_state":"open"})
            ),
            "delivered"
        );
        assert_eq!(
            classify_delivery(
                &json!({"execution_status":"active","holder_session_id":"a","holder_connected":true,"entry_session_id":"old"})
            ),
            "recovery_connected"
        );
        assert_eq!(
            classify_delivery(
                &json!({"execution_status":"active","holder_session_id":"a","holder_connected":false})
            ),
            "undelivered"
        );
        assert_eq!(
            classify_delivery(&json!({"pr_state":"OPEN","pr_number":7})),
            "undelivered"
        );
        assert_eq!(
            classify_delivery(
                &json!({"execution_status":"active","holder_session_id":"a","holder_connected":true,"entry_session_id":"a","owner_state":"closed","source_surface":"zero"})
            ),
            "undelivered"
        );
        assert_eq!(
            classify_delivery(
                &json!({"merged_pr":7,"merge_sha":"abc","merge_in_base":true,"source_surface":"present"})
            ),
            "unknown"
        );
    }
}
