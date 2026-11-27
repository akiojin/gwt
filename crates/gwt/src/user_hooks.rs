//! Scripts a user registers to run at a hook timing (Issue #7730, SPEC #7007
//! item 210).
//!
//! A registration is a timing plus the location of a script. It is saved in
//! the machine-local project store, never in the repository: cloning or
//! pulling a repository must not be able to make gwt run a script.
//!
//! Registered hooks are part of the hook audit ([`audit`]) and are started,
//! without being waited for, when their timing fires ([`run_for_event`]).

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Stdio},
};

use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 1;

/// When a registered script is started. Each timing is a hook event gwt
/// already receives from the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserHookTiming {
    /// An agent session starts.
    SessionStart,
    /// An agent finishes responding and stops.
    Stop,
}

impl UserHookTiming {
    fn from_event(event: &str) -> Option<Self> {
        match event {
            "SessionStart" => Some(Self::SessionStart),
            "Stop" => Some(Self::Stop),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::Stop => "stop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserHook {
    pub timing: UserHookTiming,
    /// Absolute location of the script.
    pub script: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    hooks: Vec<UserHook>,
}

/// A change requested from the Hook Center.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UserHookChange {
    Register {
        timing: UserHookTiming,
        script: String,
    },
    Remove {
        timing: UserHookTiming,
        script: String,
    },
}

/// What the audit found at a registered script's location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptState {
    Ready,
    Missing,
    NotAFile,
    NotExecutable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserHookAudit {
    pub timing: UserHookTiming,
    pub script: String,
    pub state: ScriptState,
}

/// The registered hooks as the audit sees them. `error` is set when the
/// registrations themselves could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserHooksAudit {
    pub hooks: Vec<UserHookAudit>,
    pub error: Option<String>,
}

/// Where a project's registrations are saved. Worktrees of one repository
/// share it.
pub fn path(project_root: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(project_root).join("user-hooks.json")
}

fn load_from(path: &Path) -> io::Result<Vec<UserHook>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Store>(&bytes)
            .map(|store| store.hooks)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// Resolve what the user typed to an absolute location: `~/` is the home
/// directory and a relative location is inside the project.
fn resolve_script(project_root: &Path, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Enter the location of the script.".to_string());
    }
    let path = match raw.strip_prefix("~/") {
        Some(rest) => home_dir()
            .ok_or_else(|| "The home directory could not be resolved.".to_string())?
            .join(rest),
        None => PathBuf::from(raw),
    };
    Ok(if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    })
}

fn script_state(script: &Path) -> ScriptState {
    let Ok(metadata) = fs::metadata(script) else {
        return ScriptState::Missing;
    };
    if !metadata.is_file() {
        return ScriptState::NotAFile;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return ScriptState::NotExecutable;
        }
    }
    ScriptState::Ready
}

fn apply_at(store_path: &Path, project_root: &Path, change: UserHookChange) -> Result<(), String> {
    let mut hooks = load_from(store_path)
        .map_err(|error| format!("Registered hooks could not be read: {error}"))?;
    match change {
        UserHookChange::Register { timing, script } => {
            let resolved = resolve_script(project_root, &script)?;
            let script = resolved.to_string_lossy().into_owned();
            match script_state(&resolved) {
                ScriptState::Ready => {}
                ScriptState::Missing => return Err(format!("No file exists at {script}.")),
                ScriptState::NotAFile => return Err(format!("{script} is not a file.")),
                ScriptState::NotExecutable => {
                    return Err(format!(
                        "{script} is not executable. Make it executable, then register it."
                    ))
                }
            }
            let hook = UserHook { timing, script };
            if hooks.contains(&hook) {
                return Err("This script is already registered for this timing.".to_string());
            }
            hooks.push(hook);
        }
        UserHookChange::Remove { timing, script } => {
            let before = hooks.len();
            hooks.retain(|hook| !(hook.timing == timing && hook.script == script));
            if hooks.len() == before {
                return Err("This hook is no longer registered.".to_string());
            }
        }
    }
    let store = Store {
        schema_version: SCHEMA_VERSION,
        hooks,
    };
    let bytes = serde_json::to_vec_pretty(&store).map_err(|error| error.to_string())?;
    let save = || -> io::Result<()> {
        if let Some(parent) = store_path.parent() {
            fs::create_dir_all(parent)?;
        }
        gwt_core::atomic_file::write_atomic(store_path, &bytes)
    };
    save().map_err(|error| format!("Registered hooks could not be saved: {error}"))
}

/// Register or remove a hook for the project. The message of a refusal is
/// shown to the user as is.
pub fn apply(project_root: &Path, change: UserHookChange) -> Result<(), String> {
    apply_at(&path(project_root), project_root, change)
}

fn audit_at(store_path: &Path) -> UserHooksAudit {
    match load_from(store_path) {
        Ok(hooks) => UserHooksAudit {
            hooks: hooks
                .into_iter()
                .map(|hook| UserHookAudit {
                    state: script_state(Path::new(&hook.script)),
                    timing: hook.timing,
                    script: hook.script,
                })
                .collect(),
            error: None,
        },
        Err(error) => UserHooksAudit {
            hooks: Vec::new(),
            error: Some(format!("Registered hooks could not be read: {error}")),
        },
    }
}

/// Audit the project's registered hooks. Reading never changes them.
pub fn audit(project_root: &Path) -> UserHooksAudit {
    audit_at(&path(project_root))
}

