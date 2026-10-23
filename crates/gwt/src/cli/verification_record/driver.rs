//! Windows holds a running image open. Move that image out of Cargo's target
//! directory rather than spawning a child while its parent still locks it.

use std::{
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const WINDOWS_ARTIFACT_LOCK_RECOVERY: &str = "\nWindows artifact lock: a running process may be holding Cargo's gwtd.exe output open. Identify the holder and stop it only if it is your own process. Run `cargo build -p gwt --bin gwtd` without a manual verification lease, then invoke verify.run from a SHA256-verified copy of that checkout build outside all Cargo target directories.\n";

pub(super) fn relink_failure_hint(exit_code: i32, output: &str) -> &'static str {
    if cfg!(windows)
        && exit_code != 0
        && ["(os error 5)", "(os error 32)", "(os error 33)"]
            .iter()
            .any(|code| output.contains(code))
        && output.lines().any(|line| {
            line.split_once("failed to remove file ")
                .map(|(_, path)| path.trim().trim_matches(['`', '\'', '"']))
                .and_then(|path| path.rsplit(['\\', '/']).next())
                .is_some_and(|file| file.eq_ignore_ascii_case("gwtd.exe"))
        })
    {
        WINDOWS_ARTIFACT_LOCK_RECOVERY
    } else {
        ""
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverProvenance {
    pub original_path: PathBuf,
    pub fixed_path: PathBuf,
    pub sha256: String,
    /// Embedded by build.rs, not inferred from the current working tree.
    pub source_head: String,
}

pub(super) fn prepare(
    worktree: &Path,
    commands: &[String],
) -> io::Result<Option<DriverProvenance>> {
    // In-process unit fixtures have no production gwtd image to relocate.
    #[cfg(any(not(windows), test))]
    {
        let _ = (worktree, commands);
        Ok(None)
    }
    #[cfg(all(windows, not(test)))]
    {
        let original = dunce::canonicalize(std::env::current_exe()?)?;
        if original.file_name().and_then(|name| name.to_str()) != Some("gwtd.exe") {
            return Ok(None);
        }
        let source_head = env!("GWT_BUILD_COMMIT");
        validate_for_head(
            worktree,
            super::current_head_sha(worktree).ok().as_deref(),
            source_head,
        )?;
        let root = dunce::canonicalize(worktree)?;
        let Some(target) = cargo_artifact_target(&root, &original, commands) else {
            // Installed drivers and already fixed copies do not lock this
            // checkout's Cargo artifact. Keep them in place.
            return Ok(Some(DriverProvenance {
                sha256: hash_file(&original)?,
                fixed_path: original.clone(),
                original_path: original,
                source_head: source_head.to_string(),
            }));
        };
        pin_artifact(
            &original,
            &fixed_driver_root(&root, &target, commands)?,
            source_head,
        )
        .map(Some)
    }
}

#[cfg(any(windows, test))]
fn cargo_artifact_target(worktree: &Path, original: &Path, commands: &[String]) -> Option<PathBuf> {
    if original.starts_with(worktree.join("target")) {
        return Some(worktree.join("target"));
    }
    // Resolve the ambient target for config/env, then each command's overrides.
    // Use the same Cargo metadata contract as build admission and GC locks.
    std::iter::once("cargo build")
        .chain(commands.iter().map(String::as_str))
        .filter_map(|command| {
            crate::cli::verification_lease::effective_cargo_target(worktree, command, false)
                .ok()
                .flatten()
        })
        .map(|target| dunce::canonicalize(&target).unwrap_or(target))
        .find(|target| original.starts_with(target))
}

#[cfg(any(windows, test))]
fn fixed_driver_root(worktree: &Path, target: &Path, commands: &[String]) -> io::Result<PathBuf> {
    // Rename and hard-link restoration must stay on the artifact's volume.
    // Ascend if this cache would itself be inside any command's Cargo target.
    let mut parent = target;
    loop {
        parent = parent.parent().ok_or_else(|| {
            io::Error::other("Cargo target has no parent outside its verification artifacts")
        })?;
        let fixed = parent.join(".gwt/skill-state/verification-drivers");
        if cargo_artifact_target(worktree, &fixed, commands).is_none() {
            return Ok(fixed);
        }
    }
}

pub(super) fn validate_for_head(
    worktree: &Path,
    head: Option<&str>,
    source_head: &str,
) -> io::Result<()> {
    #[cfg(any(windows, test))]
    {
        if super::is_gwt_checkout(worktree) {
            return validate_source_head(source_head, head.unwrap_or_default());
        }
    }
    let _ = (worktree, head, source_head);
    Ok(())
}

#[cfg(any(windows, test))]
fn validate_source_head(built: &str, expected: &str) -> io::Result<()> {
    if built.is_empty() || built != expected {
        return Err(io::Error::other(format!(
            "verification driver source HEAD {built:?} does not match checkout HEAD {expected:?}; rebuild with `cargo build -p gwt --bin gwtd` before verify.run"
        )));
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn hash_file(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = artifact_io("open for SHA256", &[path], || std::fs::File::open(path))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = artifact_io("read for SHA256", &[path], || file.read(&mut buffer))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(any(windows, test))]
fn artifact_io<T>(
    operation: &str,
    paths: &[&Path],
    mut action: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        match action() {
            Ok(value) => return Ok(value),
            Err(error) => {
                // AV/indexer handles may briefly deny delete or access. Retry
                // only this operation: restarting pinning after rename could
                // move a different artifact published by a concurrent build.
                if cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)) && attempts <= 10
                {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                let paths = paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ");
                let recovery = if cfg!(windows)
                    && operation == "rename"
                    && matches!(error.raw_os_error(), Some(5 | 32 | 33))
                {
                    WINDOWS_ARTIFACT_LOCK_RECOVERY
                } else {
                    ""
                };
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "verification driver {operation} {paths} failed after {attempts} attempt(s): {error}{recovery}"
                    ),
                ));
            }
        }
    }
}

