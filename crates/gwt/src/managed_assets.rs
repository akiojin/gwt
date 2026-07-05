use std::{
    io,
    path::{Path, PathBuf},
};

use crate::cli::gwtd_resolver::{
    default_development_fallbacks, default_installed_candidates, resolve_gwtd_path_with,
    GwtdResolutionInputs,
};
use crate::native_app::{GUI_FRONT_DOOR_BINARY_NAME, INTERNAL_DAEMON_BINARY_NAME};
use gwt_agent::AgentId;
use gwt_skills::{
    distribute_to_worktree_for_targets, generate_codex_hooks_for_mode,
    generate_coordination_guidance_for_claude, generate_coordination_guidance_for_codex,
    generate_hermes_hooks, generate_openclaw_hooks, generate_opencode_hooks,
    generate_settings_local, update_git_exclude, update_git_exclude_for_targets,
    CodexHookDiscoveryMode, ManagedAssetTarget, SessionKind,
};

pub fn refresh_managed_gwt_assets_for_worktree(worktree: &Path) -> io::Result<()> {
    crate::cli::memory::migrate_legacy_memory_file(worktree).ok();
    crate::cli::discussion::migrate_legacy_discussions_file(worktree).ok();
    materialize_managed_gwt_assets_for_targets(
        worktree,
        &ManagedAssetTarget::ALL,
        CodexHookDiscoveryMode::WorkspaceHome,
        session_kind_for_worktree(worktree),
    )?;
    update_git_exclude(worktree).map_err(|error| {
        io::Error::other(format!("failed to update gwt managed excludes: {error}"))
    })?;
    Ok(())
}

pub fn refresh_managed_gwt_assets_for_agent(worktree: &Path, agent_id: &AgentId) -> io::Result<()> {
    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
        worktree,
        agent_id,
        CodexHookDiscoveryMode::WorkspaceHome,
        session_kind_for_worktree(worktree),
    )
}

pub fn refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
    worktree: &Path,
    agent_id: &AgentId,
    codex_hook_discovery_mode: CodexHookDiscoveryMode,
    session_kind: SessionKind,
) -> io::Result<()> {
    let targets = managed_targets_for_agent(agent_id)
        .into_iter()
        .collect::<Vec<_>>();
    materialize_managed_gwt_assets_for_targets(
        worktree,
        &targets,
        codex_hook_discovery_mode,
        session_kind,
    )?;
    let exclude_targets = detect_existing_managed_asset_targets(worktree);
    update_git_exclude_for_targets(worktree, &exclude_targets).map_err(|error| {
        io::Error::other(format!("failed to update gwt managed excludes: {error}"))
    })?;
    Ok(())
}

pub fn refresh_existing_managed_gwt_assets_for_worktree(worktree: &Path) -> io::Result<()> {
    let targets = detect_existing_managed_asset_targets(worktree);
    materialize_managed_gwt_assets_for_targets(
        worktree,
        &targets,
        CodexHookDiscoveryMode::WorkspaceHome,
        session_kind_for_worktree(worktree),
    )?;
    update_git_exclude_for_targets(worktree, &targets).map_err(|error| {
        io::Error::other(format!("failed to update gwt managed excludes: {error}"))
    })?;
    Ok(())
}

/// Determine the [`SessionKind`] for a non-launch (re-)materialization
/// (SPEC-3247 FR-002 / FR-004). The refreshers that use this run either
/// (a) inside an agent process — hook-time re-materialization, where the agent
/// inherited `GWT_SESSION_KIND` from its launch env — or (b) from the GUI /
/// startup / tests, where no such signal exists. Reading the ambient signal
/// therefore keeps an intake agent's guidance intake on re-materialization,
/// while every other caller decodes to Execution and preserves the current
/// producing-work behavior. (The launch path does NOT use this: it passes the
/// kind from `config.is_ephemeral` directly, because the launching GUI process
/// has no intake env of its own — the signal is written into the *child*.)
///
/// `worktree` is currently unused because the deterministic intake-worktree
/// probe lives in a binary-only module; the ambient signal is the lib-visible
/// source of truth and is authoritative for the hook-time case this guards.
fn session_kind_for_worktree(_worktree: &Path) -> SessionKind {
    SessionKind::from_env()
}

