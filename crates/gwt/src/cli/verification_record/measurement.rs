//! Environment evidence for canonical performance commands (Issue #5816).
//!
//! CPU samples are process accounting, not scheduler wait estimates. Waiting
//! for admission and waiting for cap relief are recorded separately.

use std::{
    collections::BTreeSet,
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

const RELIEF_TIMEOUT: Duration = Duration::from_secs(15);
const RELIEF_POLL: Duration = Duration::from_millis(100);
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

pub use gwt_core::process_tree::job_cpu::JobCpuCap as JobCap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub pid: u32,
    /// The runner and nominated GUI must be uncapped; other descendants are
    /// CPU context and retain their ordinary resource policy.
    pub required_uncapped: bool,
    pub start_time_seconds: u64,
    pub initial_cpu_time_ms: u64,
    pub cpu_time_ms: u64,
    /// One-core CPU percent multiplied by 1000; values can exceed 100000.
    pub cpu_percent_milli: u64,
    pub first_observed_ms: u64,
    pub observed_ms: u64,
    pub sample_count: u64,
    pub job_caps: Vec<JobCap>,
    pub job_cap_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub status: String,
    pub reason: Option<String>,
    pub admission_wait_ms: u64,
    pub cap_relief_wait_ms: u64,
    pub raw_exit_code: Option<i32>,
    pub targets: Vec<Target>,
}

impl Evidence {
    pub fn incompatible(&self) -> bool {
        self.status == "environment_incompatible"
    }

    fn reject(&mut self, reason: String) {
        self.status = "environment_incompatible".into();
        self.reason.get_or_insert(reason);
    }
}

pub(super) struct Capture {
    evidence: Evidence,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<Evidence>>,
}

impl Capture {
    pub fn prepare(command: &str, canonical_headed: bool, admission_wait_ms: u64) -> Option<Self> {
        let (env, _) =
            super::take_env_assignments(super::split_command_line(command).ok()?).ok()?;
        let nominated = env
            .iter()
            .rev()
            .find(|(key, _)| key == "GWT_VERIFY_TARGET_PIDS");
        if !canonical_headed
            && nominated.is_none()
            && !env
                .iter()
                .rev()
                .find(|(key, _)| key == "GWT_VERIFY_PERFORMANCE")
                .is_some_and(|(_, value)| value == "1")
        {
            return None;
        }
        let mut evidence = Evidence {
            status: "ready".into(),
            reason: None,
            admission_wait_ms,
            cap_relief_wait_ms: 0,
            raw_exit_code: None,
            targets: Vec::new(),
        };
        let mut seeds = vec![std::process::id()];
        if let Some((_, pids)) = nominated {
            for value in pids.split(',') {
                match value.trim().parse::<u32>() {
                    Ok(pid) if pid != 0 => {
                        if !seeds.contains(&pid) {
                            seeds.push(pid);
                        }
                    }
                    _ => evidence.reject(
                        "GWT_VERIFY_TARGET_PIDS requires comma-separated nonzero PIDs".into(),
                    ),
                }
            }
        }
        let started = Instant::now();
        if !evidence.incompatible() {
            loop {
                match sample(&mut evidence, &seeds, started.elapsed(), true) {
                    Ok(None) => break,
                    Ok(Some(reason)) if started.elapsed() >= RELIEF_TIMEOUT => {
                        evidence.reject(format!("{reason}; CPU cap relief did not complete within 15s. Run the updated gwt runtime and retry the registered matrix"));
                        break;
                    }
                    Ok(Some(_)) => std::thread::sleep(RELIEF_POLL),
                    Err(reason) => {
                        evidence.reject(reason);
                        break;
                    }
                }
            }
        }
        evidence.cap_relief_wait_ms = millis(started.elapsed());
        // Start CPU accounting at the measurement boundary, after relief.
        for target in &mut evidence.targets {
            target.initial_cpu_time_ms += target.cpu_time_ms;
            target.cpu_time_ms = 0;
            target.cpu_percent_milli = 0;
            target.first_observed_ms = 0;
            target.observed_ms = 0;
        }
        let mut capture = Self {
            evidence,
            stop: None,
            worker: None,
        };
        if capture.ready() {
            let (stop, receiver) = mpsc::channel();
            let mut observed = capture.evidence.clone();
            match std::thread::Builder::new()
                .name("gwt-measurement-environment".into())
                .spawn(move || {
                    let started = Instant::now();
                    loop {
                        let stopped = !matches!(
                            receiver.recv_timeout(SAMPLE_INTERVAL),
                            Err(mpsc::RecvTimeoutError::Timeout)
                        );
                        match sample(&mut observed, &seeds, started.elapsed(), false) {
                            Ok(Some(reason)) | Err(reason) => observed.reject(reason),
                            Ok(None) => {}
                        }
                        if stopped {
                            return observed;
                        }
                    }
                }) {
                Ok(worker) => {
                    capture.stop = Some(stop);
                    capture.worker = Some(worker);
                }
                Err(error) => capture
                    .evidence
                    .reject(format!("CPU environment observer could not start: {error}")),
            }
        }
        Some(capture)
    }

