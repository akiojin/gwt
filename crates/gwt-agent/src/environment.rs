//! Launch environment composition for host and container agent processes.

#[cfg(not(windows))]
use std::path::PathBuf;
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

use crate::{
    session::{
        GWT_BIN_PATH_ENV, GWT_HOOK_FORWARD_TOKEN_ENV, GWT_HOOK_FORWARD_URL_ENV, GWT_SESSION_ID_ENV,
        GWT_SESSION_RUNTIME_PATH_ENV,
    },
    types::LaunchRuntimeTarget,
};

const GWT_PROJECT_ROOT_ENV: &str = "GWT_PROJECT_ROOT";
const GWT_REPO_HASH_ENV: &str = "GWT_REPO_HASH";
const GWT_WORKTREE_HASH_ENV: &str = "GWT_WORKTREE_HASH";
const INHERITED_TERMINAL_COLOR_SUPPRESSOR_ENV_KEYS: &[&str] = &["NO_COLOR"];
const INHERITED_LAUNCH_ENV_KEYS: &[&str] = &[
    GWT_BIN_PATH_ENV,
    GWT_HOOK_FORWARD_TOKEN_ENV,
    GWT_HOOK_FORWARD_URL_ENV,
    GWT_PROJECT_ROOT_ENV,
    GWT_REPO_HASH_ENV,
    GWT_SESSION_ID_ENV,
    GWT_SESSION_RUNTIME_PATH_ENV,
    GWT_WORKTREE_HASH_ENV,
];

/// Return the current host process environment with GUI-launch PATH gaps filled.
pub fn host_process_env() -> HashMap<String, String> {
    hydrate_host_base_env(std::env::vars())
}

/// Fill common GUI-launch PATH gaps in a host base environment.
pub(crate) fn hydrate_host_base_env<I>(base_env: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut env: HashMap<String, String> = base_env.into_iter().collect();
    normalize_windows_path_key(&mut env);
    remove_inherited_launch_env(&mut env);
    hydrate_host_path(&mut env);
    env
}

/// Compute the hydrated `PATH` for the given base environment.
///
/// On non-Windows the result includes macOS `/usr/libexec/path_helper` output
/// and `~/.bun/bin`, `~/.local/bin`, `~/.cargo/bin` if they exist. On Windows
/// the result echoes the input PATH (if any) without modification. Returns
/// `None` only when no PATH entries can be derived (input had no PATH and
/// hydration found no fallbacks).
pub fn compute_hydrated_path<I>(base_env: I) -> Option<String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut env: HashMap<String, String> = base_env.into_iter().collect();
    normalize_windows_path_key(&mut env);
    hydrate_host_path(&mut env);
    env.get("PATH").cloned()
}

/// Apply host PATH hydration to the running process's `std::env`.
///
/// MUST be called from `main` BEFORE any thread is spawned and before any
/// other env mutation. macOS GUI launches via launchd inherit the minimal
/// `/usr/bin:/bin:/usr/sbin:/sbin` PATH which omits `/usr/local/bin` and
/// `/opt/homebrew/bin`; without this hydration, child processes spawned by
/// the app cannot resolve `docker`, `gh`, `claude`, `codex`, `bunx`, or `npx`
/// that live in those directories.
///
/// Idempotent: re-running on an already-hydrated PATH yields the same value
/// because `push_unique_path` deduplicates entries. No-op on Windows.
///
/// Reads the current process env via `vars_os` (not `vars`) so a single
/// non-Unicode environment variable does not panic startup. Skips writing to
/// `std::env` if the computed PATH is empty so we never blank a usable PATH.
pub fn apply_host_path_hydration_to_std_env() {
    if cfg!(windows) {
        return;
    }
    let before = std::env::var("PATH").unwrap_or_default();
    let before_count = std::env::split_paths(&before).count();
    let home = std::env::var("HOME").unwrap_or_default();
    let Some(hydrated) = current_process_hydrated_path() else {
        tracing::info!(
            target: "gwt::launch::startup",
            stage = "path_hydration",
            outcome = "skipped",
            reason = "no_hydrated_path",
            path_before = %before,
            path_entry_count_before = before_count,
            home = %home,
            "host PATH hydration skipped"
        );
        return;
    };
    let after_count = std::env::split_paths(&hydrated).count();
    let added = after_count.saturating_sub(before_count);
    std::env::set_var("PATH", &hydrated);
    tracing::info!(
        target: "gwt::launch::startup",
        stage = "path_hydration",
        outcome = "applied",
        path_before = %before,
        path_after = %hydrated,
        path_entry_count_before = before_count,
        path_entry_count_after = after_count,
        path_entries_added = added,
        home = %home,
        "host PATH hydration applied"
    );
}

