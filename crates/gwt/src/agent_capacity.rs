//! Machine-wide measured admission recommendations (SPEC #3200, Issue #3620).

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::session_inventory::{MachineSessionInventory, MachineSessionRole};
use serde::{Deserialize, Serialize};

const SNAPSHOT_TTL_SECS: u64 = 30;
const TARGET_TTL_SECS: u64 = 600;
const RAM_FLOOR_BYTES: u64 = 512 * 1024 * 1024;
const SCAN_ENTRIES_PER_REFRESH: usize = 2_048;
const SCAN_TIME_PER_REFRESH: Duration = Duration::from_millis(20);
const MACHINE_REFRESH_SECS: u64 = 5;
const MAX_REASON_CHARS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CapacityConstraint {
    pub resource: String,
    pub capacity: Option<usize>,
    pub binding: bool,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentCapacity {
    pub measurement_complete: bool,
    pub machine_budget: Option<usize>,
    pub recommended_worker_limit: usize,
    pub recommended_implementation_count: usize,
    pub recommended_total_count: usize,
    pub machine_live_agents: usize,
    pub own_live_agents: usize,
    pub own_pm_agents: usize,
    pub own_implementation_agents: usize,
    pub own_review_agents: usize,
    pub other_live_agents: usize,
    pub other_pm_agents: usize,
    pub other_implementation_agents: usize,
    pub other_review_agents: usize,
    pub gui_cpu_millicores: u64,
    pub constraints: Vec<CapacityConstraint>,
    pub limiting_constraint: String,
    pub reason: String,
    #[serde(default)]
    pub observed_at: Option<u64>,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

impl AgentCapacity {
    pub fn is_fresh(&self) -> bool {
        match (self.observed_at, self.expires_at) {
            (None, None) => self.measurement_complete,
            (Some(observed), Some(expires)) => fresh(observed, expires, epoch_seconds()),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct MachineCapacitySnapshot {
    observed_at: u64,
    expires_at: u64,
    sample_completed_at: u64,
    performance_cores: Option<usize>,
    gui_cpu_millicores: Option<u64>,
    available_ram_bytes: Option<u64>,
    per_agent_ram_bytes: u64,
    free_disk_bytes: Option<u64>,
    targets: BTreeMap<PathBuf, TargetMeasurement>,
    inventory: MachineSessionInventory,
    diagnostics: Vec<String>,
    machine_budget: Option<usize>,
    constraints: Vec<CapacityConstraint>,
    projects: BTreeMap<PathBuf, ProjectConsumption>,
    disk_observations: BTreeMap<PathBuf, DiskMeasurement>,
    gui_identities: BTreeSet<(u32, u64)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProjectConsumption {
    total: usize,
    pm: usize,
    implementation: usize,
    review: usize,
    monitor_workers: usize,
    untracked_workers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TargetMeasurement {
    bytes: u64,
    observed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiskMeasurement {
    available_bytes: u64,
    observed_at: u64,
}

fn merge_disk_observations(
    mut previous: BTreeMap<PathBuf, DiskMeasurement>,
    updates: BTreeMap<PathBuf, Option<u64>>,
    now: u64,
) -> BTreeMap<PathBuf, DiskMeasurement> {
    previous.retain(|_, sample| {
        fresh(
            sample.observed_at,
            sample.observed_at.saturating_add(SNAPSHOT_TTL_SECS),
            now,
        )
    });
    for (path, bytes) in updates {
        if let Some(available_bytes) = bytes {
            previous.insert(
                path,
                DiskMeasurement {
                    available_bytes,
                    observed_at: now,
                },
            );
        } else {
            previous.remove(&path);
        }
    }
    previous
}

fn reusable_machine_sample(observed_at: u64, elapsed: Duration, now: u64) -> bool {
    observed_at <= now
        && now - observed_at < SNAPSHOT_TTL_SECS
        && elapsed < Duration::from_secs(MACHINE_REFRESH_SECS)
}

#[derive(Default)]
struct Inputs {
    cores: Option<usize>,
    gui: Option<u64>,
    ram: Option<u64>,
    ram_per_agent: u64,
    disk: Option<u64>,
    target: Option<u64>,
    live: usize,
    other: usize,
    pm: usize,
    untracked: usize,
    reviews: usize,
    inventory_complete: bool,
}

fn calculate(input: &Inputs) -> AgentCapacity {
    let cpu = input.cores.zip(input.gui).map(|(cores, gui)| {
        ((cores as u64).saturating_mul(1_000).saturating_sub(gui) / 1_000) as usize
    });
    let ram = input.ram.filter(|_| input.ram_per_agent > 0).map(|bytes| {
        input
            .live
            .saturating_add(usize::try_from(bytes / input.ram_per_agent).unwrap_or(usize::MAX))
    });
    let disk = input
        .disk
        .zip(input.target.filter(|bytes| *bytes > 0))
        .map(|(free, target)| {
            input
                .live
                .saturating_add(usize::try_from(free / target).unwrap_or(usize::MAX))
        });
    let complete = input.inventory_complete && cpu.is_some() && ram.is_some() && disk.is_some();
    let budget = complete.then(|| cpu.unwrap().min(ram.unwrap()).min(disk.unwrap()));
    let constraints = [
        (
            "cpu",
            cpu,
            format!(
                "P cores {}; GUI {}",
                input
                    .cores
                    .map(|cores| cores.to_string())
                    .unwrap_or_else(|| "unavailable".to_string()),
                input
                    .gui
                    .map(|cpu| format!("{:.2} cores", cpu as f64 / 1_000.0))
                    .unwrap_or_else(|| "unavailable".to_string())
            ),
        ),
        (
            "ram",
            ram,
            format!(
                "available RAM {}; per-agent {}; live {}",
                format_bytes(input.ram),
                format_bytes(Some(input.ram_per_agent)),
                input.live
            ),
        ),
        (
            "disk",
            disk,
            format!(
                "free disk {}; measured target {}; live {}",
                format_bytes(input.disk),
                format_bytes(input.target),
                input.live
            ),
        ),
    ]
    .into_iter()
    .map(|(resource, capacity, reason)| CapacityConstraint {
        resource: resource.to_string(),
        capacity,
        binding: budget.is_some() && budget == capacity,
        reason,
    })
    .collect::<Vec<_>>();
    let limiting_constraint = constraints
        .iter()
        .filter(|row| row.binding)
        .map(|row| row.resource.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let total = budget.unwrap_or(0).saturating_sub(input.other);
    let pm_reserve = input.pm.max(1);
    let workers = total
        .saturating_sub(pm_reserve)
        .saturating_sub(input.untracked);
    let missing = constraints
        .iter()
        .filter(|row| row.capacity.is_none())
        .map(|row| row.resource.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let reason = format!(
        "{}; other={}; own_pm={}; pm_reserve={}; own_untracked={}; monitor_reviews={}",
        if complete {
            "measured machine budget".to_string()
        } else {
            format!(
                "measurement_unavailable: {}; inventory_complete={}",
                missing, input.inventory_complete
            )
        },
        input.other,
        input.pm,
        pm_reserve,
        input.untracked,
        input.reviews
    );
    AgentCapacity {
        measurement_complete: complete,
        machine_budget: budget,
        recommended_worker_limit: workers,
        recommended_implementation_count: workers.saturating_sub(input.reviews),
        recommended_total_count: total,
        machine_live_agents: input.live,
        own_live_agents: input.live.saturating_sub(input.other),
        own_pm_agents: input.pm,
        other_live_agents: input.other,
        gui_cpu_millicores: input.gui.unwrap_or(0),
        constraints,
        limiting_constraint,
        reason,
        ..AgentCapacity::default()
    }
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn format_bytes(bytes: Option<u64>) -> String {
    bytes
        .map(|bytes| format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0)))
        .unwrap_or_else(|| "unavailable".to_string())
}

fn normalized(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn snapshot_path() -> PathBuf {
    capacity_directory().join("agent-capacity.json")
}

fn capacity_directory() -> PathBuf {
    gwt_core::paths::gwt_home().join("machine-state")
}

fn read_snapshot(path: &Path) -> io::Result<MachineCapacitySnapshot> {
    serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)
}

fn fresh(observed_at: u64, expires_at: u64, now: u64) -> bool {
    observed_at <= now && now < expires_at
}

/// Read the shared census only. Missing, expired or uncertain observations
/// produce a zero Auto cap; callers may offer an explicit manual override.
pub fn project_capacity(
    project_root: &Path,
    monitored_session_ids: &BTreeSet<String>,
    own_monitor_review_count: usize,
) -> AgentCapacity {
    match read_snapshot(&snapshot_path()) {
        Ok(snapshot) => project_capacity_at(
            &snapshot,
            project_root,
            monitored_session_ids,
            own_monitor_review_count,
            epoch_seconds(),
        ),
        Err(error) => AgentCapacity {
            reason: format!("snapshot_unavailable: {error}"),
            ..AgentCapacity::default()
        },
    }
}

fn project_capacity_at(
    snapshot: &MachineCapacitySnapshot,
    project_root: &Path,
    monitored: &BTreeSet<String>,
    reviews: usize,
    now: u64,
) -> AgentCapacity {
    let root = normalized(project_root);
    let repo_hash = gwt_core::repo_hash::detect_repo_identity(&root)
        .map(|identity| identity.hash.as_str().to_string());
    let is_own = |row: &crate::session_inventory::MachineSessionObservation| {
        normalized(&row.project_root) == root
            || repo_hash
                .as_ref()
                .is_some_and(|hash| row.repo_hash.as_ref() == Some(hash))
    };
    let own = snapshot
        .inventory
        .sessions
        .iter()
        .filter(|row| is_own(row))
        .collect::<Vec<_>>();
    let other = snapshot
        .inventory
        .sessions
        .iter()
        .filter(|row| !is_own(row))
        .collect::<Vec<_>>();
    let tracked = |row: &crate::session_inventory::MachineSessionObservation| {
        row.monitor_owned || monitored.contains(&row.session.session_id)
    };
    let count_role = |rows: &[&crate::session_inventory::MachineSessionObservation], role| {
        rows.iter().filter(|row| row.role == role).count()
    };
    let mut own_roots = BTreeSet::from([root.clone()]);
    own_roots.extend(own.iter().map(|row| normalized(&row.session.worktree_path)));
    let own_targets = own_roots
        .iter()
        .map(|root| root.join("target"))
        .collect::<BTreeSet<_>>();
    let disk_observed = own_roots.iter().any(|root| {
        snapshot.disk_observations.get(root).is_some_and(|sample| {
            fresh(
                sample.observed_at,
                sample.observed_at.saturating_add(SNAPSHOT_TTL_SECS),
                now,
            )
        })
    });
    let target = snapshot
        .targets
        .iter()
        .filter(|(path, measurement)| {
            own_targets.contains(*path)
                && measurement.bytes > 0
                && fresh(
                    measurement.observed_at,
                    measurement.observed_at.saturating_add(TARGET_TTL_SECS),
                    now,
                )
        })
        .map(|(_, measurement)| measurement.bytes)
        .max();
    // The largest completed fresh target is conservative across the machine;
    // a requested project still needs its own measured representative first.
    let target = target.map(|own| {
        snapshot
            .targets
            .values()
            .filter(|measurement| {
                fresh(
                    measurement.observed_at,
                    measurement.observed_at.saturating_add(TARGET_TTL_SECS),
                    now,
                )
            })
            .map(|measurement| measurement.bytes)
            .max()
            .unwrap_or(own)
            .max(own)
    });
    let current = fresh(snapshot.observed_at, snapshot.expires_at, now)
        && now.saturating_sub(snapshot.observed_at) < SNAPSHOT_TTL_SECS;
    let mut view = calculate(&Inputs {
        cores: snapshot.performance_cores,
        gui: snapshot.gui_cpu_millicores,
        ram: snapshot.available_ram_bytes,
        ram_per_agent: snapshot.per_agent_ram_bytes,
        disk: snapshot.free_disk_bytes.filter(|_| disk_observed),
        target,
        live: snapshot.inventory.sessions.len(),
        other: other.len(),
        pm: count_role(&own, MachineSessionRole::Pm),
        untracked: own
            .iter()
            .filter(|row| row.role != MachineSessionRole::Pm && !tracked(row))
            .count(),
        reviews: reviews.max(
            own.iter()
                .filter(|row| row.role == MachineSessionRole::Review && tracked(row))
                .count(),
        ),
        inventory_complete: current && snapshot.inventory.uncertainties.is_empty(),
    });
    view.own_implementation_agents = count_role(&own, MachineSessionRole::Implementation);
    view.own_review_agents = count_role(&own, MachineSessionRole::Review);
    view.other_pm_agents = count_role(&other, MachineSessionRole::Pm);
    view.other_implementation_agents = count_role(&other, MachineSessionRole::Implementation);
    view.other_review_agents = count_role(&other, MachineSessionRole::Review);
    view.observed_at = Some(snapshot.observed_at);
    view.expires_at = Some(snapshot.expires_at);
    let mut diagnostics = Vec::new();
    if !current {
        diagnostics.push("snapshot_expired: wait for the next measurement".to_string());
    }
    if !disk_observed || snapshot.free_disk_bytes.is_none() {
        diagnostics.push(
            "project_disk_measurement_unavailable: check volume/path access or use Manual"
                .to_string(),
        );
    }
    if target.is_none() {
        diagnostics.push(
            "target measurement unavailable: wait for the scan; if missing/empty, build the project or use Manual"
                .to_string(),
        );
    }
    // Keep recovery advice and grouped inventory ahead of verbose paths.
    diagnostics.extend(
        snapshot
            .inventory
            .uncertainties
            .iter()
            .map(|uncertainty| format!("inventory_uncertain: {}", uncertainty.reason)),
    );
    if target.is_none() {
        diagnostics.push(format!("target: {}", root.join("target").display()));
    }
    diagnostics.extend(snapshot.diagnostics.iter().cloned());
    view.reason = capacity_reason_summary(view.reason, &diagnostics);
    view
}

fn capacity_reason_summary(mut reason: String, diagnostics: &[String]) -> String {
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic).or_insert(0_usize) += 1;
    }
    for diagnostic in diagnostics {
        let Some(count) = counts.remove(diagnostic) else {
            continue;
        };
        reason.push_str("; ");
        reason.push_str(diagnostic);
        if count > 1 {
            reason.push_str(&format!(" ×{count}"));
        }
    }
    if reason.chars().count() > MAX_REASON_CHARS {
        reason = reason.chars().take(MAX_REASON_CHARS - 1).collect();
        reason.push('…');
    }
    reason
}

/// GUI-independent refresh. CPU baselines and target walks survive ticks;
/// only complete positive target measurements enter the shared atomic file.
pub fn refresh_machine_capacity(project_root: &Path) -> io::Result<()> {
    static PROBE: OnceLock<Mutex<RuntimeProbe>> = OnceLock::new();
    let mut probe = PROBE
        .get_or_init(|| Mutex::new(RuntimeProbe::default()))
        .lock()
        .map_err(|_| io::Error::other("capacity probe poisoned"))?;
    probe.refresh(project_root)
}

#[derive(Default)]
struct RuntimeProbe {
    runtime_dir: PathBuf,
    system: sysinfo::System,
    last_cpu_sample: Option<Instant>,
    walks: BTreeMap<PathBuf, TargetWalk>,
    gui_identities: BTreeSet<(u32, u64)>,
    machine_sample: Option<MachineCapacitySnapshot>,
    machine_sample_completed_at: Option<Instant>,
}

impl RuntimeProbe {
    fn refresh(&mut self, project_root: &Path) -> io::Result<()> {
        use fs2::FileExt;
        let directory = capacity_directory();
        if self.runtime_dir != directory {
            *self = Self {
                runtime_dir: directory.clone(),
                ..Self::default()
            };
        }
        std::fs::create_dir_all(&directory)?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("agent-capacity.lock"))?;
        // A different runtime already refreshing should not block a daemon tick.
        match lock.try_lock_exclusive() {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error),
        }
        let now = epoch_seconds();
        let previous = read_snapshot(&snapshot_path()).unwrap_or_default();
        let elapsed = self
            .machine_sample_completed_at
            .map(|sample| sample.elapsed())
            .unwrap_or(Duration::MAX);
        let local = self
            .machine_sample
            .as_ref()
            .filter(|sample| reusable_machine_sample(sample.observed_at, elapsed, now))
            .cloned();
        let completed_age = Duration::from_secs(now.saturating_sub(previous.sample_completed_at));
        let machine = if let Some(local) = local {
            local
        } else if previous.sample_completed_at > 0
            && previous.sample_completed_at <= now
            && fresh(previous.observed_at, previous.expires_at, now)
            && reusable_machine_sample(previous.observed_at, completed_age, now)
        {
            self.machine_sample_completed_at = Instant::now().checked_sub(completed_age);
            previous.clone()
        } else {
            self.sample_machine(&previous, now)
        };
        self.machine_sample = Some(machine.clone());
        let inventory = machine.inventory;
        let mut roots = BTreeSet::from([normalized(project_root)]);
        roots.extend(
            inventory
                .sessions
                .iter()
                .map(|row| normalized(&row.session.worktree_path)),
        );
        let mut disk_errors = Vec::new();
        let updates = roots
            .iter()
            .map(|root| {
                let available = match fs2::available_space(root) {
                    Ok(bytes) => Some(bytes),
                    Err(error) => {
                        disk_errors.push(format!(
                            "disk_measurement_unavailable: {}: {error}; check volume/path access or use Manual",
                            root.display()
                        ));
                        None
                    }
                };
                (root.clone(), available)
            })
            .collect::<BTreeMap<_, _>>();
        let disk_observations = merge_disk_observations(previous.disk_observations, updates, now);
        let free_disk_bytes = if disk_errors.is_empty() {
            disk_observations
                .values()
                .map(|sample| sample.available_bytes)
                .min()
        } else {
            None
        };
        let mut diagnostics = machine.diagnostics;
        diagnostics.extend(disk_errors);
        let mut targets = previous.targets;
        targets.retain(|_, value| {
            fresh(
                value.observed_at,
                value.observed_at.saturating_add(TARGET_TTL_SECS),
                now,
            )
        });
        let paths = roots
            .into_iter()
            .map(|root| root.join("target"))
            .collect::<BTreeSet<_>>();
        self.walks.retain(|_, walk| {
            walk.last_requested_at <= now
                && now.saturating_sub(walk.last_requested_at) < TARGET_TTL_SECS
        });
        let deadline = Instant::now() + SCAN_TIME_PER_REFRESH;
        let mut remaining = SCAN_ENTRIES_PER_REFRESH;
        for path in paths {
            if targets
                .get(&path)
                .is_some_and(|sample| now < sample.observed_at.saturating_add(TARGET_TTL_SECS / 2))
            {
                continue;
            }
            let walk = self
                .walks
                .entry(path.clone())
                .or_insert_with(|| TargetWalk::new(path.clone()));
            walk.last_requested_at = now;
            match walk.advance(&mut remaining, deadline) {
                Ok(Some(bytes)) => {
                    self.walks.remove(&path);
                    if bytes > 0 {
                        targets.insert(
                            path,
                            TargetMeasurement {
                                bytes,
                                observed_at: now,
                            },
                        );
                    } else {
                        targets.remove(&path);
                        diagnostics.push(format!(
                            "target_measurement_empty: {}; build the project or use Manual",
                            path.display()
                        ));
                    }
                }
                Ok(None) => diagnostics.push(format!(
                    "target_measurement_in_progress: {}; wait for the scan or use Manual",
                    path.display()
                )),
                Err(error) => {
                    self.walks.remove(&path);
                    targets.remove(&path);
                    diagnostics.push(format!(
                        "target_measurement_unavailable: {}: {error}; check path access, build the project or use Manual",
                        path.display()
                    ));
                }
            }
        }
        let mut snapshot = MachineCapacitySnapshot {
            observed_at: machine.observed_at,
            expires_at: machine.expires_at,
            sample_completed_at: machine.sample_completed_at,
            performance_cores: machine.performance_cores,
            gui_cpu_millicores: machine.gui_cpu_millicores,
            gui_identities: machine.gui_identities,
            available_ram_bytes: machine.available_ram_bytes,
            per_agent_ram_bytes: machine.per_agent_ram_bytes,
            free_disk_bytes,
            disk_observations,
            targets,
            inventory,
            diagnostics,
            ..MachineCapacitySnapshot::default()
        };
        let target = snapshot
            .targets
            .values()
            .map(|measurement| measurement.bytes)
            .max();
        let machine = calculate(&Inputs {
            cores: snapshot.performance_cores,
            gui: snapshot.gui_cpu_millicores,
            ram: snapshot.available_ram_bytes,
            ram_per_agent: snapshot.per_agent_ram_bytes,
            disk: snapshot.free_disk_bytes,
            target,
            live: snapshot.inventory.sessions.len(),
            inventory_complete: snapshot.inventory.uncertainties.is_empty(),
            ..Inputs::default()
        });
        snapshot.machine_budget = machine.machine_budget;
        snapshot.constraints = machine.constraints;
        snapshot
            .projects
            .entry(normalized(project_root))
            .or_default();
        for row in &snapshot.inventory.sessions {
            let count = snapshot
                .projects
                .entry(row.project_root.clone())
                .or_default();
            count.total += 1;
            match row.role {
                MachineSessionRole::Pm => count.pm += 1,
                MachineSessionRole::Implementation => count.implementation += 1,
                MachineSessionRole::Review => count.review += 1,
            }
            if row.role != MachineSessionRole::Pm {
                if row.monitor_owned {
                    count.monitor_workers += 1;
                } else {
                    count.untracked_workers += 1;
                }
            }
        }
        let bytes = serde_json::to_vec(&snapshot).map_err(io::Error::other)?;
        gwt_github::cache::write_atomic(&snapshot_path(), &bytes)
    }

    fn sample_machine(
        &mut self,
        previous: &MachineCapacitySnapshot,
        now: u64,
    ) -> MachineCapacitySnapshot {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
        self.system.refresh_memory();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_exe(UpdateKind::OnlyIfNotSet),
        );
        let gui = self
            .system
            .processes()
            .values()
            .filter(|process| {
                is_gui_process(process.exe().unwrap_or_else(|| Path::new(process.name())))
            })
            .collect::<Vec<_>>();
        let current_gui = gui
            .iter()
            .map(|process| {
                (
                    process.pid().as_u32(),
                    crate::process::snapshot_process_start_identity(
                        process.pid().as_u32(),
                        process,
                    ),
                )
            })
            .collect::<BTreeSet<_>>();
        let elapsed = self
            .last_cpu_sample
            .is_some_and(|sample| sample.elapsed() >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        let sampled = cpu_baseline_ready(&self.gui_identities, &current_gui, elapsed);
        let cpu = if gui.is_empty() {
            Some(0)
        } else if sampled {
            let millicores = gui
                .iter()
                .map(|process| f64::from(process.cpu_usage()) * 10.0)
                .sum::<f64>();
            millicores
                .is_finite()
                .then(|| millicores.max(0.0).ceil() as u64)
        } else if previous.gui_identities == current_gui
            && fresh(previous.observed_at, previous.expires_at, now)
        {
            previous.gui_cpu_millicores
        } else {
            None
        };
        self.last_cpu_sample = Some(Instant::now());
        self.gui_identities = current_gui.clone();
        let inventory = crate::session_inventory::observe_machine_sessions(
            &gwt_core::paths::gwt_sessions_dir(),
        );
        let per_agent_ram_bytes = inventory
            .sessions
            .iter()
            .filter_map(|row| row.resident_memory_bytes)
            .max()
            .unwrap_or(RAM_FLOOR_BYTES)
            .max(RAM_FLOOR_BYTES);
        let mut diagnostics = Vec::new();
        if cpu.is_none() {
            diagnostics.push(
                if sampled {
                    "gui_cpu_measurement_unavailable"
                } else {
                    "gui_cpu_warming"
                }
                .to_string(),
            );
        }
        if inventory.sessions.is_empty() {
            diagnostics
                .push("per_agent_ram: 0.5 GiB lower-bound estimate (no live PTY RSS)".to_string());
        }
        self.machine_sample_completed_at = Some(Instant::now());
        MachineCapacitySnapshot {
            observed_at: now,
            expires_at: now.saturating_add(SNAPSHOT_TTL_SECS),
            sample_completed_at: epoch_seconds(),
            performance_cores: performance_cores(),
            gui_cpu_millicores: cpu,
            gui_identities: current_gui,
            available_ram_bytes: (self.system.total_memory() > 0)
                .then(|| self.system.available_memory()),
            per_agent_ram_bytes,
            inventory,
            diagnostics,
            ..MachineCapacitySnapshot::default()
        }
    }
}

fn is_gui_process(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("gwt"))
}

fn cpu_baseline_ready(
    previous: &BTreeSet<(u32, u64)>,
    current: &BTreeSet<(u32, u64)>,
    elapsed: bool,
) -> bool {
    elapsed && current.iter().all(|(_, started)| *started > 0) && current.is_subset(previous)
}

fn performance_cores() -> Option<usize> {
    select_core_budget(
        sysctl_cores,
        std::thread::available_parallelism().ok().map(usize::from),
    )
}

fn select_core_budget(
    mut probe: impl FnMut(&str) -> Option<usize>,
    logical: Option<usize>,
) -> Option<usize> {
    probe("hw.perflevel0.logicalcpu")
        .filter(|value| *value > 0)
        .or_else(|| probe("hw.ncpu").filter(|value| *value > 0))
        .or(logical.filter(|value| *value > 0))
}

fn sysctl_cores(key: &str) -> Option<usize> {
    #[cfg(target_os = "macos")]
    {
        let key = std::ffi::CString::new(key).ok()?;
        let mut value = 0_u32;
        let mut length = std::mem::size_of_val(&value);
        // SAFETY: NUL-terminated key and a correctly sized writable u32.
        let result = unsafe {
            libc::sysctlbyname(
                key.as_ptr(),
                (&mut value as *mut u32).cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        (result == 0 && length == std::mem::size_of_val(&value) && value > 0)
            .then_some(value as usize)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = key;
        None
    }
}

struct TargetWalk {
    root: PathBuf,
    pending: Vec<PathBuf>,
    current: Option<std::fs::ReadDir>,
    bytes: u64,
    last_requested_at: u64,
}

impl TargetWalk {
    fn new(path: PathBuf) -> Self {
        Self {
            root: path.clone(),
            pending: vec![path],
            current: None,
            bytes: 0,
            last_requested_at: 0,
        }
    }

    fn advance(&mut self, remaining: &mut usize, deadline: Instant) -> io::Result<Option<u64>> {
        loop {
            if *remaining == 0 || Instant::now() >= deadline {
                return Ok(None);
            }
            if self.current.is_none() {
                let Some(path) = self.pending.pop() else {
                    // An open directory iterator can outlive a removed target.
                    // Never publish its accumulated bytes as a current sample.
                    if !std::fs::metadata(&self.root)?.is_dir() {
                        return Err(io::Error::from(io::ErrorKind::NotADirectory));
                    }
                    return Ok(Some(self.bytes));
                };
                *remaining -= 1;
                self.current = match std::fs::read_dir(&path) {
                    Ok(entries) => Some(entries),
                    Err(error) if error.kind() == io::ErrorKind::NotFound && path != self.root => {
                        continue;
                    }
                    Err(error) => return Err(error),
                };
            }
            let Some(entry) = self.current.as_mut().unwrap().next() else {
                self.current = None;
                continue;
            };
            *remaining = remaining.saturating_sub(1);
            // Cargo may remove intermediate artifacts between bounded ticks.
            // Disappeared children contribute zero; other I/O errors invalidate
            // the walk rather than turning a partial scan into a measurement.
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if kind.is_dir() {
                self.pending.push(entry.path());
            } else if kind.is_file() {
                match entry.metadata() {
                    Ok(metadata) => self.bytes = self.bytes.saturating_add(metadata.len()),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured() -> Inputs {
        Inputs {
            cores: Some(8),
            gui: Some(0),
            ram: Some(80),
            ram_per_agent: 4,
            disk: Some(100),
            target: Some(5),
            inventory_complete: true,
            ..Inputs::default()
        }
    }

    #[test]
    fn recommends_pcores_minus_one_with_one_resident_pm_and_review_consumption() {
        let absent = calculate(&measured());
        assert_eq!(absent.machine_budget, Some(8));
        assert_eq!(absent.recommended_worker_limit, 7);
        let resident = calculate(&Inputs {
            live: 3,
            pm: 1,
            reviews: 1,
            ..measured()
        });
        assert_eq!(
            resident.recommended_worker_limit, 7,
            "existing PM is deducted once"
        );
        assert_eq!(resident.recommended_implementation_count, 6);
    }

    #[test]
    fn normalizes_free_resources_to_machine_total_before_other_project_deductions() {
        let view = calculate(&Inputs {
            live: 5,
            other: 3,
            pm: 1,
            ram: Some(8),
            ..measured()
        });
        assert_eq!(view.machine_budget, Some(7));
        assert_eq!(view.recommended_worker_limit, 3);
        assert_eq!(view.limiting_constraint, "ram");
        assert!(view.reason.contains("other=3"));
    }

    #[test]
    fn measured_gui_cpu_disk_and_untracked_panes_reduce_recommendation() {
        let view = calculate(&Inputs {
            gui: Some(1_250),
            live: 2,
            untracked: 2,
            ..measured()
        });
        assert_eq!(view.machine_budget, Some(6));
        assert_eq!(view.recommended_worker_limit, 3);
        assert_eq!(view.limiting_constraint, "cpu");
        let disk = calculate(&Inputs {
            disk: Some(10),
            ..measured()
        });
        assert_eq!(disk.machine_budget, Some(2));
        assert_eq!(disk.recommended_worker_limit, 1);
        assert_eq!(disk.limiting_constraint, "disk");
    }

    #[test]
    fn zero_and_missing_measurements_fail_closed_without_clamping_to_one() {
        let zero = calculate(&Inputs {
            gui: Some(8_000),
            ..measured()
        });
        assert_eq!(zero.machine_budget, Some(0));
        assert!(zero.measurement_complete);
        assert_eq!(zero.recommended_worker_limit, 0);
        for missing in [
            Inputs {
                target: None,
                ..measured()
            },
            Inputs {
                target: Some(0),
                ..measured()
            },
            Inputs {
                gui: None,
                ..measured()
            },
            Inputs {
                inventory_complete: false,
                ..measured()
            },
        ] {
            let view = calculate(&missing);
            assert!(!view.measurement_complete);
            assert_eq!(view.machine_budget, None);
            assert_eq!(view.recommended_worker_limit, 0);
            assert!(!view.reason.is_empty());
        }
    }

    #[test]
    fn all_equal_machine_constraints_remain_visible_as_binding() {
        let view = calculate(&Inputs {
            cores: Some(2),
            ram: Some(8),
            disk: Some(10),
            ..measured()
        });
        assert_eq!(view.constraints.iter().filter(|row| row.binding).count(), 3);
        assert_eq!(view.limiting_constraint, "cpu,ram,disk");
    }

    fn scan_target(path: &Path, entry_budget: usize) -> Option<u64> {
        let mut walk = TargetWalk::new(path.to_path_buf());
        for _ in 0..100 {
            let mut budget = entry_budget;
            if let Some(bytes) = walk
                .advance(&mut budget, Instant::now() + Duration::from_secs(1))
                .unwrap()
            {
                return Some(bytes);
            }
        }
        None
    }

    #[test]
    fn target_measurement_resumes_small_budgets_and_ignores_symlinked_children() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..20 {
            let directory = temp.path().join(index.to_string());
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("artifact"), [0_u8; 8]).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(temp.path(), temp.path().join("cycle")).unwrap();
        assert_eq!(scan_target(temp.path(), 2), Some(160));
    }

    #[test]
    fn target_measurement_skips_a_child_removed_between_refreshes() {
        let temp = tempfile::tempdir().unwrap();
        let removed = temp.path().join("removed");
        std::fs::create_dir(&removed).unwrap();
        std::fs::write(removed.join("artifact"), [0_u8; 9]).unwrap();
        std::fs::write(temp.path().join("survivor"), [0_u8; 7]).unwrap();
        let mut walk = TargetWalk::new(temp.path().to_path_buf());
        while !walk.pending.contains(&removed) {
            assert_eq!(
                walk.advance(&mut 1, Instant::now() + Duration::from_secs(1))
                    .unwrap(),
                None
            );
        }
        std::fs::remove_dir_all(&removed).unwrap();
        assert_eq!(
            walk.advance(&mut 100, Instant::now() + Duration::from_secs(1))
                .unwrap(),
            Some(7)
        );
    }

    #[test]
    fn target_measurement_rejects_root_removed_before_completion() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("target");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("artifact"), [0_u8; 9]).unwrap();
        let mut walk = TargetWalk::new(root.clone());
        assert_eq!(
            walk.advance(&mut 2, Instant::now() + Duration::from_secs(1))
                .unwrap(),
            None
        );
        assert_eq!(walk.bytes, 9);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            walk.advance(&mut 100, Instant::now() + Duration::from_secs(1))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn absent_shared_snapshot_is_diagnostic_and_never_creates_runtime_files() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let view = project_capacity(temp.path(), &BTreeSet::new(), 0);
        assert_eq!(view.recommended_worker_limit, 0);
        assert!(view.reason.contains("snapshot_unavailable"));
        assert!(!gwt_core::paths::gwt_runtime_dir().exists());
    }

    fn snapshot(root: &Path) -> MachineCapacitySnapshot {
        MachineCapacitySnapshot {
            observed_at: 100,
            expires_at: 130,
            performance_cores: Some(8),
            gui_cpu_millicores: Some(0),
            available_ram_bytes: Some(80),
            per_agent_ram_bytes: 4,
            free_disk_bytes: Some(100),
            disk_observations: BTreeMap::from([(
                normalized(root),
                DiskMeasurement {
                    available_bytes: 100,
                    observed_at: 100,
                },
            )]),
            targets: BTreeMap::from([(
                normalized(root).join("target"),
                TargetMeasurement {
                    bytes: 5,
                    observed_at: 100,
                },
            )]),
            ..MachineCapacitySnapshot::default()
        }
    }

    #[test]
    fn snapshot_and_target_deadlines_are_revalidated_on_each_read() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot = snapshot(temp.path());
        let fresh = project_capacity_at(&snapshot, temp.path(), &BTreeSet::new(), 0, 101);
        assert!(fresh.measurement_complete);
        assert_eq!(fresh.recommended_worker_limit, 7);
        let stale = project_capacity_at(&snapshot, temp.path(), &BTreeSet::new(), 0, 130);
        assert!(!stale.measurement_complete);
        assert_eq!(stale.recommended_worker_limit, 0);
        assert!(stale.reason.contains("snapshot_expired"));
        let mut snapshot = snapshot;
        snapshot.observed_at = 800;
        snapshot.expires_at = 830;
        let target_stale = project_capacity_at(&snapshot, temp.path(), &BTreeSet::new(), 0, 801);
        assert!(!target_stale.measurement_complete);
        assert_eq!(target_stale.recommended_worker_limit, 0);
        let retained_view = AgentCapacity {
            measurement_complete: true,
            observed_at: Some(1),
            expires_at: Some(2),
            ..AgentCapacity::default()
        };
        assert!(!retained_view.is_fresh());
    }

    fn row(
        root: &Path,
        id: &str,
        role: MachineSessionRole,
        monitored: bool,
    ) -> crate::session_inventory::MachineSessionObservation {
        crate::session_inventory::MachineSessionObservation {
            project_root: root.to_path_buf(),
            repo_hash: None,
            role,
            monitor_owned: monitored,
            resident_memory_bytes: Some(1),
            session: crate::session_inventory::SessionObservation {
                session_id: id.to_string(),
                issue_number: None,
                execution_binding_missing: false,
                agent_id: "codex".to_string(),
                worktree_path: root.to_path_buf(),
                worktree_exists: true,
                host_pid: 1,
                child_pid: 2,
                child_started_at: 3,
                started_at: "epoch".to_string(),
                launch_origin: gwt_agent::SessionLaunchOrigin::Unknown,
                restore_source_session_id: None,
            },
        }
    }

    #[test]
    fn canonical_monitor_provenance_and_observed_reviews_work_without_ui_binding_ids() {
        let temp = tempfile::tempdir().unwrap();
        let own = temp.path().join("own");
        let foreign = temp.path().join("other");
        let mut snapshot = snapshot(&own);
        snapshot.inventory.sessions = vec![
            row(&own, "pm", MachineSessionRole::Pm, false),
            row(&own, "review", MachineSessionRole::Review, true),
            row(&own, "manual", MachineSessionRole::Implementation, false),
            row(&foreign, "foreign-pm", MachineSessionRole::Pm, false),
            row(
                &foreign,
                "foreign-worker",
                MachineSessionRole::Implementation,
                true,
            ),
            row(&foreign, "foreign-review", MachineSessionRole::Review, true),
        ];
        let view = project_capacity_at(&snapshot, &own, &BTreeSet::new(), 0, 101);
        assert_eq!(view.machine_live_agents, 6);
        assert_eq!(view.other_live_agents, 3);
        assert_eq!(view.own_pm_agents, 1);
        assert_eq!(view.own_review_agents, 1);
        assert_eq!(view.other_pm_agents, 1);
        assert_eq!(view.other_implementation_agents, 1);
        assert_eq!(view.other_review_agents, 1);
        assert_eq!(view.recommended_total_count, 5);
        assert_eq!(view.recommended_worker_limit, 3);
        assert_eq!(view.recommended_implementation_count, 2);
        let tracked_manual = project_capacity_at(
            &snapshot,
            &own,
            &BTreeSet::from(["manual".to_string()]),
            0,
            101,
        );
        assert_eq!(tracked_manual.recommended_worker_limit, 4);
    }

    #[test]
    fn other_project_live_panes_reduce_capacity_and_release_restores_it() {
        let temp = tempfile::tempdir().unwrap();
        let own = temp.path().join("own");
        let other = temp.path().join("other");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let own = normalized(&own);
        let other = normalized(&other);
        let mut snapshot = snapshot(&own);
        snapshot.inventory.sessions = vec![
            row(&own, "own-pm", MachineSessionRole::Pm, false),
            row(&own, "own-worker", MachineSessionRole::Implementation, true),
        ];
        // Free resources cover additional panes. Keep their normalized
        // machine totals at 20 while three other panes consume and release them.
        let free_ram = 80 - 2 * snapshot.per_agent_ram_bytes;
        let free_disk = 100 - 2 * 5;
        snapshot.available_ram_bytes = Some(free_ram);
        snapshot.free_disk_bytes = Some(free_disk);
        snapshot
            .disk_observations
            .get_mut(&own)
            .unwrap()
            .available_bytes = free_disk;
        let baseline = project_capacity_at(&snapshot, &own, &BTreeSet::new(), 0, 101);
        assert!(baseline.measurement_complete);
        assert_eq!(baseline.machine_live_agents, 2);
        assert_eq!(baseline.other_live_agents, 0);
        assert_eq!(baseline.recommended_worker_limit, 7);

        snapshot.inventory.sessions.extend([
            row(&other, "other-pm", MachineSessionRole::Pm, false),
            row(
                &other,
                "other-worker",
                MachineSessionRole::Implementation,
                true,
            ),
            row(&other, "other-review", MachineSessionRole::Review, true),
        ]);
        snapshot.available_ram_bytes = Some(free_ram - 3 * snapshot.per_agent_ram_bytes);
        snapshot.free_disk_bytes = Some(free_disk - 3 * 5);
        snapshot
            .disk_observations
            .get_mut(&own)
            .unwrap()
            .available_bytes = free_disk - 3 * 5;
        let occupied = project_capacity_at(&snapshot, &own, &BTreeSet::new(), 0, 101);
        assert_eq!(occupied.machine_live_agents, 5);
        assert_eq!(occupied.other_live_agents, 3);
        assert_eq!(occupied.own_live_agents, baseline.own_live_agents);
        assert_eq!(occupied.machine_budget, baseline.machine_budget);
        assert_eq!(occupied.recommended_worker_limit, 4);
        assert!(occupied.recommended_worker_limit < baseline.recommended_worker_limit);
        for view in [&baseline, &occupied] {
            assert_eq!(
                view.constraints
                    .iter()
                    .map(|row| row.capacity)
                    .collect::<Vec<_>>(),
                vec![Some(8), Some(20), Some(20)],
                "physical resource budgets must not change with pane occupancy"
            );
        }

        snapshot
            .inventory
            .sessions
            .retain(|row| row.project_root == own);
        snapshot.available_ram_bytes = Some(free_ram);
        snapshot.free_disk_bytes = Some(free_disk);
        snapshot
            .disk_observations
            .get_mut(&own)
            .unwrap()
            .available_bytes = free_disk;
        let released = project_capacity_at(&snapshot, &own, &BTreeSet::new(), 0, 101);
        assert_eq!(released.other_live_agents, 0);
        assert_eq!(
            released, baseline,
            "ending other panes restores the original recommendation"
        );
    }

    #[test]
    fn twenty_thousand_registered_projects_do_not_consume_machine_capacity() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let own = temp.path().join("own");
        std::fs::create_dir_all(&own).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        std::fs::create_dir_all(sessions.join("runtime")).unwrap();
        let mut snapshot = snapshot(&own);
        snapshot.inventory = crate::session_inventory::observe_machine_sessions(&sessions);
        assert_eq!(snapshot.inventory, MachineSessionInventory::default());
        let baseline = project_capacity_at(&snapshot, &own, &BTreeSet::new(), 0, 101);
        assert!(baseline.measurement_complete);
        assert_eq!(baseline.machine_budget, Some(8));
        assert_eq!(baseline.recommended_worker_limit, 7);

        let projects = gwt_core::paths::gwt_projects_dir();
        std::fs::create_dir_all(&projects).unwrap();
        for index in 0..20_000 {
            std::fs::create_dir(projects.join(format!("{index:016x}"))).unwrap();
        }
        assert_eq!(std::fs::read_dir(&projects).unwrap().count(), 20_000);
        let registered = crate::session_inventory::observe_machine_sessions(&sessions);
        assert_eq!(
            registered, snapshot.inventory,
            "registration alone creates no live PTY consumption"
        );
        snapshot.inventory = registered;
        let after_registration = project_capacity_at(&snapshot, &own, &BTreeSet::new(), 0, 101);
        assert_eq!(
            after_registration, baseline,
            "registered projects leave the shared budget and consumption unchanged"
        );
    }

    #[test]
    fn repeated_capacity_diagnostics_are_grouped_and_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("long-project-name-".repeat(5));
        std::fs::create_dir(&root).unwrap();
        let mut snapshot = snapshot(&root);
        snapshot.inventory.uncertainties = (0..37)
            .map(
                |_| crate::session_inventory::SessionObservationUncertainty {
                    runtime_path: PathBuf::new(),
                    session_id: None,
                    reason: "child_identity_missing".to_string(),
                },
            )
            .collect();
        let view = project_capacity_at(&snapshot, &root, &BTreeSet::new(), 0, 101);
        assert!(view
            .reason
            .contains("inventory_uncertain: child_identity_missing ×37"));
        assert_eq!(view.reason.matches("child_identity_missing").count(), 1);
        snapshot.diagnostics = (0..40)
            .map(|index| format!("cause-{index}: {}", "測".repeat(80)))
            .collect();
        snapshot.free_disk_bytes = None;
        snapshot.targets.clear();
        let view = project_capacity_at(&snapshot, &root, &BTreeSet::new(), 0, 101);
        assert!(view.reason.chars().count() <= 512);
        assert!(view.reason.ends_with('…'));
        assert!(view.reason.contains("child_identity_missing ×37"));
        assert!(view.reason.contains("build the project or use Manual"));
    }

    #[test]
    fn unavailable_disk_slots_explain_pending_or_missing_target_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let mut snapshot = snapshot(temp.path());
        snapshot.targets.clear();
        snapshot.diagnostics = vec!["target_measurement_in_progress".to_string()];
        let pending = project_capacity_at(&snapshot, temp.path(), &BTreeSet::new(), 0, 101);
        assert!(!pending.measurement_complete);
        assert!(pending.reason.contains("wait"), "{}", pending.reason);
        snapshot.diagnostics.clear();
        let missing = project_capacity_at(&snapshot, temp.path(), &BTreeSet::new(), 0, 101);
        assert!(missing.reason.contains("build"), "{}", missing.reason);
        assert!(missing.reason.contains("Manual"), "{}", missing.reason);
        // Windows temp paths may use an 8.3 alias; recovery text uses the normalized root.
        assert!(
            missing
                .reason
                .contains(&normalized(temp.path()).join("target").display().to_string()),
            "{}",
            missing.reason
        );
    }

    #[test]
    fn uncertain_machine_inventory_never_becomes_available_capacity() {
        let temp = tempfile::tempdir().unwrap();
        let mut snapshot = snapshot(temp.path());
        snapshot.inventory.uncertainties.push(
            crate::session_inventory::SessionObservationUncertainty {
                runtime_path: PathBuf::new(),
                session_id: None,
                reason: "foreign_child_unknown".to_string(),
            },
        );
        let view = project_capacity_at(&snapshot, temp.path(), &BTreeSet::new(), 0, 101);
        assert_eq!(view.recommended_worker_limit, 0);
        assert!(!view.measurement_complete);
        assert!(view.reason.contains("foreign_child_unknown"));
    }

    #[test]
    fn shared_refresh_measures_target_and_writes_an_expiring_snapshot_without_ui_clients() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let root = temp.path().join("project");
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/artifact"), [0_u8; 8]).unwrap();
        let root = normalized(&root);
        refresh_machine_capacity(&root).unwrap();
        let snapshot = read_snapshot(&snapshot_path()).unwrap();
        assert_eq!(snapshot.targets[&root.join("target")].bytes, 8);
        assert_eq!(snapshot.per_agent_ram_bytes, RAM_FLOOR_BYTES);
        assert!(snapshot.inventory.sessions.is_empty());
        assert_eq!(
            snapshot.expires_at - snapshot.observed_at,
            SNAPSHOT_TTL_SECS
        );
        assert!(snapshot.available_ram_bytes.is_some());
        assert!(snapshot.free_disk_bytes.is_some());
        assert!(snapshot.performance_cores.is_some());
        let wire: serde_json::Value =
            serde_json::from_slice(&std::fs::read(snapshot_path()).unwrap()).unwrap();
        assert!(
            wire.get("machine_budget").is_some(),
            "shared file carries the derived physical budget"
        );
        assert!(wire.get("constraints").is_some());
        assert!(
            wire.get("projects").is_some(),
            "shared file carries explicit live project consumption"
        );
    }

    #[test]
    fn cpu_sensor_selects_only_gui_roots() {
        assert!(is_gui_process(Path::new(
            "/Applications/GWT.app/Contents/MacOS/gwt"
        )));
        for name in ["gwtd", "gwt-helper", "codex", "cargo"] {
            assert!(!is_gui_process(Path::new(name)));
        }
    }

    #[test]
    fn alternating_open_projects_keep_unfinished_target_walks() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir_all(first.join("target")).unwrap();
        std::fs::create_dir_all(second.join("target")).unwrap();
        let first = normalized(&first);
        let second = normalized(&second);
        for index in 0..SCAN_ENTRIES_PER_REFRESH + 10 {
            std::fs::write(first.join("target").join(index.to_string()), [0_u8; 1]).unwrap();
        }
        std::fs::write(second.join("target/artifact"), [0_u8; 1]).unwrap();
        let mut probe = RuntimeProbe::default();
        probe.refresh(&first).unwrap();
        assert!(probe.walks.contains_key(&first.join("target")));
        probe.refresh(&second).unwrap();
        let snapshot = read_snapshot(&snapshot_path()).unwrap();
        assert!(
            probe.walks.contains_key(&first.join("target"))
                || snapshot.targets.contains_key(&first.join("target")),
            "switching requested projects must resume or complete the existing walk"
        );
    }

    #[test]
    fn performance_core_probe_uses_apple_primary_then_intel_logical_fallback() {
        let hardware = BTreeMap::from([
            ("hw.perflevel0.logicalcpu", 8),
            ("hw.ncpu", 12),
            ("hw.physicalcpu", 6),
        ]);
        assert_eq!(
            select_core_budget(|key| hardware.get(key).copied(), Some(24)),
            Some(8)
        );
        let intel = BTreeMap::from([("hw.ncpu", 12), ("hw.physicalcpu", 6)]);
        assert_eq!(
            select_core_budget(|key| intel.get(key).copied(), Some(24)),
            Some(12)
        );
        assert_eq!(select_core_budget(|_| None, Some(24)), Some(24));
        assert_eq!(select_core_budget(|_| Some(0), None), None);
    }

    #[test]
    fn newly_detected_gui_process_requires_its_own_second_cpu_sample() {
        let prior = BTreeSet::from([(1, 10)]);
        assert!(cpu_baseline_ready(&prior, &prior, true));
        assert!(!cpu_baseline_ready(
            &prior,
            &BTreeSet::from([(1, 10), (2, 20)]),
            true
        ));
        assert!(!cpu_baseline_ready(
            &prior,
            &BTreeSet::from([(1, 11)]),
            true
        ));
        assert!(!cpu_baseline_ready(&prior, &prior, false));
        assert!(!cpu_baseline_ready(&BTreeSet::new(), &prior, true));
    }

    #[test]
    fn fresh_disk_observations_from_other_writers_are_preserved_and_expire() {
        let first = PathBuf::from("first");
        let second = PathBuf::from("second");
        let previous = BTreeMap::from([(
            first.clone(),
            DiskMeasurement {
                available_bytes: 2,
                observed_at: 100,
            },
        )]);
        let merged = merge_disk_observations(
            previous.clone(),
            BTreeMap::from([(second.clone(), Some(100))]),
            101,
        );
        assert_eq!(
            merged.values().map(|sample| sample.available_bytes).min(),
            Some(2)
        );
        assert_eq!(
            merged[&first].observed_at, 100,
            "merging must not freshen another sensor's timestamp"
        );
        let expired =
            merge_disk_observations(previous.clone(), BTreeMap::from([(second, Some(100))]), 130);
        assert!(!expired.contains_key(&first));
        let failed =
            merge_disk_observations(previous, BTreeMap::from([(first.clone(), None)]), 101);
        assert!(
            !failed.contains_key(&first),
            "failed required probe must not reuse the preceding value"
        );
    }

    #[test]
    fn same_tick_machine_sample_reuse_keeps_actual_sensor_deadline() {
        assert!(reusable_machine_sample(100, Duration::from_secs(4), 104));
        assert!(
            reusable_machine_sample(100, Duration::ZERO, 106),
            "a just-completed slow census is reused without resetting its original observed_at"
        );
        assert!(!reusable_machine_sample(100, Duration::from_secs(5), 105));
        assert!(!reusable_machine_sample(105, Duration::from_secs(1), 104));
        assert!(
            !reusable_machine_sample(100, Duration::ZERO, 130),
            "completed census must not extend its original sensor TTL"
        );
    }

    #[test]
    fn capacity_storage_is_shared_by_projects_within_one_home() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for root in [&first, &second] {
            std::fs::create_dir_all(root.join("target")).unwrap();
            std::fs::write(root.join("target/artifact"), [0_u8; 1]).unwrap();
        }
        let mut probe = RuntimeProbe::default();
        probe.refresh(&first).unwrap();
        let path = snapshot_path();
        probe.refresh(&second).unwrap();
        assert_eq!(path, snapshot_path());
        let snapshot = read_snapshot(&path).unwrap();
        assert!(snapshot
            .targets
            .contains_key(&normalized(&first).join("target")));
        assert!(snapshot
            .targets
            .contains_key(&normalized(&second).join("target")));
        assert!(snapshot.disk_observations.contains_key(&normalized(&first)));
        assert!(snapshot
            .disk_observations
            .contains_key(&normalized(&second)));
    }

    #[cfg(unix)]
    #[test]
    fn shared_runtime_symlink_does_not_share_mutable_capacity_observations_between_homes() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let production = tempfile::tempdir().unwrap();
        let isolated = tempfile::tempdir().unwrap();
        let _production_home = gwt_core::test_support::ScopedGwtHome::set(production.path());
        let runtime = gwt_core::paths::gwt_runtime_dir();
        std::fs::create_dir_all(&runtime).unwrap();
        let production_snapshot = snapshot_path();
        let production_lock = capacity_directory().join("agent-capacity.lock");
        std::fs::create_dir_all(production_snapshot.parent().unwrap()).unwrap();
        std::fs::write(&production_snapshot, b"production snapshot sentinel").unwrap();
        std::fs::write(&production_lock, b"production lock sentinel").unwrap();
        let _isolated_home = gwt_core::test_support::ScopedGwtHome::set(isolated.path());
        std::fs::create_dir_all(gwt_core::paths::gwt_home()).unwrap();
        std::os::unix::fs::symlink(&runtime, gwt_core::paths::gwt_runtime_dir()).unwrap();
        let root = isolated.path().join("project");
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/artifact"), [0_u8; 1]).unwrap();
        refresh_machine_capacity(&root).unwrap();
        assert_eq!(
            std::fs::read(&production_snapshot).unwrap(),
            b"production snapshot sentinel",
            "isolated observer must not overwrite production's live census"
        );
        assert_eq!(
            std::fs::read(production_lock).unwrap(),
            b"production lock sentinel"
        );
        assert_ne!(
            normalized(snapshot_path().parent().unwrap()),
            normalized(production_snapshot.parent().unwrap())
        );
    }

    #[test]
    fn failed_target_remeasurement_invalidates_last_good_sample_and_valid_retry_recovers() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let root = temp.path().join("project");
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/artifact"), [0_u8; 8]).unwrap();
        let root = normalized(&root);
        let target = root.join("target");
        let mut probe = RuntimeProbe::default();
        probe.refresh(&root).unwrap();
        let mut snapshot = read_snapshot(&snapshot_path()).unwrap();
        snapshot.targets.get_mut(&target).unwrap().observed_at =
            epoch_seconds() - TARGET_TTL_SECS / 2 - 1;
        gwt_github::cache::write_atomic(&snapshot_path(), &serde_json::to_vec(&snapshot).unwrap())
            .unwrap();
        std::fs::remove_dir_all(&target).unwrap();
        probe.refresh(&root).unwrap();
        let failed = read_snapshot(&snapshot_path()).unwrap();
        assert!(
            !failed.targets.contains_key(&target),
            "I/O failure invalidates a required measured target"
        );
        let view = project_capacity(&root, &BTreeSet::new(), 0);
        assert_eq!(view.recommended_worker_limit, 0);
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("artifact"), [0_u8; 7]).unwrap();
        probe.refresh(&root).unwrap();
        assert_eq!(
            read_snapshot(&snapshot_path()).unwrap().targets[&target].bytes,
            7
        );
    }
}
