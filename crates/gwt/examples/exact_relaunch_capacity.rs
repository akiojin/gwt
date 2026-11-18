//! Test-only resource fixture for the real PM plus four children browser case.
//! Hardware inputs are fixed; Session/process identity always uses the native census.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fs2::FileExt;
use gwt::session_inventory::{observe_machine_sessions, MachineSessionRole};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let project = dunce::canonicalize(
        std::env::args_os()
            .nth(1)
            .ok_or("fixture project is required")?,
    )?;
    let home = dunce::canonicalize(std::env::var_os("HOME").ok_or("HOME is required")?)?;
    if !project.starts_with(&home)
        || !home.join("provider-argv.jsonl").is_file()
        || gwt_core::paths::gwt_home() != home.join(".gwt")
    {
        return Err("capacity fixture requires an isolated exact-relaunch HOME".into());
    }
    let directory = home.join(".gwt/machine-state");
    std::fs::create_dir_all(&directory)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("agent-capacity.lock"))?;
    lock.try_lock_exclusive()?;
    let (closed, input) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = io::copy(&mut io::stdin(), &mut io::sink());
        let _ = closed.send(());
    });
    let mut first = true;
    loop {
        let inventory = observe_machine_sessions(&gwt_core::paths::gwt_sessions_dir());
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let complete = inventory.uncertainties.is_empty();
        let ram_budget = 128 + inventory.sessions.len();
        let mut roots = BTreeSet::from([project.clone()]);
        let empty_counts = || {
            json!({"total": 0, "pm": 0, "implementation": 0, "review": 0,
            "monitor_workers": 0, "untracked_workers": 0})
        };
        let mut projects = BTreeMap::from([(project.clone(), empty_counts())]);
        for row in &inventory.sessions {
            roots.insert(row.project_root.clone());
            roots.insert(row.session.worktree_path.clone());
            let count = projects
                .entry(row.project_root.clone())
                .or_insert_with(empty_counts);
            count["total"] = json!(count["total"].as_u64().unwrap_or(0) + 1);
            let role = match row.role {
                MachineSessionRole::Pm => "pm",
                MachineSessionRole::Implementation => "implementation",
                MachineSessionRole::Review => "review",
            };
            count[role] = json!(count[role].as_u64().unwrap_or(0) + 1);
            if row.role != MachineSessionRole::Pm {
                let key = if row.monitor_owned {
                    "monitor_workers"
                } else {
                    "untracked_workers"
                };
                count[key] = json!(count[key].as_u64().unwrap_or(0) + 1);
            }
        }
        // The throwaway repositories do not build target artifacts. Keep their
        // paths exact while fixing only the resource model, never the census.
        let targets: BTreeMap<PathBuf, _> = roots
            .iter()
            .map(|root| {
                (
                    root.join("target"),
                    json!({"bytes": 0, "observed_at": now, "refreshed_until": 0}),
                )
            })
            .collect();
        let disk: BTreeMap<PathBuf, _> = roots
            .into_iter()
            .map(|root| {
                (
                    root,
                    json!({"available_bytes": 64_u64 << 30, "observed_at": now}),
                )
            })
            .collect();
        let snapshot = json!({
            "observed_at": now, "expires_at": now + 30, "sample_completed_at": now,
            "performance_cores": 6, "gui_cpu_millicores": 1000,
            "available_ram_bytes": 64_u64 << 30, "per_agent_ram_bytes": 512_u64 << 20,
            "free_disk_bytes": 64_u64 << 30, "targets": targets, "disk_observations": disk,
            "machine_budget": complete.then_some(5), "inventory": inventory, "projects": projects,
            "diagnostics": ["Exact relaunch fixture: fixed hardware inputs; native Session census"],
            "constraints": [
                {"resource": "cpu", "capacity": 5, "binding": complete, "reason": "Fixture P cores 6; GUI 1.00 cores"},
                {"resource": "ram", "capacity": ram_budget, "binding": false, "reason": "Fixture available RAM 64 GiB"},
                {"resource": "disk", "capacity": usize::MAX, "binding": false, "reason": "Fixture target storage 0 bytes"}
            ]
        });
        gwt_github::cache::write_atomic(
            &directory.join("agent-capacity.json"),
            &serde_json::to_vec(&snapshot)?,
        )?;
        if first {
            println!("GWT_CAPACITY_FIXTURE_READY");
            io::stdout().flush()?;
            first = false;
        }
        match input.recv_timeout(Duration::from_secs(1)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            _ => break,
        }
    }
    Ok(())
}