/// Compute the hydrated PATH from the running process env.
///
/// Returns `None` when the hydrated PATH is empty so `apply_host_path_hydration_to_std_env`
/// never overwrites a usable `std::env::PATH` with a blank value (which would
/// disable command lookup for subsequent `Command::new(...)` calls).
fn current_process_hydrated_path() -> Option<String> {
    let base_env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.into_string().ok()?;
            let value = value.into_string().ok()?;
            Some((key, value))
        })
        .collect();
    compute_hydrated_path(base_env).filter(|hydrated| !hydrated.is_empty())
}

/// Effective environment assembled from the active profile and launch context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchEnvironment {
    base_env: HashMap<String, String>,
    profile_env: HashMap<String, String>,
    remove_env: Vec<String>,
    override_env: HashMap<String, String>,
}

impl LaunchEnvironment {
    /// Build an empty launch environment. This is useful for applying only
    /// launch-derived overrides such as `GWT_PROJECT_ROOT`.
    pub fn empty() -> Self {
        Self {
            base_env: HashMap::new(),
            profile_env: HashMap::new(),
            remove_env: Vec::new(),
            override_env: HashMap::new(),
        }
    }

    /// Build from a host base environment and install terminal defaults.
    pub fn from_base_env<I>(base_env: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut env: HashMap<String, String> = base_env.into_iter().collect();
        normalize_windows_path_key(&mut env);
        apply_required_terminal_defaults(&mut env);
        Self {
            base_env: env,
            profile_env: HashMap::new(),
            remove_env: inherited_terminal_color_suppressor_remove_env(),
            override_env: HashMap::new(),
        }
    }

    /// Build from the active profile. Host launches inherit the current OS
    /// environment; Docker launches start from an empty base.
    pub fn from_active_profile(
        config_path: &Path,
        runtime_target: LaunchRuntimeTarget,
    ) -> Result<Self, String> {
        match runtime_target {
            LaunchRuntimeTarget::Host => {
                Self::from_active_profile_with_base(config_path, host_process_env())
            }
            LaunchRuntimeTarget::Docker => Self::from_active_profile_with_base(
                config_path,
                std::iter::empty::<(String, String)>(),
            ),
        }
    }

    /// Build from the active profile using an explicit host base environment.
    pub fn from_active_profile_with_base<I>(config_path: &Path, base_env: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut settings = if config_path.exists() {
            gwt_config::Settings::load_from_path(config_path).map_err(|error| error.to_string())?
        } else {
            gwt_config::Settings::default()
        };
        let active_name = settings.profiles.normalize_active_profile().name;
        let Some(profile) = settings.profiles.get(&active_name) else {
            return Err(format!("active profile not found: {active_name}"));
        };

        let inherited_remove_env = inherited_terminal_color_suppressor_remove_env();
        let profile_remove_env = normalized_remove_env(&profile.disabled_env);
        let remove_env = merged_remove_env(&inherited_remove_env, &profile_remove_env);
        let mut base_env: HashMap<String, String> = base_env.into_iter().collect();
        normalize_windows_path_key(&mut base_env);
        for key in &remove_env {
            base_env.remove(key);
        }
        apply_required_terminal_defaults(&mut base_env);

        Ok(Self {
            base_env,
            profile_env: profile.env_vars.clone(),
            remove_env,
            override_env: HashMap::new(),
        })
    }

