use std::{
    io,
    path::{Path, PathBuf},
};

use crate::native_app::{GUI_FRONT_DOOR_BINARY_NAME, INTERNAL_DAEMON_BINARY_NAME};
use gwt_skills::{
    distribute_to_worktree, generate_codex_hooks, generate_settings_local, update_git_exclude,
};

pub fn refresh_managed_gwt_assets_for_worktree(worktree: &Path) -> io::Result<()> {
    distribute_to_worktree(worktree).map_err(|error| {
        io::Error::other(format!("failed to distribute gwt managed assets: {error}"))
    })?;
    update_git_exclude(worktree).map_err(|error| {
        io::Error::other(format!("failed to update gwt managed excludes: {error}"))
    })?;
    let _hook_bin_guard = install_hook_bin_override()?;
    generate_settings_local(worktree).map_err(|error| {
        io::Error::other(format!(
            "failed to regenerate Claude hook settings: {error}"
        ))
    })?;
    generate_codex_hooks(worktree).map_err(|error| {
        io::Error::other(format!("failed to regenerate Codex hook settings: {error}"))
    })?;
    Ok(())
}

fn install_hook_bin_override() -> io::Result<EnvVarGuard> {
    if std::env::var_os("GWT_HOOK_BIN").is_some() {
        return Ok(EnvVarGuard::noop("GWT_HOOK_BIN"));
    }
    let hook_bin = resolve_public_gwt_bin_path()?;
    Ok(EnvVarGuard::set("GWT_HOOK_BIN", hook_bin))
}

pub fn resolve_public_gwt_bin_path() -> io::Result<PathBuf> {
    let current_exe = std::env::current_exe()
        .map_err(|error| io::Error::other(format!("current_exe: {error}")))?;
    Ok(resolve_public_gwt_bin_with_lookup(
        &current_exe,
        |command| which::which(command).ok(),
    ))
}

pub fn resolve_public_gwt_bin_with_lookup(
    current_exe: &Path,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> PathBuf {
    if should_prefer_path_gwt(current_exe) {
        if let Some(candidate) = lookup(INTERNAL_DAEMON_BINARY_NAME).filter(|candidate| {
            !same_path(candidate, current_exe) && !is_bunx_temp_executable(candidate)
        }) {
            return candidate;
        }
        if let Some(candidate) = sibling_daemon_binary(current_exe) {
            return candidate;
        }
    }
    current_exe.to_path_buf()
}

fn should_prefer_path_gwt(current_exe: &Path) -> bool {
    is_bunx_temp_executable(current_exe) || !is_named_gwtd_binary(current_exe)
}

fn strip_windows_exe_suffix(value: &str) -> &str {
    value
        .rsplit_once('.')
        .filter(|(_, ext)| ext.eq_ignore_ascii_case("exe"))
        .map(|(stem, _)| stem)
        .unwrap_or(value)
}

fn is_named_gwt_binary(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .next_back()
        .map(|value| strip_windows_exe_suffix(&value).to_string())
        .is_some_and(|value| value.eq_ignore_ascii_case(GUI_FRONT_DOOR_BINARY_NAME))
}

fn is_named_gwtd_binary(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .next_back()
        .map(|value| strip_windows_exe_suffix(&value).to_string())
        .is_some_and(|value| value.eq_ignore_ascii_case(INTERNAL_DAEMON_BINARY_NAME))
}

fn is_bunx_temp_executable(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .any(|segment| segment.starts_with("bunx-"))
}

fn sibling_daemon_binary(path: &Path) -> Option<PathBuf> {
    if !is_named_gwt_binary(path) {
        return None;
    }
    let sibling_name = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("exe") => {
            format!("{INTERNAL_DAEMON_BINARY_NAME}.exe")
        }
        _ => INTERNAL_DAEMON_BINARY_NAME.to_string(),
    };
    Some(path.with_file_name(sibling_name))
}

