//! Read-only process-tree observations shared by pane health callers.

use std::collections::HashSet;

use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

/// A live descendant, without command arguments, paths, or environment data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveProcess {
    pub pid: u32,
    pub name: String,
    pub elapsed_secs: u64,
}

struct ProcessEntry {
    parent_pid: Option<u32>,
    process: LiveProcess,
}

/// One operating-system process table, reusable across all panes in a call.
pub struct ProcessSnapshot {
    processes: Vec<ProcessEntry>,
}

impl ProcessSnapshot {
    /// Capture process names, parent links, and lifetimes without arguments.
    pub fn capture() -> Self {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            // Linux otherwise lists threads as child processes. Names and
            // lifetimes need no CPU sampling, arguments, paths, or environment.
            ProcessRefreshKind::nothing().without_tasks(),
        );
        Self {
            processes: system
                .processes()
                .values()
                .filter(|process| process.status() != ProcessStatus::Zombie)
                .map(|process| ProcessEntry {
                    parent_pid: process.parent().map(sysinfo::Pid::as_u32),
                    process: LiveProcess {
                        pid: process.pid().as_u32(),
                        name: process.name().to_string_lossy().into_owned(),
                        elapsed_secs: process.run_time(),
                    },
                })
                .collect(),
        }
    }

    /// Whether the snapshot contains a live process with this PID.
    pub fn contains_pid(&self, pid: u32) -> bool {
        self.processes.iter().any(|entry| entry.process.pid == pid)
    }

    /// Return live descendants in PID order, excluding the root itself.
    /// Missing roots return no descendants; use [`Self::contains_pid`] to
    /// distinguish them from a present root that has no children.
    pub fn descendants(&self, root_pid: u32) -> Vec<LiveProcess> {
        if root_pid == 0 || !self.contains_pid(root_pid) {
            return Vec::new();
        }
        let mut owned = HashSet::from([root_pid]);
        loop {
            let previous = owned.len();
            for entry in &self.processes {
                if entry
                    .parent_pid
                    .is_some_and(|parent| owned.contains(&parent))
                {
                    owned.insert(entry.process.pid);
                }
            }
            if owned.len() == previous {
                break;
            }
        }
        let mut descendants: Vec<_> = self
            .processes
            .iter()
            .filter(|entry| entry.process.pid != root_pid && owned.contains(&entry.process.pid))
            .map(|entry| entry.process.clone())
            .collect();
        descendants.sort_by_key(|process| process.pid);
        descendants
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Child, Stdio};

    use super::*;

    struct OwnedChild(Child);

    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn idle_child() -> OwnedChild {
        #[cfg(windows)]
        let mut command = {
            let mut command = crate::process::hidden_command("cmd");
            command.args(["/C", "set /p health="]);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = crate::process::hidden_command("sleep");
            command.arg("60");
            command
        };
        OwnedChild(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn owned idle child"),
        )
    }

    #[test]
    fn observes_an_owned_child_until_it_is_killed_and_reaped() {
        let mut child = idle_child();
        let pid = child.0.id();
        let root = std::process::id();
        let snapshot = ProcessSnapshot::capture();
        let observed = snapshot
            .descendants(root)
            .into_iter()
            .find(|process| process.pid == pid)
            .expect("the live child must be observed beneath its parent");
        assert!(!observed.name.is_empty());
        assert!(snapshot.contains_pid(root));
        assert!(snapshot.contains_pid(pid));
        assert!(snapshot.descendants(pid).is_empty());

        child.0.kill().expect("kill owned child");
        child.0.wait().expect("reap owned child");

        let after = ProcessSnapshot::capture();
        assert!(!after.contains_pid(pid));
        assert!(after.descendants(pid).is_empty());
        assert!(after
            .descendants(root)
            .iter()
            .all(|process| process.pid != pid));
        // Reusing an earlier snapshot never performs another OS observation.
        assert!(snapshot.contains_pid(pid));
    }

    #[test]
    fn selects_transitive_children_without_root_or_unrelated_cycles() {
        let entry = |pid, parent_pid| ProcessEntry {
            parent_pid,
            process: LiveProcess {
                pid,
                name: format!("process-{pid}"),
                elapsed_secs: u64::from(pid),
            },
        };
        let snapshot = ProcessSnapshot {
            processes: vec![
                entry(30, Some(20)),
                entry(10, None),
                entry(20, Some(10)),
                entry(40, Some(41)),
                entry(41, Some(40)),
            ],
        };
        assert_eq!(
            snapshot
                .descendants(10)
                .iter()
                .map(|process| (process.pid, process.elapsed_secs))
                .collect::<Vec<_>>(),
            vec![(20, 20), (30, 30)]
        );
        assert_eq!(snapshot.descendants(40)[0].pid, 41);
        assert!(snapshot.descendants(999).is_empty());
        assert!(snapshot.contains_pid(10));
        assert!(!snapshot.contains_pid(999));
    }
}
