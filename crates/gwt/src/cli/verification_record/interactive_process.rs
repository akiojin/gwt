//! Detect authentication programs inside one verification command's tree.
//! Reclamation stays with the caller's existing process-group/Job ownership.

use std::collections::BTreeSet;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Default)]
pub(crate) struct InteractiveProcessDetector {
    system: System,
}

impl InteractiveProcessDetector {
    pub(crate) fn detect(&mut self, root_pid: u32) -> Option<String> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .without_tasks()
                // A shell can exec a helper without changing its PID.
                .with_exe(UpdateKind::Always),
        );
        let processes = self
            .system
            .processes()
            .values()
            .map(|process| {
                // Linux's process name can truncate credential-manager to 15
                // bytes. The executable basename retains the exact identity.
                let name = process
                    .exe()
                    .and_then(std::path::Path::file_name)
                    .unwrap_or_else(|| process.name())
                    .to_string_lossy()
                    .into_owned();
                (
                    process.pid().as_u32(),
                    process.parent().map(sysinfo::Pid::as_u32),
                    name,
                )
            })
            .collect::<Vec<_>>();
        detected_in_snapshot(root_pid, &processes)
    }
}

fn detected_in_snapshot(root_pid: u32, processes: &[(u32, Option<u32>, String)]) -> Option<String> {
    if !processes.iter().any(|(pid, _, _)| *pid == root_pid) {
        return None;
    }
    let mut owned = BTreeSet::from([root_pid]);
    loop {
        let previous = owned.len();
        for (pid, parent, _) in processes {
            if parent.is_some_and(|parent| owned.contains(&parent)) {
                owned.insert(*pid);
            }
        }
        if owned.len() == previous {
            break;
        }
    }
    processes.iter().find_map(|(pid, _, name)| {
        let normalized = name.to_ascii_lowercase();
        let executable = normalized.strip_suffix(".exe").unwrap_or(&normalized);
        (owned.contains(pid)
            && matches!(
                executable,
                "git-credential-manager"
                    | "git-credential-manager-core"
                    | "git-askpass"
                    | "ssh-askpass"
            ))
        .then(|| {
            format!(
                "interactive authentication process '{name}' (pid {pid}) detected inside owned \
                 command pid {root_pid}; reclaiming only that command's workload. Fix the \
                 authentication-requesting fixture or command and rerun verify.run."
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_authentication_detection_is_scoped_to_the_owned_command() {
        let processes = vec![
            (10, None, "cargo".to_string()),
            (11, Some(10), "test-fixture".to_string()),
            (12, Some(11), "git-credential-manager.exe".to_string()),
            (20, None, "cargo".to_string()),
            (21, Some(20), "git-credential-cache".to_string()),
            (22, Some(20), "compiler".to_string()),
        ];
        let diagnostic = detected_in_snapshot(10, &processes).unwrap();
        assert!(diagnostic.contains("git-credential-manager.exe"));
        assert!(diagnostic.contains("pid 12"));
        assert!(diagnostic.contains("owned command pid 10"));
        assert!(diagnostic.contains("fixture or command"));
        assert!(diagnostic.contains("rerun verify.run"));
        assert!(detected_in_snapshot(20, &processes).is_none());
        assert!(detected_in_snapshot(99, &processes).is_none());
    }

    #[test]
    fn interactive_authentication_detection_uses_exact_executable_names() {
        for name in [
            "git-credential-manager",
            "GIT-CREDENTIAL-MANAGER.EXE",
            "git-credential-manager-core.exe",
            "git-askpass",
            "ssh-askpass.exe",
        ] {
            assert!(
                detected_in_snapshot(10, &[(10, None, name.to_string())]).is_some(),
                "{name}"
            );
        }
        for name in [
            "git",
            "git-credential-cache",
            "test-git-credential-manager",
            "cargo",
        ] {
            assert!(
                detected_in_snapshot(10, &[(10, None, name.to_string())]).is_none(),
                "{name}"
            );
        }
    }
}
