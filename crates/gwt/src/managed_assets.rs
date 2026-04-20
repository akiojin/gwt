use std::{
    io,
    path::{Path, PathBuf},
};

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
        if let Some(candidate) = lookup("gwt").filter(|candidate| {
            !same_path(candidate, current_exe) && !is_bunx_temp_executable(candidate)
        }) {
            return candidate;
        }
    }
    current_exe.to_path_buf()
}

fn should_prefer_path_gwt(current_exe: &Path) -> bool {
    is_bunx_temp_executable(current_exe) || !is_named_gwt_binary(current_exe)
}

fn is_named_gwt_binary(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .next_back()
        .map(|value| value.trim_end_matches(".exe").to_string())
        .is_some_and(|value| value.eq_ignore_ascii_case("gwt"))
}

fn is_bunx_temp_executable(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .any(|segment| segment.starts_with("bunx-"))
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
    use std::path::{Path, PathBuf};

    use super::resolve_public_gwt_bin_with_lookup;

    #[test]
    fn bunx_temp_current_exe_prefers_stable_path_gwt() {
        let current_exe = Path::new(
            r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let stable = PathBuf::from(r"C:\Users\Example\.bun\bin\gwt.exe");

        let resolved = resolve_public_gwt_bin_with_lookup(current_exe, |command| {
            assert_eq!(command, "gwt");
            Some(stable.clone())
        });

        assert_eq!(resolved, stable);
    }

    #[test]
    fn stable_gwt_current_exe_is_kept_without_path_lookup() {
        let current_exe = Path::new(r"C:\Users\Example\.bun\bin\gwt.exe");

        let resolved = resolve_public_gwt_bin_with_lookup(current_exe, |_command| {
            panic!("stable gwt binary should not hit PATH lookup");
        });

        assert_eq!(resolved, current_exe);
    }

    #[test]
    fn bunx_temp_current_exe_keeps_current_when_path_only_returns_bunx_temp() {
        let current_exe = Path::new(
            r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let path_candidate = PathBuf::from(
            r"C:\Users\Example\AppData\Local\Temp\bunx-2222222222-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );

        let resolved = resolve_public_gwt_bin_with_lookup(current_exe, |_command| {
            Some(path_candidate.clone())
        });

        assert_eq!(resolved, current_exe);
    }
}