#[cfg(any(windows, test))]
fn pin_artifact(
    original: &Path,
    fixed_root: &Path,
    source_head: &str,
) -> io::Result<DriverProvenance> {
    use std::fs;

    let sha256 = hash_file(original)?;
    let directory = fixed_root.join(uuid::Uuid::new_v4().simple().to_string());
    artifact_io("create_dir_all", &[&directory], || {
        fs::create_dir_all(&directory)
    })?;
    let copy = directory.join("restore.exe");
    artifact_io("copy", &[original, &copy], || fs::copy(original, &copy))?;
    if hash_file(&copy)? != sha256 {
        return Err(io::Error::other(format!(
            "verification driver copy {} failed SHA256 validation",
            copy.display()
        )));
    }
    let fixed_path = directory.join("gwtd.exe");
    artifact_io("rename", &[original, &fixed_path], || {
        fs::rename(original, &fixed_path)
    })?;
    // A create-new link restores an independent, verified file object. Never
    // overwrite an artifact a concurrent Cargo build published after rename.
    match artifact_io("hard_link", &[&copy, original], || {
        fs::hard_link(&copy, original)
    }) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    artifact_io("remove_file", &[&copy], || fs::remove_file(&copy))?;
    if hash_file(&fixed_path)? != sha256 {
        return Err(io::Error::other(format!(
            "verification driver image {} failed SHA256 validation",
            fixed_path.display()
        )));
    }
    Ok(DriverProvenance {
        original_path: original.to_path_buf(),
        fixed_path,
        sha256,
        source_head: source_head.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_and_command_output_drivers_require_pinning() {
        let _lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _target = gwt_core::test_support::ScopedEnvVar::unset("CARGO_TARGET_DIR");
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='driver-fixture'\nversion='0.1.0'\n[lib]\npath='lib.rs'\n",
        )
        .unwrap();
        std::fs::write(root.join("lib.rs"), "").unwrap();
        std::fs::create_dir(root.join(".cargo")).unwrap();
        std::fs::write(
            root.join(".cargo/config.toml"),
            "[build]\ntarget-dir='build-output'\n",
        )
        .unwrap();
        let configured = root.join("build-output/debug/gwtd.exe");
        assert_eq!(
            cargo_artifact_target(&root, &configured, &[]),
            Some(root.join("build-output"))
        );
        let command_output = root.join("command-output/debug/gwtd.exe");
        let commands = vec!["cargo test --target-dir command-output".to_string()];
        assert_eq!(
            cargo_artifact_target(&root, &command_output, &commands),
            Some(root.join("command-output"))
        );
        let env_output = root.join("env-output/debug/gwtd.exe");
        let commands = vec!["CARGO_TARGET_DIR=env-output cargo test".to_string()];
        assert_eq!(
            cargo_artifact_target(&root, &env_output, &commands),
            Some(root.join("env-output"))
        );
        assert!(cargo_artifact_target(&root, &root.join("tools/gwtd.exe"), &commands).is_none());
        let target = root.join(".gwt");
        let commands = vec!["cargo test --target-dir .gwt".to_string()];
        let fixed = fixed_driver_root(&root, &target, &commands).unwrap();
        assert!(!fixed.starts_with(&target));
    }

    #[test]
    fn pin_preserves_bytes_and_uses_an_independent_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("gwtd.exe");
        std::fs::write(&artifact, b"driver").unwrap();
        let driver = pin_artifact(&artifact, &dir.path().join("fixed"), "head").unwrap();
        assert_eq!(hash_file(&artifact).unwrap(), driver.sha256);
        std::fs::write(&artifact, b"relinked").unwrap();
        assert_eq!(std::fs::read(&driver.fixed_path).unwrap(), b"driver");
        assert_eq!(hash_file(&driver.fixed_path).unwrap(), driver.sha256);
    }

    #[test]
    fn pin_missing_artifact_identifies_operation_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("missing.exe");
        let error = pin_artifact(&artifact, &dir.path().join("fixed"), "head").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        let message = error.to_string();
        assert!(message.contains("open"), "{message}");
        assert!(
            message.contains(&artifact.display().to_string()),
            "{message}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn pin_locked_artifact_identifies_rename_and_both_paths() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("gwtd.exe");
        std::fs::write(&artifact, b"driver").unwrap();
        // Model an AV/indexer handle that allows reads/writes but denies delete.
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&artifact)
            .unwrap();
        let fixed_root = dir.path().join("fixed");
        let error = pin_artifact(&artifact, &fixed_root, "head").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("os error 32"), "{message}");
        assert!(message.contains("11 attempt(s)"), "{message}");
        assert!(message.contains("rename"), "{message}");
        assert!(
            message.contains(&artifact.display().to_string()),
            "{message}"
        );
        assert!(
            message.contains(&fixed_root.display().to_string()),
            "{message}"
        );
        assert!(message.contains("running process"), "{message}");
        assert!(
            message.contains("cargo build -p gwt --bin gwtd"),
            "{message}"
        );
        assert!(message.contains("SHA256"), "{message}");
        assert_eq!(std::fs::read(&artifact).unwrap(), b"driver");
        drop(lock);
    }

    #[cfg(windows)]
    #[test]
    fn artifact_access_denied_explains_recovery() {
        let error = artifact_io("rename", &[Path::new("target/debug/gwtd.exe")], || {
            Err::<(), _>(io::Error::from_raw_os_error(5))
        })
        .unwrap_err();
        let message = error.to_string();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(message.contains("os error 5"), "{message}");
        assert!(message.contains("running process"), "{message}");
        assert!(
            message.contains("cargo build -p gwt --bin gwtd"),
            "{message}"
        );
        assert!(message.contains("SHA256"), "{message}");
    }

    #[cfg(windows)]
    #[test]
    fn relink_failure_records_recovery_without_changing_the_result() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        for (exit, file, expected) in [
            (7, "GWTD.EXE", true),
            (0, "gwtd.exe", false),
            (7, "other-gwtd.exe", false),
        ] {
            let command = format!(
                r#"powershell.exe -NoProfile -NonInteractive -Command "Write-Output 'error: failed to remove file target/debug/{file}'; Write-Output 'Caused by: Access is denied. (os error 5)'; exit {exit}""#
            );
            let (record, transcript) =
                super::super::run_verification(dir.path(), "driver-diagnostic", &[command])
                    .unwrap();
            assert_eq!(record.commands[0].exit_code, exit, "{transcript}");
            assert_eq!(record.all_passed, exit == 0, "{transcript}");
            assert_eq!(
                transcript.contains("running process"),
                expected,
                "{transcript}"
            );
            assert_eq!(
                record.commands[0].output_tail.contains("SHA256"),
                expected,
                "{transcript}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn artifact_io_recovers_after_observed_sharing_violation() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("gwtd.exe");
        let fixed = dir.path().join("fixed.exe");
        std::fs::write(&original, b"driver").unwrap();
        let mut lock = Some(
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(3)
                .open(&original)
                .unwrap(),
        );
        let mut attempts = 0;
        artifact_io("rename", &[&original, &fixed], || {
            attempts += 1;
            let result = std::fs::rename(&original, &fixed);
            if attempts == 1 {
                assert_eq!(result.as_ref().unwrap_err().raw_os_error(), Some(32));
                // Release only after observing the real OS failure, not a timer.
                drop(lock.take());
            }
            result
        })
        .unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(std::fs::read(&fixed).unwrap(), b"driver");
    }

    #[cfg(windows)]
    #[test]
    fn artifact_io_retries_lock_violations() {
        let mut attempts = 0;
        artifact_io("read", &[Path::new("driver.exe")], || {
            attempts += 1;
            if attempts == 1 {
                Err(io::Error::from_raw_os_error(33))
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(attempts, 2);
    }

    #[test]
    fn artifact_io_does_not_retry_unrelated_errors() {
        let mut attempts = 0;
        let error = artifact_io("copy", &[Path::new("from"), Path::new("to")], || {
            attempts += 1;
            Err::<(), _>(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert_eq!(attempts, 1);
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("copy from -> to"));
        assert!(!error.to_string().contains("running process"));
    }

    #[test]
    fn stale_or_unknown_build_source_is_refused() {
        assert!(validate_source_head("old-head", "new-head").is_err());
        assert!(validate_source_head("", "head").is_err());
        assert!(validate_source_head("head", "head").is_ok());
    }

    #[test]
    fn foreign_gwt_build_is_refused_but_other_projects_keep_their_own_head() {
        let dir = tempfile::tempdir().unwrap();
        // An unrelated project must not compare its HEAD with gwt's commit.
        assert!(validate_for_head(dir.path(), Some("project-head"), "gwt-head").is_ok());
        std::fs::create_dir_all(dir.path().join("crates/gwt")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/gwt\"]\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("crates/gwt/Cargo.toml"),
            "[package]\nname = \"gwt\"\n[features]\ntest-gh-guard = []\n[[bin]]\nname = \"gwtd\"\npath = \"src/bin/gwtd.rs\"\n").unwrap();
        assert!(validate_for_head(dir.path(), Some("project-head"), "foreign-head").is_err());
        assert!(validate_for_head(dir.path(), Some("project-head"), "project-head").is_ok());
    }
}
