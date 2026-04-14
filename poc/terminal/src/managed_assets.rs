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
