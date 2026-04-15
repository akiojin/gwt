use std::io;
use std::path::Path;

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
    let current_exe = std::env::current_exe()
        .map_err(|error| io::Error::other(format!("current_exe: {error}")))?;
    Ok(EnvVarGuard::set("GWT_HOOK_BIN", current_exe))
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
