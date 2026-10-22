//! Explicit, generation-bound fleet disposition; inventory is never authority.

use std::{collections::HashSet, path::Path};

use gwt_github::{ApiError, SpecOpsError};
use serde::Deserialize;
use serde_json::{json, Value};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use super::{CliEnv, CliParseError};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetTarget {
    pub id: String,
    pub session_id: Option<String>,
    pub child_pid: Option<u32>,
    pub child_started_at: Option<u64>,
    pub pid: Option<u32>,
    pub started_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetCommand {
    Inventory,
    Orphans,
    Disposition {
        targets: Vec<FleetTarget>,
        allow_unknown: bool,
        orphan_only: bool,
    },
}

pub(super) fn parse_targets(value: &Value) -> Result<Vec<FleetTarget>, CliParseError> {
    let targets: Vec<FleetTarget> = serde_json::from_value(value.clone())
        .map_err(|error| CliParseError::InvalidJson(format!("invalid fleet targets: {error}")))?;
    if targets.is_empty() || targets.iter().any(|target| target.id.trim().is_empty()) {
        return Err(CliParseError::InvalidJson(
            "fleet targets must be a nonempty explicit set".into(),
        ));
    }
    let mut ids = HashSet::new();
    if targets.iter().any(|target| !ids.insert(&target.id)) {
        return Err(CliParseError::InvalidJson(
            "fleet targets contain duplicate ids".into(),
        ));
    }
    Ok(targets)
}

fn preflight(rows: &[Value], targets: &[FleetTarget], allow_unknown: bool) -> Result<(), String> {
    if !allow_unknown
        && targets.iter().any(|target| {
            rows.iter()
                .find(|row| row["id"] == target.id)
                .is_some_and(|row| row["delivery_state"] == "unknown")
        })
    {
        return Err("fleet disposition refused: target set contains unknown delivery; explicitly set allow_unknown to override".into());
    }
    Ok(())
}

fn validate_target(row: &Value, target: &FleetTarget) -> Result<(), String> {
    if row["claim_holder"] == true
        || row["execution_holder"] == true
        || row["delivery_evidence"]["entry_is_execution_holder"] == true
    {
        return Err("current claim holder is protected; use its owner lifecycle operation".into());
    }
    if row["session_id"].as_str() != target.session_id.as_deref() {
        return Err("target Session changed since selection".into());
    }
    if row["pane"].is_object() {
        if target.session_id.as_deref().is_none_or(str::is_empty) {
            return Err("pane Session identity is unavailable".into());
        }
        if row["pane"]["child_pid"].as_u64() != target.child_pid.map(u64::from)
            || row["pane"]["child_started_at"].as_u64() != target.child_started_at
            || target.child_pid.is_some() != target.child_started_at.is_some()
        {
            return Err("target runtime generation changed since selection".into());
        }
    } else if row["process"].is_object()
        && (row["process"]["pid"].as_u64() != target.pid.map(u64::from)
            || row["process"]["started_at"].as_u64() != target.started_at
            || target.started_at.is_none())
    {
        return Err("target process generation changed or is unavailable".into());
    }
    Ok(())
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: FleetCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if matches!(command, FleetCommand::Inventory) {
        return super::fleet::run(env, out);
    }
    let report = super::fleet::collect(env.repo_path());
    if matches!(command, FleetCommand::Orphans) {
        let entries: Vec<_> = report
            .entries
            .iter()
            .filter(|entry| entry.process.is_some() && entry.pane.is_none())
            .collect();
        out.push_str(&format!(
            "{}\n",
            json!({"sources":report.sources,"orphans":entries})
        ));
        return Ok(0);
    }
    if !super::pm::caller_is_registered_pm(env.repo_path()) {
        return Err(config_error(
            "fleet disposition requires the registered project PM",
        ));
    }
    let FleetCommand::Disposition {
        targets,
        allow_unknown,
        orphan_only,
    } = command
    else {
        unreachable!()
    };
    let rows: Vec<Value> = report
        .entries
        .iter()
        .map(|entry| serde_json::to_value(entry).expect("fleet entry is serializable"))
        .collect();
    preflight(&rows, &targets, allow_unknown).map_err(config_error)?;
    let panes_observed = report
        .sources
        .iter()
        .any(|source| source.name == "panes" && source.status == "ok");
    let receipts: Vec<_> = targets.iter().map(|target| {
        let result = (|| {
            let row = rows.iter().find(|row| row["id"] == target.id).ok_or("target no longer observed")?;
            validate_target(row,target)?;
            if orphan_only && !row["pane"].is_null() { return Err("target currently belongs to a pane".into()); }
            let duplicate_non_holder = row["claim_holder"] == false && row["duplicate_count"].as_u64().is_some_and(|count|count>1);
            if !duplicate_non_holder && !matches!(row["delivery_state"].as_str(),Some("delivered"|"recovery_connected"))
                && !(allow_unknown && row["delivery_state"] == "unknown") {
                return Err("undelivered work is protected".to_string());
            }
            if row["pane"].is_object() {
                let pane_id = row["pane"]["id"].as_str().or_else(|| row["pane"]["window_id"].as_str()).ok_or("pane identity unavailable")?;
                super::pane::fleet_close(pane_id,target.session_id.as_deref().unwrap_or_default(),target.child_pid,target.child_started_at)
            } else if row["process"].is_object() {
                if !panes_observed { return Err("orphan termination requires a complete current pane observation".into()); }
                terminate_orphan(row, target)
            } else {
                Err("Monitor-only rows have no process or pane to close; use issue.monitor lifecycle operations".into())
            }
        })();
        match result {
            Ok(detail) => json!({"id":target.id,"ok":true,"detail":detail}),
            Err(reason) => json!({"id":target.id,"ok":false,"reason":reason}),
        }
    }).collect();
    out.push_str(&format!("{}\n", json!({"receipts":receipts})));
    Ok(0)
}

fn terminate_orphan(row: &Value, target: &FleetTarget) -> Result<Value, String> {
    let pid = target.pid.ok_or("orphan target requires pid")?;
    let start = target
        .started_at
        .ok_or("orphan target requires started_at")?;
    if pid == std::process::id() || crate::process::is_descendant_of(std::process::id(), pid) {
        return Err("the calling PM process tree is protected".into());
    }
    let expected_cwd = row["process"]["cwd"]
        .as_str()
        .ok_or("orphan cwd unavailable")?;
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything().without_tasks(),
    );
    let process = system
        .process(sysinfo::Pid::from_u32(pid))
        .ok_or("orphan no longer exists")?;
    if crate::process::host_process_start_time(pid) != Some(start)
        || process.cwd() != Some(Path::new(expected_cwd))
        || process
            .exe()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_none_or(|name| !matches!(name, "codex" | "claude" | "codex.exe" | "claude.exe"))
    {
        return Err("orphan executable, cwd, or process generation changed".into());
    }
    let mut descendants = vec![pid];
    loop {
        let previous = descendants.len();
        for child in system.processes().values() {
            if child
                .parent()
                .is_some_and(|parent| descendants.contains(&parent.as_u32()))
                && !descendants.contains(&child.pid().as_u32())
            {
                descendants.push(child.pid().as_u32());
            }
        }
        if descendants.len() == previous {
            break;
        }
    }
    let identities: Vec<_> = descendants
        .into_iter()
        .filter_map(|child| {
            if child == pid {
                return Some((pid, start));
            }
            let snapshot = system.process(sysinfo::Pid::from_u32(child))?;
            if crate::process::host_process_start_epoch_secs(child) != Some(snapshot.start_time())
                || crate::process::host_process_start_time(pid) != Some(start)
                || !crate::process::is_descendant_of(child, pid)
            {
                return None;
            }
            let identity = crate::process::snapshot_process_start_identity(child, snapshot);
            (identity > 0).then_some((child, identity))
        })
        .collect();
    let mut failures = Vec::new();
    for (child, identity) in identities.iter().rev() {
        if crate::process::host_process_start_time(*child) != Some(*identity) {
            continue;
        }
        if *child != pid
            && (crate::process::host_process_start_time(pid) != Some(start)
                || !crate::process::is_descendant_of(*child, pid))
        {
            failures.push(*child);
            continue;
        }
        if !system
            .process(sysinfo::Pid::from_u32(*child))
            .is_some_and(|process| process.kill())
        {
            failures.push(*child);
        }
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let survivors = loop {
        let survivors: Vec<_> = identities
            .iter()
            .filter(|(child, identity)| {
                crate::process::host_process_start_time(*child) == Some(*identity)
            })
            .map(|(child, _)| *child)
            .collect();
        if survivors.is_empty() || std::time::Instant::now() >= deadline {
            break survivors;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    if !failures.is_empty() || !survivors.is_empty() {
        return Err(format!("process-tree termination incomplete; signal failures={failures:?}, remaining pids={survivors:?}; re-inventory before retry"));
    }
    Ok(json!({"terminated_pid":pid,"started_at":start}))
}

fn config_error(message: impl Into<String>) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fleet_disposition_unknown_refuses_entire_batch_before_side_effects() {
        let rows = vec![
            json!({"id":"pane:a","delivery_state":"delivered"}),
            json!({"id":"pane:b","delivery_state":"unknown"}),
        ];
        let targets = vec![target("pane:a"), target("pane:b")];
        assert!(preflight(&rows, &targets, false)
            .unwrap_err()
            .contains("unknown"));
        assert!(preflight(&rows, &targets, true).is_ok());
    }

    fn target(id: &str) -> FleetTarget {
        serde_json::from_value(json!({"id":id,"session_id":"s","child_pid":1,"child_started_at":2}))
            .unwrap()
    }

    #[test]
    fn fleet_disposition_rejects_replaced_session_runtime_and_claim_holder() {
        let expected = target("pane:a");
        let mut row = json!({"id":"pane:a","session_id":"s", "claim_holder":false,
            "pane":{"child_pid":1,"child_started_at":2}});
        assert!(validate_target(&row, &expected).is_ok());
        row["session_id"] = json!("successor");
        assert!(validate_target(&row, &expected)
            .unwrap_err()
            .contains("Session"));
        row["session_id"] = json!("s");
        row["pane"]["child_started_at"] = json!(3);
        assert!(validate_target(&row, &expected)
            .unwrap_err()
            .contains("runtime"));
        row["pane"]["child_started_at"] = json!(2);
        row["claim_holder"] = json!(true);
        assert!(validate_target(&row, &expected)
            .unwrap_err()
            .contains("claim"));
    }
}