    #[cfg(test)]
    fn from_active_profile_for_runtime_with_base<I>(
        config_path: &Path,
        runtime_target: LaunchRuntimeTarget,
        base_env: I,
    ) -> Result<Self, String>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        match runtime_target {
            LaunchRuntimeTarget::Host => {
                Self::from_active_profile_with_base(config_path, hydrate_host_base_env(base_env))
            }
            LaunchRuntimeTarget::Docker => Self::from_active_profile_with_base(
                config_path,
                std::iter::empty::<(String, String)>(),
            ),
        }
    }

    /// Set the project root as a launch-derived override.
    ///
    /// Also exports `GWT_REPO_HASH` and `GWT_WORKTREE_HASH` derived from the
    /// same resolved path. These travel with `GWT_PROJECT_ROOT` so that
    /// skill-driven index runner calls can reconstruct the on-disk DB path
    /// without re-deriving the hashes on every invocation (Issue #2933 /
    /// SPEC-1939 US-2 AC-11). This is the canonical injection point after the
    /// worktree is resolved; the hashes are omitted (rather than blanked) when
    /// the path has no `origin` remote or cannot be canonicalized.
    pub fn with_project_root(mut self, project_root: impl AsRef<Path>) -> Self {
        let project_root =
            gwt_core::paths::normalize_windows_child_process_path(project_root.as_ref());
        self.override_env.insert(
            GWT_PROJECT_ROOT_ENV.to_string(),
            project_root.display().to_string(),
        );
        if let Some(repo_hash) = gwt_core::repo_hash::detect_repo_hash(&project_root) {
            self.override_env.insert(
                GWT_REPO_HASH_ENV.to_string(),
                repo_hash.as_str().to_string(),
            );
        }
        if let Ok(worktree_hash) = gwt_core::worktree_hash::compute_worktree_hash(&project_root) {
            self.override_env.insert(
                GWT_WORKTREE_HASH_ENV.to_string(),
                worktree_hash.as_str().to_string(),
            );
        }
        self
    }

    /// Merge this environment into spawn parts.
    ///
    /// Merge order is profile/base env, inherited env removals, explicit launch
    /// env, and finally launch-derived overrides.
    pub fn apply_to_parts(
        &self,
        env_vars: &mut HashMap<String, String>,
        remove_env: &mut Vec<String>,
    ) {
        let explicit_env = std::mem::take(env_vars);
        let merged_remove_env = merged_remove_env(&self.remove_env, remove_env);

        let mut merged_env = self.base_env.clone();
        for key in &merged_remove_env {
            merged_env.remove(key);
        }
        merged_env.extend(self.profile_env.clone());
        merged_env.extend(explicit_env);
        merged_env.extend(self.override_env.clone());

        *env_vars = merged_env;
        *remove_env = merged_remove_env;
    }

    /// Return the resolved environment and inherited env removal list.
    pub fn into_parts(mut self) -> (HashMap<String, String>, Vec<String>) {
        self.base_env.extend(std::mem::take(&mut self.profile_env));
        self.base_env.extend(std::mem::take(&mut self.override_env));
        (self.base_env, self.remove_env)
    }

    pub fn remove_env(&self) -> &[String] {
        &self.remove_env
    }
}

fn apply_required_terminal_defaults(env: &mut HashMap<String, String>) {
    for key in INHERITED_TERMINAL_COLOR_SUPPRESSOR_ENV_KEYS {
        env.remove(*key);
    }
    let replace_term = env
        .get("TERM")
        .map(|value| value.trim().is_empty() || value.eq_ignore_ascii_case("dumb"))
        .unwrap_or(true);
    if replace_term {
        env.insert("TERM".to_string(), "xterm-256color".to_string());
    }
    let replace_colorterm = env
        .get("COLORTERM")
        .map(|value| value.trim().is_empty())
        .unwrap_or(true);
    if replace_colorterm {
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
    }
}

fn inherited_terminal_color_suppressor_remove_env() -> Vec<String> {
    INHERITED_TERMINAL_COLOR_SUPPRESSOR_ENV_KEYS
        .iter()
        .map(|key| (*key).to_string())
        .collect()
}

fn remove_inherited_launch_env(env: &mut HashMap<String, String>) {
    for key in INHERITED_LAUNCH_ENV_KEYS {
        env.remove(*key);
    }
}

#[cfg(windows)]
fn normalize_windows_path_key(env: &mut HashMap<String, String>) {
    if env.contains_key("PATH") {
        return;
    }
    let Some(existing_key) = env
        .keys()
        .find(|key| key.eq_ignore_ascii_case("PATH"))
        .cloned()
    else {
        return;
    };
    if let Some(value) = env.remove(&existing_key) {
        env.insert("PATH".to_string(), value);
    }
}

#[cfg(not(windows))]
fn normalize_windows_path_key(_env: &mut HashMap<String, String>) {}

#[cfg(windows)]
fn hydrate_host_path(_env: &mut HashMap<String, String>) {}

