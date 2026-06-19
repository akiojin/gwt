//! Runtime health poller for the operator status strip (SPEC-3107).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use gwt::protocol::{RuntimeHealthProcessView, RuntimeHealthQueueView, RuntimeHealthSnapshotView};
use gwt::BackendEvent;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio::time::{interval, MissedTickBehavior};

use crate::embedded_server::{ClientHub, ClientHubHealthStats};
use crate::OutboundEvent;
use crate::PtyWriterRegistry;

const TICK_SECS: u64 = 5;
const WARMING_SAMPLES: u64 = 2;
const WARN_CPU_PERCENT: f32 = 50.0;
const HOT_CPU_PERCENT: f32 = 100.0;
const WARN_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;
const HOT_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Spawn the runtime health poller onto the shared tokio runtime.
pub fn spawn_runtime_health_poller(
    runtime: &tokio::runtime::Runtime,
    clients: ClientHub,
    pty_writers: PtyWriterRegistry,
) {
    drop(runtime.handle().spawn(run(clients, pty_writers)));
}

async fn run(clients: ClientHub, pty_writers: PtyWriterRegistry) {
    let mut poller = Poller::new(pty_writers);
    let mut ticker = interval(Duration::from_secs(TICK_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        if !clients.has_clients() {
            continue;
        }
        let snapshot = poller.poll_once(Utc::now(), &clients);
        clients.dispatch(vec![OutboundEvent::broadcast(
            BackendEvent::RuntimeHealth { snapshot },
        )]);
    }
}

struct Poller {
    system: System,
    root_pid: u32,
    sample_count: u64,
    last_dropped_lossy: u64,
    severity: SeverityTracker,
    pty_writers: PtyWriterRegistry,
}

impl Poller {
    fn new(pty_writers: PtyWriterRegistry) -> Self {
        Self {
            system: System::new(),
            root_pid: std::process::id(),
            sample_count: 0,
            last_dropped_lossy: 0,
            severity: SeverityTracker::default(),
            pty_writers,
        }
    }

    fn poll_once(
        &mut self,
        generated_at: DateTime<Utc>,
        clients: &ClientHub,
    ) -> RuntimeHealthSnapshotView {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );
        let observed = observe_processes(&self.system);
        let parent_by_pid = parent_by_pid(&observed);
        let direct_focus_windows = focus_window_ids_by_pty_pid(&self.pty_writers);
        let mut selected = select_runtime_processes(self.root_pid, &observed);
        let cpu_percent = selected
            .iter()
            .map(|process| process.cpu_percent)
            .reduce(|acc, value| acc + value);
        let memory_bytes = selected
            .iter()
            .map(|process| process.memory_bytes)
            .sum::<u64>();
        let queue = self.queue_snapshot(clients.health_stats());
        let state = self.next_state(cpu_percent, memory_bytes, queue.dropped_lossy_delta);
        let runner_count = selected
            .iter()
            .filter(|process| process.role(self.root_pid) == "runner")
            .count();
        selected.sort_by(|left, right| {
            right
                .cpu_percent
                .total_cmp(&left.cpu_percent)
                .then_with(|| right.memory_bytes.cmp(&left.memory_bytes))
                .then_with(|| left.pid.cmp(&right.pid))
        });

        RuntimeHealthSnapshotView {
            generated_at: generated_at.to_rfc3339(),
            state,
            cpu_percent,
            memory_bytes,
            process_count: selected.len(),
            runner_count,
            queue,
            processes: detail_process_views(
                self.root_pid,
                selected,
                &parent_by_pid,
                &direct_focus_windows,
            ),
        }
    }

    fn queue_snapshot(&mut self, stats: ClientHubHealthStats) -> RuntimeHealthQueueView {
        let dropped_lossy_delta = stats.dropped_lossy.saturating_sub(self.last_dropped_lossy);
        self.last_dropped_lossy = stats.dropped_lossy;
        RuntimeHealthQueueView {
            client_count: stats.client_count,
            queued_entries: stats.queued_entries,
            dirty_panes: stats.dirty_panes,
            dropped_lossy: stats.dropped_lossy,
            dropped_lossy_delta,
            dead_clients: stats.dead_clients,
        }
    }

    fn next_state(
        &mut self,
        cpu_percent: Option<f32>,
        memory_bytes: u64,
        dropped_lossy_delta: u64,
    ) -> String {
        self.sample_count += 1;
        let classified = self.severity.classify(SeverityInput {
            cpu_percent,
            memory_bytes,
            dropped_lossy_delta,
        });
        if self.sample_count <= WARMING_SAMPLES {
            "warming".to_string()
        } else {
            classified.as_wire().to_string()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ObservedProcess {
    pid: u32,
    parent_pid: Option<u32>,
    name: String,
    command_line: String,
    executable_path: String,
    current_dir: Option<String>,
    cpu_percent: f32,
    memory_bytes: u64,
}

impl ObservedProcess {
    #[cfg(test)]
    fn new(
        pid: u32,
        parent_pid: Option<u32>,
        name: &str,
        cpu_percent: f32,
        memory_bytes: u64,
    ) -> Self {
        Self {
            pid,
            parent_pid,
            name: name.to_string(),
            command_line: String::new(),
            executable_path: String::new(),
            current_dir: None,
            cpu_percent,
            memory_bytes,
        }
    }

    #[cfg(test)]
    fn with_command_line(mut self, command_line: &str) -> Self {
        self.command_line = command_line.to_string();
        self
    }

    #[cfg(test)]
    fn with_executable_path(mut self, executable_path: &str) -> Self {
        self.executable_path = executable_path.to_string();
        self
    }

    fn role(&self, root_pid: u32) -> &'static str {
        if self.pid == root_pid || self.looks_like_gwt() {
            return "gwt";
        }
        if self.looks_like_gwtd() {
            "gwtd"
        } else if self.looks_like_runner() {
            "runner"
        } else if self.looks_like_docker() {
            "docker"
        } else if self.looks_like_codex() {
            "codex"
        } else if self.looks_like_claude() {
            "claude"
        } else {
            "child"
        }
    }

    fn is_runtime_seed(&self, root_pid: u32) -> bool {
        self.pid == root_pid
            || self.looks_like_gwt()
            || self.looks_like_gwtd()
            || self.looks_like_runner()
    }

    fn looks_like_gwt(&self) -> bool {
        self.matches_basename("gwt")
    }

    fn looks_like_gwtd(&self) -> bool {
        self.matches_basename("gwtd")
    }

    fn looks_like_runner(&self) -> bool {
        self.matches_basename("chroma_index_runner")
            || self.command_fingerprint_contains("chroma_index_runner")
    }

    fn looks_like_docker(&self) -> bool {
        self.matches_basename("docker")
            || self.matches_basename("docker-compose")
            || self.matches_basename("com.docker.cli")
            || self.command_fingerprint_contains("docker compose exec")
    }

    fn looks_like_codex(&self) -> bool {
        self.matches_basename("codex")
            || self.basename_starts_with("codex-")
            || self.command_fingerprint_contains("@openai/codex")
            || self.command_fingerprint_contains("openai/codex")
    }

    fn looks_like_claude(&self) -> bool {
        self.matches_basename("claude")
            || self.matches_basename("claude-code")
            || self.command_fingerprint_contains("@anthropic-ai/claude-code")
            || self.command_fingerprint_contains("claude-code")
    }

    fn matches_basename(&self, expected: &str) -> bool {
        let expected_exe = format!("{expected}.exe");
        self.field_basenames()
            .iter()
            .any(|basename| basename == expected || basename == &expected_exe)
    }

    fn basename_starts_with(&self, prefix: &str) -> bool {
        self.field_basenames()
            .iter()
            .any(|basename| basename.starts_with(prefix))
    }

    fn field_basenames(&self) -> [String; 2] {
        [
            lowercase_basename(&self.name),
            lowercase_basename(&self.executable_path),
        ]
    }

    fn command_fingerprint_contains(&self, needle: &str) -> bool {
        self.command_fingerprint().contains(needle)
    }

    fn command_fingerprint(&self) -> String {
        let current_dir = self.current_dir.as_deref().unwrap_or_default();
        format!(
            "{} {} {} {}",
            self.name, self.command_line, self.executable_path, current_dir
        )
        .to_ascii_lowercase()
    }

    fn into_view(self, root_pid: u32, focus_window_id: Option<String>) -> RuntimeHealthProcessView {
        RuntimeHealthProcessView {
            pid: self.pid,
            parent_pid: self.parent_pid,
            role: self.role(root_pid).to_string(),
            name: self.name,
            cpu_percent: Some(self.cpu_percent),
            memory_bytes: self.memory_bytes,
            focus_window_id,
        }
    }
}

fn observe_processes(system: &System) -> Vec<ObservedProcess> {
    system
        .processes()
        .values()
        .map(|process| ObservedProcess {
            pid: process.pid().as_u32(),
            parent_pid: process.parent().map(sysinfo::Pid::as_u32),
            name: process.name().to_string_lossy().into_owned(),
            command_line: process
                .cmd()
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" "),
            executable_path: process
                .exe()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            current_dir: process
                .cwd()
                .map(|path| path.to_string_lossy().into_owned()),
            cpu_percent: process.cpu_usage(),
            memory_bytes: process.memory(),
        })
        .collect()
}

fn select_runtime_processes(root_pid: u32, processes: &[ObservedProcess]) -> Vec<ObservedProcess> {
    let parent_by_pid = parent_by_pid(processes);
    let seed_pids: HashSet<u32> = processes
        .iter()
        .filter(|process| process.is_runtime_seed(root_pid))
        .map(|process| process.pid)
        .collect();
    processes
        .iter()
        .filter(|process| {
            seed_pids.contains(&process.pid)
                || has_any_seed_ancestor(process.pid, &seed_pids, &parent_by_pid)
        })
        .cloned()
        .collect()
}

fn parent_by_pid(processes: &[ObservedProcess]) -> HashMap<u32, Option<u32>> {
    processes
        .iter()
        .map(|process| (process.pid, process.parent_pid))
        .collect()
}

fn has_any_seed_ancestor(
    pid: u32,
    seed_pids: &HashSet<u32>,
    parent_by_pid: &HashMap<u32, Option<u32>>,
) -> bool {
    let mut seen = HashSet::new();
    let mut current = pid;
    while let Some(Some(parent)) = parent_by_pid.get(&current).copied() {
        if seed_pids.contains(&parent) {
            return true;
        }
        if !seen.insert(parent) {
            return false;
        }
        current = parent;
    }
    false
}

fn focus_window_ids_by_pty_pid(pty_writers: &PtyWriterRegistry) -> HashMap<u32, String> {
    let Ok(guard) = pty_writers.read() else {
        tracing::warn!(
            target: "gwt_runtime_health",
            "failed to read PTY writer registry for runtime health focus targets"
        );
        return HashMap::new();
    };
    guard
        .iter()
        .filter_map(|(window_id, pty)| pty.process_id().map(|pid| (pid, window_id.clone())))
        .collect()
}

fn focus_window_id_for_process(
    pid: u32,
    parent_by_pid: &HashMap<u32, Option<u32>>,
    direct_focus_windows: &HashMap<u32, String>,
) -> Option<String> {
    let mut seen = HashSet::new();
    let mut current = pid;
    loop {
        if let Some(window_id) = direct_focus_windows.get(&current) {
            return Some(window_id.clone());
        }
        if !seen.insert(current) {
            return None;
        }
        current = parent_by_pid.get(&current).copied().flatten()?;
    }
}

fn detail_process_views(
    root_pid: u32,
    processes: Vec<ObservedProcess>,
    parent_by_pid: &HashMap<u32, Option<u32>>,
    direct_focus_windows: &HashMap<u32, String>,
) -> Vec<RuntimeHealthProcessView> {
    processes
        .into_iter()
        .map(|process| {
            let focus_window_id =
                focus_window_id_for_process(process.pid, parent_by_pid, direct_focus_windows);
            process.into_view(root_pid, focus_window_id)
        })
        .collect()
}

fn lowercase_basename(value: &str) -> String {
    value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeHealthState {
    Ok,
    Warn,
    Hot,
}

impl RuntimeHealthState {
    fn as_wire(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Hot => "hot",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SeverityInput {
    cpu_percent: Option<f32>,
    memory_bytes: u64,
    dropped_lossy_delta: u64,
}

#[derive(Debug, Default)]
struct SeverityTracker {
    warn_samples: u8,
    hot_samples: u8,
}

impl SeverityTracker {
    fn classify(&mut self, input: SeverityInput) -> RuntimeHealthState {
        match target_state(input) {
            RuntimeHealthState::Hot => {
                self.hot_samples = self.hot_samples.saturating_add(1);
                self.warn_samples = self.warn_samples.saturating_add(1);
            }
            RuntimeHealthState::Warn => {
                self.hot_samples = 0;
                self.warn_samples = self.warn_samples.saturating_add(1);
            }
            RuntimeHealthState::Ok => {
                self.hot_samples = 0;
                self.warn_samples = 0;
            }
        }

        if self.hot_samples >= 3 {
            RuntimeHealthState::Hot
        } else if self.warn_samples >= 3 {
            RuntimeHealthState::Warn
        } else {
            RuntimeHealthState::Ok
        }
    }
}

fn target_state(input: SeverityInput) -> RuntimeHealthState {
    if input.cpu_percent.is_some_and(|cpu| cpu >= HOT_CPU_PERCENT)
        || input.memory_bytes >= HOT_MEMORY_BYTES
    {
        return RuntimeHealthState::Hot;
    }
    if input.cpu_percent.is_some_and(|cpu| cpu >= WARN_CPU_PERCENT)
        || input.memory_bytes >= WARN_MEMORY_BYTES
        || input.dropped_lossy_delta > 0
    {
        return RuntimeHealthState::Warn;
    }
    RuntimeHealthState::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_current_process_external_gwtd_and_descendants() {
        let processes = vec![
            ObservedProcess::new(10, None, "gwt", 4.0, 100),
            ObservedProcess::new(11, Some(10), "agent", 5.0, 200),
            ObservedProcess::new(12, Some(11), "helper", 6.0, 300),
            ObservedProcess::new(20, None, "unrelated-gwt", 90.0, 400),
            ObservedProcess::new(21, Some(20), "helper", 91.0, 500),
            ObservedProcess::new(30, None, "gwtd", 7.0, 600),
            ObservedProcess::new(31, Some(30), "gwtd-worker", 8.0, 700),
        ];

        let selected = select_runtime_processes(10, &processes);
        let pids: Vec<u32> = selected.iter().map(|process| process.pid).collect();

        assert_eq!(pids, vec![10, 11, 12, 30, 31]);
        assert_eq!(
            selected
                .iter()
                .map(|process| process.cpu_percent)
                .sum::<f32>(),
            30.0
        );
    }

    #[test]
    fn selects_other_gwt_instances_and_their_agent_descendants() {
        let processes = vec![
            ObservedProcess::new(10, None, "gwt", 4.0, 100),
            ObservedProcess::new(11, Some(10), "helper", 5.0, 200),
            ObservedProcess::new(20, None, "gwt", 8.0, 300)
                .with_executable_path("/Applications/GWT.app/Contents/MacOS/gwt"),
            ObservedProcess::new(21, Some(20), "node", 9.0, 400).with_command_line(
                "/usr/local/bin/node /opt/homebrew/lib/node_modules/@openai/codex/bin/codex.js",
            ),
            ObservedProcess::new(22, Some(21), "codex-aarch64-apple-darwin", 10.0, 500)
                .with_executable_path(
                    "/opt/homebrew/lib/node_modules/@openai/codex/bin/codex-aarch64-apple-darwin",
                ),
            ObservedProcess::new(30, None, "node", 70.0, 600).with_command_line(
                "/usr/local/bin/node /opt/homebrew/lib/node_modules/@openai/codex/bin/codex.js",
            ),
            ObservedProcess::new(31, Some(30), "codex-aarch64-apple-darwin", 80.0, 700)
                .with_executable_path(
                    "/opt/homebrew/lib/node_modules/@openai/codex/bin/codex-aarch64-apple-darwin",
                ),
            ObservedProcess::new(40, None, "claude", 90.0, 800)
                .with_command_line("/opt/homebrew/bin/claude"),
            ObservedProcess::new(50, None, "gwtd", 6.0, 900),
            ObservedProcess::new(60, None, "python3", 3.0, 70)
                .with_command_line("/tmp/chroma_index_runner"),
        ];

        let selected = select_runtime_processes(10, &processes);
        let pids: Vec<u32> = selected.iter().map(|process| process.pid).collect();
        let roles: Vec<(u32, &'static str)> = selected
            .iter()
            .map(|process| (process.pid, process.role(10)))
            .collect();

        assert_eq!(pids, vec![10, 11, 20, 21, 22, 50, 60]);
        assert_eq!(
            roles,
            vec![
                (10, "gwt"),
                (11, "child"),
                (20, "gwt"),
                (21, "codex"),
                (22, "codex"),
                (50, "gwtd"),
                (60, "runner"),
            ]
        );
    }

    #[test]
    fn excludes_independent_agents_but_keeps_selected_descendant_roles() {
        let processes = vec![
            ObservedProcess::new(10, None, "gwt", 4.0, 100),
            ObservedProcess::new(11, Some(10), "codex", 6.0, 120)
                .with_command_line("/opt/homebrew/bin/codex"),
            ObservedProcess::new(12, Some(11), "node", 8.0, 80)
                .with_command_line("/usr/local/bin/node /tmp/managed-mcp/index.js"),
            ObservedProcess::new(30, None, "node", 70.0, 400).with_command_line(
                "/usr/local/bin/node /opt/homebrew/lib/node_modules/@openai/codex/bin/codex.js",
            ),
            ObservedProcess::new(31, Some(30), "codex-x86_64-apple-darwin", 10.0, 100)
                .with_executable_path(
                    "/opt/homebrew/lib/node_modules/@openai/codex/bin/codex-x86_64-apple-darwin",
                ),
            ObservedProcess::new(32, Some(31), "node", 15.0, 100)
                .with_command_line("/usr/local/bin/node /tmp/playwright-mcp/index.js"),
            ObservedProcess::new(40, None, "node", 80.0, 1000)
                .with_command_line("/usr/local/bin/node /tmp/random-server.js"),
            ObservedProcess::new(50, None, "claude", 20.0, 200)
                .with_command_line("/opt/homebrew/bin/claude"),
            ObservedProcess::new(51, Some(50), "node", 5.0, 50)
                .with_command_line("/usr/local/bin/node /tmp/context7-mcp/index.js"),
            ObservedProcess::new(60, None, "gwtd", 7.0, 600),
            ObservedProcess::new(61, Some(60), "claude", 9.0, 90)
                .with_command_line("/opt/homebrew/bin/claude"),
            ObservedProcess::new(70, None, "python3", 3.0, 70)
                .with_command_line("/tmp/chroma_index_runner"),
        ];

        let selected = select_runtime_processes(10, &processes);
        let pids: Vec<u32> = selected.iter().map(|process| process.pid).collect();
        let roles: Vec<(u32, &'static str)> = selected
            .iter()
            .map(|process| (process.pid, process.role(10)))
            .collect();

        assert_eq!(pids, vec![10, 11, 12, 60, 61, 70]);
        assert_eq!(
            roles,
            vec![
                (10, "gwt"),
                (11, "codex"),
                (12, "child"),
                (60, "gwtd"),
                (61, "claude"),
                (70, "runner"),
            ]
        );
    }

    #[test]
    fn assigns_focus_window_id_to_pty_process_and_descendants() {
        let processes = vec![
            ObservedProcess::new(10, None, "gwt", 1.0, 100),
            ObservedProcess::new(20, Some(10), "zsh", 2.0, 120),
            ObservedProcess::new(21, Some(20), "node", 40.0, 300).with_command_line(
                "/usr/local/bin/node /opt/homebrew/lib/node_modules/@openai/codex/bin/codex.js",
            ),
            ObservedProcess::new(22, Some(21), "codex-aarch64-apple-darwin", 51.0, 400)
                .with_executable_path(
                    "/opt/homebrew/lib/node_modules/@openai/codex/bin/codex-aarch64-apple-darwin",
                ),
            ObservedProcess::new(30, None, "gwtd", 3.0, 140),
            ObservedProcess::new(31, Some(30), "worker", 4.0, 160),
        ];
        let parent_by_pid = parent_by_pid(&processes);
        let direct_focus_windows = HashMap::from([(20, "agent-window-1".to_string())]);
        let selected = select_runtime_processes(10, &processes);
        let focus_by_pid: HashMap<u32, Option<String>> = selected
            .iter()
            .map(|process| {
                (
                    process.pid,
                    focus_window_id_for_process(process.pid, &parent_by_pid, &direct_focus_windows),
                )
            })
            .collect();

        assert_eq!(
            focus_by_pid.get(&20).and_then(Option::as_deref),
            Some("agent-window-1")
        );
        assert_eq!(
            focus_by_pid.get(&21).and_then(Option::as_deref),
            Some("agent-window-1")
        );
        assert_eq!(
            focus_by_pid.get(&22).and_then(Option::as_deref),
            Some("agent-window-1")
        );
        assert_eq!(focus_by_pid.get(&10).and_then(Option::as_deref), None);
        assert_eq!(focus_by_pid.get(&31).and_then(Option::as_deref), None);

        let view = processes[2]
            .clone()
            .into_view(10, Some("agent-window-1".to_string()));
        assert_eq!(view.focus_window_id.as_deref(), Some("agent-window-1"));
    }

    #[test]
    fn detail_includes_all_selected_processes_and_focus_rows() {
        let mut processes = vec![ObservedProcess::new(10, None, "gwt", 1.0, 100)];
        processes.extend((0..24).map(|index| {
            ObservedProcess::new(
                200 + index as u32,
                Some(10),
                "worker",
                100.0 - index as f32,
                900 + index as u64,
            )
        }));
        processes.push(ObservedProcess::new(20, Some(10), "zsh", 0.1, 120));
        processes.push(
            ObservedProcess::new(21, Some(20), "docker", 0.2, 180)
                .with_command_line("docker compose exec gwt codex --no-alt-screen"),
        );

        let parent_by_pid = parent_by_pid(&processes);
        let direct_focus_windows = HashMap::from([(20, "docker-agent-window".to_string())]);
        let mut selected = select_runtime_processes(10, &processes);
        selected.sort_by(|left, right| {
            right
                .cpu_percent
                .total_cmp(&left.cpu_percent)
                .then_with(|| right.memory_bytes.cmp(&left.memory_bytes))
                .then_with(|| left.pid.cmp(&right.pid))
        });
        let selected_len = selected.len();

        let detail = detail_process_views(10, selected, &parent_by_pid, &direct_focus_windows);

        assert_eq!(detail.len(), selected_len);
        let docker = detail
            .iter()
            .find(|process| process.name == "docker")
            .expect("expected low-CPU docker row to remain visible");
        assert_eq!(docker.role, "docker");
        assert_eq!(
            docker.focus_window_id.as_deref(),
            Some("docker-agent-window")
        );
    }

    #[test]
    fn sustained_severity_requires_three_consecutive_hot_samples() {
        let mut tracker = SeverityTracker::default();
        let hot_sample = SeverityInput {
            cpu_percent: Some(120.0),
            memory_bytes: 3 * 1024 * 1024 * 1024,
            dropped_lossy_delta: 0,
        };

        assert_eq!(tracker.classify(hot_sample), RuntimeHealthState::Ok);
        assert_eq!(tracker.classify(hot_sample), RuntimeHealthState::Ok);
        assert_eq!(tracker.classify(hot_sample), RuntimeHealthState::Hot);

        let cool_sample = SeverityInput {
            cpu_percent: Some(1.0),
            memory_bytes: 128 * 1024 * 1024,
            dropped_lossy_delta: 0,
        };
        assert_eq!(tracker.classify(cool_sample), RuntimeHealthState::Ok);
    }

    #[test]
    fn dropped_lossy_delta_escalates_after_sustained_samples() {
        let mut tracker = SeverityTracker::default();
        let queue_sample = SeverityInput {
            cpu_percent: Some(2.0),
            memory_bytes: 128 * 1024 * 1024,
            dropped_lossy_delta: 1,
        };

        assert_eq!(tracker.classify(queue_sample), RuntimeHealthState::Ok);
        assert_eq!(tracker.classify(queue_sample), RuntimeHealthState::Ok);
        assert_eq!(tracker.classify(queue_sample), RuntimeHealthState::Warn);
    }
}