fn materialize_managed_gwt_assets_for_targets(
    worktree: &Path,
    targets: &[ManagedAssetTarget],
    codex_hook_discovery_mode: CodexHookDiscoveryMode,
    session_kind: SessionKind,
) -> io::Result<()> {
    // Fail fast with a clear, attributed error when the worktree was not
    // properly created (e.g. branch/worktree materialization failed). Without
    // this guard, distribution would silently `create_dir_all` a phantom tree
    // and the failure would surface much later as a misleading
    // "failed to generate Claude coordination skill: No such file or directory"
    // — attributing a worktree-setup failure to the skill writer.
    if !worktree.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "gwt managed assets: worktree is not a ready directory \
                 (branch/worktree creation likely failed): {}",
                worktree.display()
            ),
        ));
    }
    distribute_to_worktree_for_targets(worktree, targets).map_err(|error| {
        io::Error::other(format!("failed to distribute gwt managed assets: {error}"))
    })?;
    if targets.is_empty() {
        return Ok(());
    }
    let _hook_bin_guard = install_hook_bin_override()?;
    if targets.contains(&ManagedAssetTarget::ClaudeCode) {
        generate_settings_local(worktree).map_err(|error| {
            io::Error::other(format!(
                "failed to regenerate Claude hook settings: {error}"
            ))
        })?;
        generate_coordination_guidance_for_claude(worktree, session_kind).map_err(|error| {
            io::Error::other(format!(
                "failed to generate Claude coordination skill: {error}"
            ))
        })?;
    }
    if targets.contains(&ManagedAssetTarget::Codex) {
        generate_codex_hooks_for_mode(worktree, codex_hook_discovery_mode).map_err(|error| {
            io::Error::other(format!("failed to regenerate Codex hook settings: {error}"))
        })?;
        generate_coordination_guidance_for_codex(worktree, session_kind).map_err(|error| {
            io::Error::other(format!(
                "failed to generate Codex coordination skill: {error}"
            ))
        })?;
    }
    if targets.contains(&ManagedAssetTarget::OpenCode) {
        generate_opencode_hooks(worktree).map_err(|error| {
            io::Error::other(format!(
                "failed to regenerate OpenCode hook settings: {error}"
            ))
        })?;
    }
    if targets.contains(&ManagedAssetTarget::OpenClaw) {
        generate_openclaw_hooks(worktree).map_err(|error| {
            io::Error::other(format!(
                "failed to regenerate OpenClaw hook settings: {error}"
            ))
        })?;
    }
    if targets.contains(&ManagedAssetTarget::Hermes) {
        generate_hermes_hooks(worktree).map_err(|error| {
            io::Error::other(format!(
                "failed to regenerate Hermes hook settings: {error}"
            ))
        })?;
    }
    Ok(())
}

fn managed_targets_for_agent(agent_id: &AgentId) -> Option<ManagedAssetTarget> {
    match agent_id {
        AgentId::ClaudeCode => Some(ManagedAssetTarget::ClaudeCode),
        AgentId::Codex => Some(ManagedAssetTarget::Codex),
        AgentId::OpenCode => Some(ManagedAssetTarget::OpenCode),
        AgentId::OpenClaw => Some(ManagedAssetTarget::OpenClaw),
        AgentId::Hermes => Some(ManagedAssetTarget::Hermes),
        AgentId::Antigravity | AgentId::Gemini | AgentId::Copilot | AgentId::Custom(_) => None,
    }
}

fn detect_existing_managed_asset_targets(worktree: &Path) -> Vec<ManagedAssetTarget> {
    let mut targets = Vec::new();
    push_existing_target(
        &mut targets,
        worktree.join(".claude/skills").exists()
            || worktree.join(".claude/commands").exists()
            || worktree.join(".claude/settings.local.json").exists(),
        ManagedAssetTarget::ClaudeCode,
    );
    push_existing_target(
        &mut targets,
        worktree.join(".codex/skills").exists() || worktree.join(".codex/hooks.json").exists(),
        ManagedAssetTarget::Codex,
    );
    push_existing_target(
        &mut targets,
        worktree.join(".gwt/opencode").exists(),
        ManagedAssetTarget::OpenCode,
    );
    push_existing_target(
        &mut targets,
        worktree.join(".gwt/openclaw").exists(),
        ManagedAssetTarget::OpenClaw,
    );
    push_existing_target(
        &mut targets,
        worktree.join(".gwt/hermes").exists(),
        ManagedAssetTarget::Hermes,
    );
    targets
}

fn push_existing_target(
    targets: &mut Vec<ManagedAssetTarget>,
    exists: bool,
    target: ManagedAssetTarget,
) {
    if exists && !targets.contains(&target) {
        targets.push(target);
    }
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
    if is_named_gwtd_binary(current_exe) {
        return current_exe.to_path_buf();
    }

    if should_prefer_path_gwt(current_exe) {
        let path_candidate = lookup(INTERNAL_DAEMON_BINARY_NAME).filter(|candidate| {
            !same_path(candidate, current_exe) && !is_bunx_temp_executable(candidate)
        });
        let sibling_candidate = sibling_daemon_binary(current_exe);
        let trusted_candidates = path_candidate
            .clone()
            .into_iter()
            .chain(sibling_candidate.clone())
            .collect::<Vec<_>>();
        let resolved = resolve_gwtd_path_with(GwtdResolutionInputs {
            explicit_bin_path: None,
            path_lookup: Box::new(move |command| {
                (command == INTERNAL_DAEMON_BINARY_NAME)
                    .then(|| path_candidate.clone())
                    .flatten()
            }),
            installed_candidates: sibling_candidate
                .clone()
                .into_iter()
                .chain(default_installed_candidates(Some(current_exe)))
                .collect(),
            development_fallbacks: default_development_fallbacks(),
            is_file: Box::new(move |path| {
                path.is_file() || trusted_candidates.iter().any(|candidate| candidate == path)
            }),
        });
        if let Some(resolved) = resolved {
            return resolved;
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
    fn materialize_into_missing_worktree_fails_with_clear_attribution() {
        // #fix: when the launch's worktree was never created (branch/worktree
        // materialization failed), managed-asset materialization must fail fast
        // with a clear, attributed error — NOT the misleading downstream
        // "failed to generate Claude coordination skill: No such file or
        // directory" that points at the skill writer instead of the worktree.
        let missing = std::env::temp_dir()
            .join(format!("gwt-missing-worktree-{}", std::process::id()))
            .join("issue-3206");
        let err = super::refresh_managed_gwt_assets_for_worktree(&missing)
            .expect_err("a missing worktree must error");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        let msg = err.to_string();
        assert!(
            msg.contains("worktree is not a ready directory"),
            "error must attribute to the worktree, got: {msg}"
        );
        assert!(
            msg.contains("issue-3206"),
            "error must name the failing worktree path, got: {msg}"
        );
    }

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
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