fn normalized_path_segments(path: &Path) -> Vec<String> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = dunce::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = dunce::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
    restore: bool,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self {
            key,
            previous,
            restore: true,
        }
    }

    fn noop(key: &'static str) -> Self {
        Self {
            key,
            previous: None,
            restore: false,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if !self.restore {
            return;
        }
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
    };

    use super::{
        is_bunx_temp_executable, is_named_gwt_binary, is_named_gwtd_binary,
        normalized_path_segments, resolve_public_gwt_bin_with_lookup, same_path,
        should_prefer_path_gwt, EnvVarGuard,
    };

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn bunx_temp_current_exe_prefers_stable_path_gwtd() {
        let current_exe = Path::new(
            r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let stable = PathBuf::from(r"C:\Users\Example\.bun\bin\gwtd.exe");

        let resolved = resolve_public_gwt_bin_with_lookup(current_exe, |command| {
            assert_eq!(command, "gwtd");
            Some(stable.clone())
        });

        assert_eq!(resolved, stable);
    }

    #[test]
    fn stable_gwtd_current_exe_is_kept_without_path_lookup() {
        let current_exe = Path::new(r"C:\Users\Example\.bun\bin\gwtd.exe");

        let resolved = resolve_public_gwt_bin_with_lookup(current_exe, |_command| {
            panic!("stable gwtd binary should not hit PATH lookup");
        });

        assert_eq!(resolved, current_exe);
    }

    #[test]
    fn bunx_temp_current_exe_falls_back_to_gwtd_sibling_when_path_only_returns_bunx_temp() {
        let current_exe = Path::new(
            r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let path_candidate = PathBuf::from(
            r"C:\Users\Example\AppData\Local\Temp\bunx-2222222222-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwtd.exe",
        );

        let resolved = resolve_public_gwt_bin_with_lookup(current_exe, |_command| {
            Some(path_candidate.clone())
        });

        assert_eq!(resolved, current_exe.with_file_name("gwtd.exe"));
    }

    #[test]
    fn gui_front_door_current_exe_prefers_daemon_sibling_when_path_lookup_is_missing() {
        let current_exe = Path::new(r"C:\Program Files\GWT\gwt.exe");

        let resolved = resolve_public_gwt_bin_with_lookup(current_exe, |_command| None);

        assert_eq!(resolved, current_exe.with_file_name("gwtd.exe"));
    }

    #[test]
    fn path_helpers_identify_named_binaries_and_temp_layouts() {
        let stable = Path::new(r"C:\Users\Example\.bun\bin\gwt.exe");
        let stable_upper = Path::new(r"C:\Users\Example\.bun\bin\gwt.EXE");
        let daemon_upper = Path::new(r"C:\Program Files\GWT\GWTD.EXE");
        let bunx = Path::new(
            r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let other = Path::new(r"C:\Users\Example\.bun\bin\other.exe");

        assert!(is_named_gwt_binary(stable));
        assert!(is_named_gwt_binary(stable_upper));
        assert!(is_named_gwtd_binary(daemon_upper));
        assert!(!is_named_gwt_binary(other));
        assert!(is_bunx_temp_executable(bunx));
        assert!(!is_bunx_temp_executable(stable));
        assert_eq!(
            normalized_path_segments(Path::new(r"C:\Users\Example\.bun\bin\gwt.exe"))
                .last()
                .map(String::as_str),
            Some("gwt.exe")
        );
        assert!(should_prefer_path_gwt(stable));
        assert!(should_prefer_path_gwt(bunx));
        assert!(should_prefer_path_gwt(other));
    }

    #[test]
    fn same_path_and_env_var_guard_preserve_previous_values() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        std::fs::create_dir_all(&nested).expect("create nested");

        assert!(same_path(&nested, &dir.path().join("nested")));

        std::env::set_var("GWT_MANAGED_ASSETS_TEST", "before");
        {
            let _scoped = EnvVarGuard::set("GWT_MANAGED_ASSETS_TEST", "during");
            assert_eq!(
                std::env::var("GWT_MANAGED_ASSETS_TEST").as_deref(),
                Ok("during")
            );
        }
        assert_eq!(
            std::env::var("GWT_MANAGED_ASSETS_TEST").as_deref(),
            Ok("before")
        );

        {
            let _noop = EnvVarGuard::noop("GWT_MANAGED_ASSETS_TEST");
            assert_eq!(
                std::env::var("GWT_MANAGED_ASSETS_TEST").as_deref(),
                Ok("before")
            );
        }
        std::env::remove_var("GWT_MANAGED_ASSETS_TEST");
    }
}