    pub fn ready(&self) -> bool {
        !self.evidence.incompatible()
    }

    pub fn diagnostic(&self) -> String {
        format!(
            "ENVIRONMENT_INCOMPATIBLE: {}\n",
            self.evidence
                .reason
                .as_deref()
                .unwrap_or("measurement environment is unreadable")
        )
    }

    pub fn finish(mut self, raw_exit_code: Option<i32>) -> Evidence {
        self.stop();
        self.evidence.raw_exit_code = raw_exit_code;
        self.evidence.clone()
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            match worker.join() {
                Ok(evidence) => self.evidence = evidence,
                Err(_) => self.evidence.reject(
                    "CPU environment observer panicked; retry after repairing the environment"
                        .into(),
                ),
            }
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn sample(
    evidence: &mut Evidence,
    seeds: &[u32],
    elapsed: Duration,
    preflight: bool,
) -> Result<Option<String>, String> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu().without_tasks(),
    );
    let elapsed_ms = millis(elapsed);
    let mut capped = None;
    let mut unknown = None;
    // Put the runner first, so the initial evidence always identifies it.
    let mut pids = seeds.to_vec();
    pids.extend(
        system
            .processes()
            .keys()
            .map(|pid| pid.as_u32())
            .filter(|pid| !seeds.contains(pid) && descends_from(&system, *pid, seeds)),
    );
    for pid in pids {
        let Some(process) = system.process(sysinfo::Pid::from_u32(pid)) else {
            if preflight && seeds.contains(&pid) {
                unknown
                    .get_or_insert_with(|| format!("measurement target PID {pid} is not running"));
            }
            continue;
        };
        let start_time_seconds = process.start_time();
        if seeds.contains(&pid)
            && evidence
                .targets
                .iter()
                .any(|target| target.pid == pid && target.start_time_seconds != start_time_seconds)
        {
            unknown.get_or_insert_with(|| {
                format!(
                    "measurement target PID {pid} was recycled; nominate the new target explicitly"
                )
            });
            continue;
        }
        let index = evidence.targets.iter().position(|target| {
            target.pid == pid && target.start_time_seconds == start_time_seconds
        });
        let target = match index {
            Some(index) => &mut evidence.targets[index],
            None => {
                evidence.targets.push(Target {
                    pid,
                    required_uncapped: seeds.contains(&pid),
                    start_time_seconds,
                    initial_cpu_time_ms: process.accumulated_cpu_time(),
                    cpu_time_ms: 0,
                    cpu_percent_milli: 0,
                    first_observed_ms: elapsed_ms,
                    observed_ms: 0,
                    sample_count: 0,
                    job_caps: Vec::new(),
                    job_cap_error: None,
                });
                evidence
                    .targets
                    .last_mut()
                    .expect("inserted measurement target")
            }
        };
        target.sample_count += 1;
        target.cpu_time_ms = process
            .accumulated_cpu_time()
            .saturating_sub(target.initial_cpu_time_ms);
        target.observed_ms = elapsed_ms.saturating_sub(target.first_observed_ms);
        target.cpu_percent_milli =
            target.cpu_time_ms.saturating_mul(100_000) / target.observed_ms.max(1);
        let caps = match gwt_core::process_tree::job_cpu::observe(pid) {
            Ok(caps) => caps,
            Err(error) => {
                target.job_cap_error = Some(error.clone());
                if target.required_uncapped {
                    unknown.get_or_insert(error);
                }
                Vec::new()
            }
        };
        #[cfg(windows)]
        let (caps, unregistered_current) = {
            let mut caps = caps;
            let unknown = if pid == std::process::id() {
                match gwt_core::process_tree::job_cpu::observe_current() {
                    Ok(current) => {
                        let unknown = current.in_job && caps.is_empty();
                        if current.in_job {
                            caps.push(JobCap {
                                job_name: "native immediate Job (ancestry requires registry)"
                                    .into(),
                                owner_pid: 0,
                                hard_cap_percent: current.hard_cap_percent,
                            });
                        }
                        unknown
                    }
                    Err(error) => {
                        unknown.get_or_insert(error);
                        false
                    }
                }
            } else {
                false
            };
            (caps, unknown)
        };
        if let Some(cap) = caps
            .iter()
            .find(|cap| target.required_uncapped && cap.hard_cap_percent.is_some())
        {
            capped.get_or_insert_with(|| {
                format!(
                    "PID {pid} Job {} has CPU hard cap {}%",
                    cap.job_name,
                    cap.hard_cap_percent.unwrap()
                )
            });
        }
        // Retain the first incompatible readback even if relief happens later.
        if !target
            .job_caps
            .iter()
            .any(|cap| cap.hard_cap_percent.is_some())
            || preflight
        {
            target.job_caps = caps;
        }
        #[cfg(windows)]
        if unregistered_current {
            unknown.get_or_insert_with(|| "runner Job ancestry is unregistered; restart gwt with the updated runtime or start from an uncapped environment, then rerun the registered matrix".into());
        }
        if seeds.contains(&pid) {
            if let Err(reason) = validate_gwt_ancestors(&system, pid, &target.job_caps) {
                unknown.get_or_insert(reason);
            }
        }
    }
    if let Some(reason) = unknown {
        return Err(reason);
    }
    Ok(capped)
}