/// Start the scripts registered for `event` and hand back the running ones.
fn start_at(store_path: &Path, worktree_root: &Path, event: &str) -> Vec<Child> {
    let Some(timing) = UserHookTiming::from_event(event) else {
        return Vec::new();
    };
    let mut started = Vec::new();
    for hook in audit_at(store_path).hooks {
        if hook.timing != timing || hook.state != ScriptState::Ready {
            continue;
        }
        let request = gwt_core::process::ProcessPlanRequest::new(&hook.script)
            .current_dir(worktree_root)
            .env("GWT_HOOK_TIMING", timing.as_str());
        let spawned = gwt_core::process::resolved_command(request)
            .map_err(|failure| format!("{failure:?}"))
            .and_then(|mut command| {
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| error.to_string())
            });
        match spawned {
            Ok(child) => started.push(child),
            Err(error) => tracing::warn!(
                target: "gwt::user_hooks",
                script = %hook.script,
                %error,
                "registered hook could not be started"
            ),
        }
    }
    started
}

/// Start the scripts registered for `event` in `worktree_root`. The hook
/// event never waits for them and never fails because of them.
pub fn run_for_event(worktree_root: &Path, event: &str) {
    for mut child in start_at(&path(worktree_root), worktree_root, event) {
        // Reap without making the hook event wait for the script.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn register(timing: UserHookTiming, script: &str) -> UserHookChange {
        UserHookChange::Register {
            timing,
            script: script.to_string(),
        }
    }

    #[test]
    fn a_registered_hook_is_listed_by_the_audit_and_can_be_removed() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(project.join(".gwt/hooks")).unwrap();
        script(&project.join(".gwt/hooks"), "notify.sh", "#!/bin/sh\n");
        let notify = project.join(".gwt/hooks/notify.sh");
        let store = temp.path().join("store/user-hooks.json");

        assert_eq!(
            audit_at(&store),
            UserHooksAudit {
                hooks: Vec::new(),
                error: None
            }
        );
        // A location inside the project is saved as an absolute location.
        apply_at(
            &store,
            &project,
            register(UserHookTiming::Stop, " .gwt/hooks/notify.sh "),
        )
        .unwrap();
        let listed = audit_at(&store).hooks;
        assert_eq!(
            listed,
            vec![UserHookAudit {
                timing: UserHookTiming::Stop,
                script: notify.to_string_lossy().into_owned(),
                state: ScriptState::Ready,
            }]
        );
        assert_eq!(
            apply_at(
                &store,
                &project,
                register(UserHookTiming::Stop, &listed[0].script)
            ),
            Err("This script is already registered for this timing.".to_string())
        );

        // The audit keeps reporting a hook whose script went away.
        fs::remove_file(&notify).unwrap();
        assert_eq!(audit_at(&store).hooks[0].state, ScriptState::Missing);

        let remove = UserHookChange::Remove {
            timing: UserHookTiming::Stop,
            script: listed[0].script.clone(),
        };
        apply_at(&store, &project, remove.clone()).unwrap();
        assert!(audit_at(&store).hooks.is_empty());
        assert!(apply_at(&store, &project, remove).is_err());
    }

    #[test]
    fn registration_is_refused_without_a_script_that_can_run() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("user-hooks.json");
        let refused = |script: &str| {
            apply_at(
                &store,
                temp.path(),
                register(UserHookTiming::SessionStart, script),
            )
            .unwrap_err()
        };
        assert_eq!(refused("  "), "Enter the location of the script.");
        assert!(refused("missing.sh").starts_with("No file exists at "));
        fs::create_dir(temp.path().join("dir")).unwrap();
        assert!(refused("dir").ends_with(" is not a file."));
        #[cfg(unix)]
        {
            fs::write(temp.path().join("plain.sh"), "#!/bin/sh\n").unwrap();
            assert!(refused("plain.sh").contains("is not executable"));
        }
        assert!(!store.exists(), "a refused registration saves nothing");
    }

    #[test]
    fn unreadable_registrations_are_reported_by_the_audit() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("user-hooks.json");
        fs::write(&store, "not json").unwrap();
        let audit = audit_at(&store);
        assert!(audit.hooks.is_empty());
        assert!(audit
            .error
            .unwrap()
            .starts_with("Registered hooks could not be read"));
        assert!(apply_at(
            &store,
            temp.path(),
            register(UserHookTiming::Stop, "any.sh")
        )
        .is_err());
        assert_eq!(fs::read_to_string(&store).unwrap(), "not json");
    }

    #[cfg(unix)]
    #[test]
    fn only_the_scripts_registered_for_the_fired_timing_are_started() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        fs::create_dir(&worktree).unwrap();
        let store = temp.path().join("user-hooks.json");
        let on_stop = script(
            temp.path(),
            "stop.sh",
            "#!/bin/sh\nprintf '%s' \"$GWT_HOOK_TIMING\" > ran-stop\n",
        );
        let on_start = script(temp.path(), "start.sh", "#!/bin/sh\n: > ran-start\n");
        for (timing, path) in [
            (UserHookTiming::Stop, &on_stop),
            (UserHookTiming::SessionStart, &on_start),
        ] {
            apply_at(&store, &worktree, register(timing, &path.to_string_lossy())).unwrap();
        }

        assert!(start_at(&store, &worktree, "PreToolUse").is_empty());
        let mut started = start_at(&store, &worktree, "Stop");
        assert_eq!(started.len(), 1);
        assert!(started[0].wait().unwrap().success());
        // The script ran in the worktree with its timing.
        assert_eq!(
            fs::read_to_string(worktree.join("ran-stop")).unwrap(),
            "stop"
        );
        assert!(!worktree.join("ran-start").exists());
    }
}