#[cfg(not(windows))]
fn hydrate_host_path(env: &mut HashMap<String, String>) {
    let mut entries = env
        .get("PATH")
        .map(|path| std::env::split_paths(path).collect::<Vec<_>>())
        .unwrap_or_default();

    #[cfg(target_os = "macos")]
    if let Some(path) = macos_path_helper_path(env.get("PATH").map(String::as_str)) {
        push_path_value(&mut entries, &path);
    }

    if let Some(home) = env.get("HOME").filter(|home| !home.trim().is_empty()) {
        let home = PathBuf::from(home);
        for relative in [".bun/bin", ".local/bin", ".cargo/bin"] {
            let candidate = home.join(relative);
            if candidate.is_dir() {
                push_unique_path(&mut entries, candidate);
            }
        }
    }

    if let Ok(path) = std::env::join_paths(&entries) {
        env.insert("PATH".to_string(), path.to_string_lossy().into_owned());
    }
}

#[cfg(target_os = "macos")]
fn macos_path_helper_path(current_path: Option<&str>) -> Option<String> {
    let mut command = gwt_core::process::hidden_command("/usr/libexec/path_helper");
    command.arg("-s");
    if let Some(path) = current_path {
        command.env("PATH", path);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_macos_path_helper_output(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_os = "macos")]
fn parse_macos_path_helper_output(output: &str) -> Option<String> {
    let start = output.find("PATH=\"")? + "PATH=\"".len();
    let rest = &output[start..];
    let end = rest.find("\";")?;
    Some(rest[..end].to_string())
}

#[cfg(target_os = "macos")]
fn push_path_value(entries: &mut Vec<PathBuf>, value: &str) {
    for path in std::env::split_paths(value) {
        push_unique_path(entries, path);
    }
}

#[cfg(not(windows))]
fn push_unique_path(entries: &mut Vec<PathBuf>, path: PathBuf) {
    if path.as_os_str().is_empty() {
        return;
    }
    if !entries.iter().any(|entry| entry == &path) {
        entries.push(path);
    }
}

fn merged_remove_env(base: &[String], additional: &[String]) -> Vec<String> {
    base.iter()
        .chain(additional.iter())
        .filter_map(|key| {
            let trimmed = key.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalized_remove_env(disabled_env: &[String]) -> Vec<String> {
    disabled_env
        .iter()
        .filter_map(|key| {
            let trimmed = key.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use gwt_config::{Profile, Settings};

    use super::*;

    fn write_profile_config(profile: Profile) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let profile_name = profile.name.clone();
        let mut settings = Settings::default();
        settings.profiles.profiles = vec![profile];
        settings.profiles.active = Some(profile_name);
        settings.save(&path).unwrap();
        (dir, path)
    }

    fn dev_profile() -> Profile {
        let mut profile = Profile::new("dev");
        profile
            .env_vars
            .insert("PATH".to_string(), "/profile/bin".to_string());
        profile
            .env_vars
            .insert("PROFILE_ONLY".to_string(), "yes".to_string());
        profile.disabled_env = vec!["SECRET".to_string()];
        profile
    }

    #[test]
    fn merges_active_profile_over_host_base() {
        let (_dir, config_path) = write_profile_config(dev_profile());
        let base_env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("KEEP".to_string(), "base".to_string()),
            ("SECRET".to_string(), "old".to_string()),
        ];

        let (env, remove_env) =
            LaunchEnvironment::from_active_profile_with_base(&config_path, base_env)
                .unwrap()
                .into_parts();

        assert_eq!(env.get("PATH").map(String::as_str), Some("/profile/bin"));
        assert_eq!(env.get("KEEP").map(String::as_str), Some("base"));
        assert_eq!(env.get("PROFILE_ONLY").map(String::as_str), Some("yes"));
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert!(!env.contains_key("SECRET"));
        assert_eq!(
            remove_env,
            vec!["NO_COLOR".to_string(), "SECRET".to_string()]
        );
    }

    #[test]
    fn uses_empty_base_for_docker_runtime() {
        let (_dir, config_path) = write_profile_config(dev_profile());
        let base_env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOST_ONLY".to_string(), "1".to_string()),
            ("SECRET".to_string(), "old".to_string()),
        ];

        let (env, remove_env) = LaunchEnvironment::from_active_profile_for_runtime_with_base(
            &config_path,
            LaunchRuntimeTarget::Docker,
            base_env,
        )
        .unwrap()
        .into_parts();

        assert_eq!(env.get("PATH").map(String::as_str), Some("/profile/bin"));
        assert_eq!(env.get("PROFILE_ONLY").map(String::as_str), Some("yes"));
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert!(!env.contains_key("HOST_ONLY"));
        assert!(!env.contains_key("SECRET"));
        assert_eq!(
            remove_env,
            vec!["NO_COLOR".to_string(), "SECRET".to_string()]
        );
    }

    #[test]
    fn applies_profile_base_then_explicit_env_then_project_root() {
        let (_dir, config_path) = write_profile_config(dev_profile());
        let base_env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("KEEP".to_string(), "base".to_string()),
            ("SECRET".to_string(), "old".to_string()),
        ];
        let launch_env = LaunchEnvironment::from_active_profile_with_base(&config_path, base_env)
            .unwrap()
            .with_project_root("/tmp/new-root");
        let mut env_vars = HashMap::from([
            ("PATH".to_string(), "/explicit/bin".to_string()),
            ("EXPLICIT".to_string(), "1".to_string()),
            (
                "GWT_PROJECT_ROOT".to_string(),
                "/tmp/stale-root".to_string(),
            ),
        ]);
        let mut remove_env = vec!["EXPLICIT_REMOVE".to_string()];

        launch_env.apply_to_parts(&mut env_vars, &mut remove_env);

        assert_eq!(
            env_vars.get("PATH").map(String::as_str),
            Some("/explicit/bin")
        );
        assert_eq!(env_vars.get("KEEP").map(String::as_str), Some("base"));
        assert_eq!(env_vars.get("EXPLICIT").map(String::as_str), Some("1"));
        assert_eq!(
            env_vars.get("GWT_PROJECT_ROOT").map(String::as_str),
            Some("/tmp/new-root")
        );
        assert!(!env_vars.contains_key("SECRET"));
        assert_eq!(
            remove_env,
            vec![
                "EXPLICIT_REMOVE".to_string(),
                "NO_COLOR".to_string(),
                "SECRET".to_string()
            ]
        );
    }

    #[test]
    fn with_project_root_exports_repo_and_worktree_hashes() {
        // Issue #2933 / SPEC-1939 US-2 AC-11: with_project_root must export
        // GWT_REPO_HASH and GWT_WORKTREE_HASH alongside GWT_PROJECT_ROOT so the
        // skill-driven index runner can reconstruct the DB path. A bare
        // `.git/config` with an origin remote is enough — detect_repo_hash reads
        // it directly without invoking the git binary.
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            repo.join(".git/config"),
            "[remote \"origin\"]\n    url = https://github.com/akiojin/gwt.git\n",
        )
        .unwrap();

        let (env, _) = LaunchEnvironment::empty()
            .with_project_root(&repo)
            .into_parts();

        assert!(env.contains_key(GWT_PROJECT_ROOT_ENV));

        let normalized = gwt_core::paths::normalize_windows_child_process_path(&repo);
        let expected_repo =
            gwt_core::repo_hash::compute_repo_hash("https://github.com/akiojin/gwt.git");
        assert_eq!(
            env.get(GWT_REPO_HASH_ENV).map(String::as_str),
            Some(expected_repo.as_str())
        );
        let expected_wt = gwt_core::worktree_hash::compute_worktree_hash(&normalized).unwrap();
        assert_eq!(
            env.get(GWT_WORKTREE_HASH_ENV).map(String::as_str),
            Some(expected_wt.as_str())
        );
    }

    #[test]
    fn with_project_root_omits_hashes_without_git_repo() {
        // A non-git directory has no origin: GWT_PROJECT_ROOT is still set, but
        // the hashes are simply omitted (no panic, no empty values).
        let dir = tempfile::tempdir().unwrap();
        let (env, _) = LaunchEnvironment::empty()
            .with_project_root(dir.path())
            .into_parts();

        assert!(env.contains_key(GWT_PROJECT_ROOT_ENV));
        assert!(!env.contains_key(GWT_REPO_HASH_ENV));
    }

    #[test]
    fn windows_launch_paths_project_root_strips_provider_and_verbatim_prefixes() {
        let (env, _) = LaunchEnvironment::empty()
            .with_project_root(
                r"Microsoft.PowerShell.Core\FileSystem::\\?\E:\gwt\work\20260525-0919",
            )
            .into_parts();

        assert_eq!(
            env.get(GWT_PROJECT_ROOT_ENV).map(String::as_str),
            Some(r"E:\gwt\work\20260525-0919")
        );
    }

    #[test]
    fn windows_launch_paths_project_root_strips_unc_verbatim_prefix() {
        let (env, _) = LaunchEnvironment::empty()
            .with_project_root(r"\\?\UNC\server\share\work")
            .into_parts();

        assert_eq!(
            env.get(GWT_PROJECT_ROOT_ENV).map(String::as_str),
            Some(r"\\server\share\work")
        );
    }

    #[test]
    fn remove_env_suppresses_inherited_env_without_suppressing_profile_env() {
        let (_dir, config_path) = write_profile_config(dev_profile());
        let base_env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("KEEP".to_string(), "base".to_string()),
        ];
        let launch_env =
            LaunchEnvironment::from_active_profile_with_base(&config_path, base_env).unwrap();
        let mut env_vars = HashMap::new();
        let mut remove_env = vec!["PATH".to_string()];

        launch_env.apply_to_parts(&mut env_vars, &mut remove_env);

        assert_eq!(
            env_vars.get("PATH").map(String::as_str),
            Some("/profile/bin")
        );
        assert_eq!(env_vars.get("KEEP").map(String::as_str), Some("base"));
        assert_eq!(
            remove_env,
            vec![
                "NO_COLOR".to_string(),
                "PATH".to_string(),
                "SECRET".to_string()
            ]
        );
    }

    #[test]
    fn from_base_env_installs_terminal_defaults() {
        let base_env = vec![("PATH".to_string(), "/usr/bin".to_string())];

        let (env, remove_env) = LaunchEnvironment::from_base_env(base_env).into_parts();

        assert_eq!(env.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert_eq!(remove_env, vec!["NO_COLOR".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn from_base_env_normalizes_windows_path_key() {
        let base_env = vec![("Path".to_string(), r"C:\Windows\System32".to_string())];

        let (env, _) = LaunchEnvironment::from_base_env(base_env).into_parts();

        assert_eq!(
            env.get("PATH").map(String::as_str),
            Some(r"C:\Windows\System32")
        );
        assert!(
            !env.contains_key("Path"),
            "launch env must expose one canonical PATH key"
        );
    }

    #[cfg(windows)]
    #[test]
    fn compute_hydrated_path_reads_windows_path_key() {
        let base_env = vec![("Path".to_string(), r"C:\Windows\System32".to_string())];

        assert_eq!(
            compute_hydrated_path(base_env).as_deref(),
            Some(r"C:\Windows\System32")
        );
    }

    #[test]
    fn from_base_env_replaces_dumb_terminal_type_and_suppresses_inherited_no_color() {
        let base_env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("TERM".to_string(), "dumb".to_string()),
            ("COLORTERM".to_string(), String::new()),
            ("NO_COLOR".to_string(), "1".to_string()),
        ];

        let (env, remove_env) = LaunchEnvironment::from_base_env(base_env).into_parts();

        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert!(
            !env.contains_key("NO_COLOR"),
            "inherited NO_COLOR must not suppress colors in gwt terminal panes"
        );
        assert_eq!(
            remove_env,
            vec!["NO_COLOR".to_string()],
            "NO_COLOR must be removed from inherited process env, not only omitted from explicit env_vars"
        );
    }

    #[test]
    fn profile_env_can_explicitly_disable_terminal_colors() {
        let mut profile = dev_profile();
        profile
            .env_vars
            .insert("NO_COLOR".to_string(), "1".to_string());
        let (_dir, config_path) = write_profile_config(profile);
        let base_env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("NO_COLOR".to_string(), "1".to_string()),
        ];

        let (env, remove_env) =
            LaunchEnvironment::from_active_profile_with_base(&config_path, base_env)
                .unwrap()
                .into_parts();

        assert_eq!(env.get("NO_COLOR").map(String::as_str), Some("1"));
        assert_eq!(
            remove_env,
            vec!["NO_COLOR".to_string(), "SECRET".to_string()],
            "profile NO_COLOR is explicit env and should be re-applied after inherited env removal"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn host_runtime_expands_minimal_gui_path_with_existing_user_bins() {
        let home = tempfile::tempdir().unwrap();
        for relative in [".bun/bin", ".local/bin", ".cargo/bin"] {
            std::fs::create_dir_all(home.path().join(relative)).unwrap();
        }
        let (_dir, config_path) = write_profile_config(Profile::new("default"));
        let base_env = vec![
            (
                "PATH".to_string(),
                "/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
            ),
            ("HOME".to_string(), home.path().display().to_string()),
        ];

        let (env, _) = LaunchEnvironment::from_active_profile_for_runtime_with_base(
            &config_path,
            LaunchRuntimeTarget::Host,
            base_env,
        )
        .unwrap()
        .into_parts();

        let path = env.get("PATH").expect("PATH");
        let entries = std::env::split_paths(path).collect::<Vec<_>>();
        assert!(entries.contains(&home.path().join(".bun/bin")));
        assert!(entries.contains(&home.path().join(".local/bin")));
        assert!(entries.contains(&home.path().join(".cargo/bin")));
    }

    #[cfg(not(windows))]
    #[test]
    fn docker_runtime_does_not_import_host_user_bins() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".bun/bin")).unwrap();
        let (_dir, config_path) = write_profile_config(Profile::new("default"));

        let (env, _) = LaunchEnvironment::from_active_profile_for_runtime_with_base(
            &config_path,
            LaunchRuntimeTarget::Docker,
            vec![
                (
                    "PATH".to_string(),
                    "/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
                ),
                ("HOME".to_string(), home.path().display().to_string()),
            ],
        )
        .unwrap()
        .into_parts();

        let entries = env
            .get("PATH")
            .map(|path| std::env::split_paths(path).collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(!entries.contains(&home.path().join(".bun/bin")));
    }

    #[test]
    fn host_base_env_drops_inherited_launch_scoped_env() {
        let env = hydrate_host_base_env([
            ("PATH".to_string(), "/usr/bin".to_string()),
            (GWT_BIN_PATH_ENV.to_string(), "/stale/gwtd".to_string()),
            (GWT_SESSION_ID_ENV.to_string(), "parent-session".to_string()),
            (
                GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
                "/tmp/parent.json".to_string(),
            ),
            (
                GWT_HOOK_FORWARD_TOKEN_ENV.to_string(),
                "secret-token".to_string(),
            ),
            (GWT_PROJECT_ROOT_ENV.to_string(), "/old/project".to_string()),
        ]);

        let path_entries = env
            .get("PATH")
            .map(|path| std::env::split_paths(path).collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(path_entries.contains(&PathBuf::from("/usr/bin")));
        assert!(!env.contains_key(GWT_BIN_PATH_ENV));
        assert!(!env.contains_key(GWT_SESSION_ID_ENV));
        assert!(!env.contains_key(GWT_SESSION_RUNTIME_PATH_ENV));
        assert!(!env.contains_key(GWT_HOOK_FORWARD_TOKEN_ENV));
        assert!(!env.contains_key(GWT_PROJECT_ROOT_ENV));
    }

    #[cfg(not(windows))]
    #[test]
    fn compute_hydrated_path_preserves_existing_entries() {
        let home = tempfile::tempdir().unwrap();
        let base_env = vec![
            (
                "PATH".to_string(),
                "/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
            ),
            ("HOME".to_string(), home.path().display().to_string()),
        ];

        let hydrated = compute_hydrated_path(base_env).expect("hydrated PATH");
        let entries = std::env::split_paths(&hydrated).collect::<Vec<_>>();

        assert!(entries.contains(&PathBuf::from("/usr/bin")));
        assert!(entries.contains(&PathBuf::from("/bin")));
        assert!(entries.contains(&PathBuf::from("/usr/sbin")));
        assert!(entries.contains(&PathBuf::from("/sbin")));
    }

    #[cfg(not(windows))]
    #[test]
    fn compute_hydrated_path_adds_user_bins_when_present() {
        let home = tempfile::tempdir().unwrap();
        for relative in [".bun/bin", ".local/bin", ".cargo/bin"] {
            std::fs::create_dir_all(home.path().join(relative)).unwrap();
        }
        let base_env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), home.path().display().to_string()),
        ];

        let hydrated = compute_hydrated_path(base_env).expect("hydrated PATH");
        let entries = std::env::split_paths(&hydrated).collect::<Vec<_>>();

        assert!(entries.contains(&home.path().join(".bun/bin")));
        assert!(entries.contains(&home.path().join(".local/bin")));
        assert!(entries.contains(&home.path().join(".cargo/bin")));
    }

    #[cfg(not(windows))]
    #[test]
    fn compute_hydrated_path_is_idempotent() {
        let home = tempfile::tempdir().unwrap();
        for relative in [".bun/bin", ".cargo/bin"] {
            std::fs::create_dir_all(home.path().join(relative)).unwrap();
        }
        let base_env = vec![
            ("PATH".to_string(), "/usr/bin:/bin".to_string()),
            ("HOME".to_string(), home.path().display().to_string()),
        ];

        let first = compute_hydrated_path(base_env.clone()).expect("first hydration");
        let second = compute_hydrated_path(vec![
            ("PATH".to_string(), first.clone()),
            ("HOME".to_string(), home.path().display().to_string()),
        ])
        .expect("second hydration");

        let first_entries = std::env::split_paths(&first).collect::<Vec<_>>();
        let second_entries = std::env::split_paths(&second).collect::<Vec<_>>();
        assert_eq!(first_entries, second_entries);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compute_hydrated_path_includes_macos_path_helper_paths() {
        let base_env = vec![
            (
                "PATH".to_string(),
                "/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
            ),
            ("HOME".to_string(), String::new()),
        ];

        let hydrated = compute_hydrated_path(base_env).expect("hydrated PATH");
        let entries = std::env::split_paths(&hydrated).collect::<Vec<_>>();

        assert!(
            entries.contains(&PathBuf::from("/usr/local/bin")),
            "expected /usr/local/bin in hydrated PATH; got {hydrated}"
        );
    }

    #[test]
    fn apply_host_path_hydration_to_std_env_does_not_panic() {
        let _lock = path_env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let original = std::env::var_os("PATH");
        apply_host_path_hydration_to_std_env();
        match original {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn compute_hydrated_path_returns_empty_string_when_no_paths_available_on_linux() {
        // On Linux there is no path_helper, so an env with no PATH and no
        // HOME yields an empty hydrated PATH. The guard inside
        // apply_host_path_hydration_to_std_env must skip this case so that
        // `std::env::PATH` is never blanked.
        let result = compute_hydrated_path(Vec::<(String, String)>::new());
        assert_eq!(result.as_deref(), Some(""));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn apply_host_path_hydration_to_std_env_does_not_blank_path_when_no_inputs_on_linux() {
        let _lock = path_env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let original_path = std::env::var_os("PATH");
        let original_home = std::env::var_os("HOME");
        std::env::remove_var("PATH");
        std::env::remove_var("HOME");

        apply_host_path_hydration_to_std_env();

        if let Some(value) = std::env::var_os("PATH") {
            assert!(
                !value.is_empty(),
                "apply_host_path_hydration_to_std_env must not write an empty PATH"
            );
        }

        match original_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn apply_host_path_hydration_to_std_env_preserves_existing_path_entries() {
        let _lock = path_env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let original = std::env::var_os("PATH");
        std::env::set_var("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");

        apply_host_path_hydration_to_std_env();
        let after = std::env::var("PATH").unwrap_or_default();
        let entries = std::env::split_paths(&after).collect::<Vec<_>>();
        assert!(entries.contains(&PathBuf::from("/usr/bin")));
        assert!(entries.contains(&PathBuf::from("/bin")));

        match original {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
    }

    fn path_env_test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[cfg(not(windows))]
    #[test]
    fn apply_host_path_hydration_to_std_env_emits_info_event_with_path_summary() {
        use crate::test_capture::{CaptureLayer, CapturedEvents};
        use tracing_subscriber::layer::SubscriberExt;

        let _lock = path_env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let original_path = std::env::var_os("PATH");
        std::env::set_var("PATH", "/usr/bin:/bin");

        let events = CapturedEvents::new();
        let subscriber = tracing_subscriber::registry().with(CaptureLayer::new(events.clone()));
        tracing::subscriber::with_default(subscriber, || {
            apply_host_path_hydration_to_std_env();
        });

        match original_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }

        let captured = events.snapshot();
        let info_events: Vec<_> = captured
            .iter()
            .filter(|event| event.level == tracing::Level::INFO)
            .filter(|event| event.target == "gwt::launch::startup")
            .collect();
        assert!(
            !info_events.is_empty(),
            "expected at least one INFO event with target gwt::launch::startup; captured = {:?}",
            captured
        );
        let event = info_events[0];
        assert_eq!(
            event.fields.get("stage").map(String::as_str),
            Some("path_hydration")
        );
        assert!(event.fields.contains_key("path_before"));
        assert!(event.fields.contains_key("path_entry_count_before"));
    }
}