fn descends_from(system: &System, pid: u32, seeds: &[u32]) -> bool {
    let mut next = Some(sysinfo::Pid::from_u32(pid));
    let mut visited = BTreeSet::new();
    while let Some(pid) = next {
        if !visited.insert(pid) {
            break;
        }
        if seeds.contains(&pid.as_u32()) {
            return true;
        }
        next = system.process(pid).and_then(|process| process.parent());
    }
    false
}

fn validate_gwt_ancestors(system: &System, pid: u32, caps: &[JobCap]) -> Result<(), String> {
    #[cfg(windows)]
    {
        validate_gwt_ancestors_with(pid, caps, |pid| {
            let process = system.process(sysinfo::Pid::from_u32(pid))?;
            let name = process.name().to_string_lossy();
            Some((
                process.parent().map(|parent| parent.as_u32()),
                process.start_time(),
                name.eq_ignore_ascii_case("gwt.exe") || name.eq_ignore_ascii_case("gwt"),
            ))
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (system, pid, caps);
        Ok(())
    }
}

#[cfg(any(windows, test))]
fn validate_gwt_ancestors_with(
    pid: u32,
    caps: &[JobCap],
    mut lookup: impl FnMut(u32) -> Option<(Option<u32>, u64, bool)>,
) -> Result<(), String> {
    let Some((mut next, mut child_start_time, _)) = lookup(pid) else {
        return Ok(());
    };
    let mut visited = BTreeSet::new();
    while let Some(parent) = next {
        let Some((ancestor, start_time, is_gwt)) = lookup(parent) else {
            break;
        };
        // A stored parent PID can outlive its process and belong to a newer
        // process, even a descendant. Stop before registration or cycle checks.
        if start_time > child_start_time {
            break;
        }
        if !visited.insert(parent) {
            return Err("measurement process ancestry contains a cycle".into());
        }
        if is_gwt && !caps.iter().any(|cap| cap.owner_pid == parent) {
            return Err(format!("PID {pid} has unregistered ancestor gwt PID {parent}; restart that GUI with the updated runtime before measuring"));
        }
        next = ancestor;
        child_start_time = start_time;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestry_ignores_recycled_parent_before_gwt_registration() {
        let processes = [(Some(1), 100, false), (None, 200, true)];
        assert_eq!(
            validate_gwt_ancestors_with(0, &[], |pid| processes.get(pid as usize).copied()),
            Ok(())
        );
    }

    #[test]
    fn ancestry_checks_each_link_before_cycle_detection() {
        let processes = [
            (Some(1), 300, false),
            (Some(2), 200, false),
            (Some(1), 100, false),
        ];
        assert_eq!(
            validate_gwt_ancestors_with(0, &[], |pid| processes.get(pid as usize).copied()),
            Ok(())
        );
    }

    #[test]
    fn ancestry_rejects_old_unregistered_gwt_and_accepts_registered_gwt() {
        let processes = [(Some(1), 300, false), (None, 200, true)];
        let lookup = |pid| processes.get(pid as usize).copied();
        let reason = validate_gwt_ancestors_with(0, &[], lookup).unwrap_err();
        assert!(reason.contains("PID 0 has unregistered ancestor gwt PID 1"));
        let caps = [JobCap {
            job_name: "registered pane".into(),
            owner_pid: 1,
            hard_cap_percent: None,
        }];
        assert_eq!(validate_gwt_ancestors_with(0, &caps, lookup), Ok(()));
    }

    #[test]
    fn ancestry_rejects_real_cycle_with_equal_start_times() {
        let processes = [(Some(1), 100, false), (Some(0), 100, false)];
        assert_eq!(
            validate_gwt_ancestors_with(0, &[], |pid| processes.get(pid as usize).copied()),
            Err("measurement process ancestry contains a cycle".into())
        );
    }

    #[test]
    fn ordinary_commands_do_not_change_scheduling_or_start_a_monitor() {
        assert!(Capture::prepare("cargo test -p gwt-core", false, 0).is_none());
        assert!(Capture::prepare("GWT_VERIFY_PERFORMANCE=0 git --version", false, 0).is_none());
    }

    #[test]
    fn invalid_target_is_environment_incompatible_before_dispatch() {
        let capture = Capture::prepare(
            "GWT_VERIFY_PERFORMANCE=1 GWT_VERIFY_TARGET_PIDS=bad git --version",
            false,
            42,
        )
        .unwrap();
        assert!(!capture.ready());
        let evidence = capture.finish(None);
        assert!(evidence.incompatible());
        assert_eq!(evidence.raw_exit_code, None);
        assert_eq!(evidence.admission_wait_ms, 42);
    }
}
