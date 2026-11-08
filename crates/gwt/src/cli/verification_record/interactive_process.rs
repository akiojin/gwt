//! Detect authentication programs inside one verification command's tree.
//! Reclamation stays with the caller's existing process-group/Job ownership.

use std::collections::BTreeSet;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Default)]
pub(crate) struct InteractiveProcessDetector {
    system: Option<System>,
}

struct ProcessSample {
    pid: u32,
    parent: Option<u32>,
    process_name: String,
    executable_basename: Option<String>,
}

impl InteractiveProcessDetector {
    pub(crate) fn detect(&mut self, root_pid: u32) -> Option<String> {
        if self.system.is_none() {
            match System::new() {
                Ok(system) => self.system = Some(system),
                Err(error) => {
                    return Some(format!(
                        "interactive authentication process observation unavailable: {error}; \
                         reclaiming only owned command pid {root_pid}; repair the observation \
                         environment and rerun verify.run."
                    ));
                }
            }
        }
        let system = self.system.as_mut()?;
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .without_tasks()
                // A shell can exec a helper without changing its PID.
                .with_exe(UpdateKind::Always),
        );
        let processes = system
            .processes()
            .values()
            .map(|process| ProcessSample {
                pid: process.pid().as_u32(),
                parent: process.parent().map(sysinfo::Pid::as_u32),
                process_name: process.name().to_string_lossy().into_owned(),
                executable_basename: process
                    .exe()
                    .and_then(std::path::Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned()),
            })
            .collect::<Vec<_>>();
        detected_in_snapshot(root_pid, &processes)
    }
}

fn detected_in_snapshot(root_pid: u32, processes: &[ProcessSample]) -> Option<String> {
    if !processes.iter().any(|process| process.pid == root_pid) {
        return None;
    }
    let mut owned = BTreeSet::from([root_pid]);
    loop {
        let previous = owned.len();
        for process in processes {
            if process.parent.is_some_and(|parent| owned.contains(&parent)) {
                owned.insert(process.pid);
            }
        }
        if owned.len() == previous {
            break;
        }
    }
    processes.iter().find_map(|process| {
        let pid = process.pid;
        if !owned.contains(&pid) {
            return None;
        }
        // A script keeps its process name while exe names the interpreter;
        // a long native helper name can instead be truncated by the OS.
        let name = std::iter::once(process.process_name.as_str())
            .chain(process.executable_basename.as_deref())
            .find(|name| is_interactive_authentication_name(name))?;
        Some(format!(
            "interactive authentication process '{name}' (pid {pid}) detected inside owned \
             command pid {root_pid}; reclaiming only that command's workload. Fix the \
             authentication-requesting fixture or command and rerun verify.run."
        ))
    })
}

fn is_interactive_authentication_name(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    let name = normalized.strip_suffix(".exe").unwrap_or(&normalized);
    matches!(
        name,
        "git-credential-manager" | "git-credential-manager-core" | "git-askpass" | "ssh-askpass"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: u32, parent: Option<u32>, name: &str) -> ProcessSample {
        ProcessSample {
            pid,
            parent,
            process_name: name.to_string(),
            executable_basename: Some(name.to_string()),
        }
    }

    #[test]
    fn interactive_authentication_detection_keeps_script_and_executable_names() {
        let mut script = sample(11, Some(10), "git-askpass");
        script.executable_basename = Some("sh".to_string());
        let processes = [sample(10, None, "cargo"), script];
        assert!(detected_in_snapshot(10, &processes)
            .unwrap()
            .contains("git-askpass"));

        // Linux can truncate the process name while exe retains the helper name.
        let mut executable = sample(11, Some(10), "git-credential-");
        executable.executable_basename = Some("git-credential-manager".to_string());
        let processes = [sample(10, None, "cargo"), executable];
        assert!(detected_in_snapshot(10, &processes)
            .unwrap()
            .contains("git-credential-manager"));
    }

    #[test]
    fn interactive_authentication_detection_is_scoped_to_the_owned_command() {
        let processes = vec![
            sample(10, None, "cargo"),
            sample(11, Some(10), "test-fixture"),
            sample(12, Some(11), "git-credential-manager.exe"),
            sample(20, None, "cargo"),
            sample(21, Some(20), "git-credential-cache"),
            sample(22, Some(20), "compiler"),
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
                detected_in_snapshot(10, &[sample(10, None, name)]).is_some(),
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
                detected_in_snapshot(10, &[sample(10, None, name)]).is_none(),
                "{name}"
            );
        }
    }
}
