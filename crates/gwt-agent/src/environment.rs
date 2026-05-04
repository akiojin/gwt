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
    remove_inherited_launch_env(&mut env);
    hydrate_host_path(&mut env);
    env
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
        apply_required_terminal_defaults(&mut env);
        Self {
            base_env: env,
            profile_env: HashMap::new(),
            remove_env: Vec::new(),
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

        let remove_env = normalized_remove_env(&profile.disabled_env);
        let mut base_env: HashMap<String, String> = base_env.into_iter().collect();
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
    pub fn with_project_root(mut self, project_root: impl AsRef<Path>) -> Self {
        self.override_env.insert(
            GWT_PROJECT_ROOT_ENV.to_string(),
            project_root.as_ref().display().to_string(),
        );
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

fn remove_inherited_launch_env(env: &mut HashMap<String, String>) {
    for key in INHERITED_LAUNCH_ENV_KEYS {
        env.remove(*key);
    }
}

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
    let mut command = std::process::Command::new("/usr/libexec/path_helper");
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
        assert_eq!(remove_env, vec!["SECRET".to_string()]);
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
        assert_eq!(remove_env, vec!["SECRET".to_string()]);
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
            vec!["EXPLICIT_REMOVE".to_string(), "SECRET".to_string()]
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
        assert_eq!(remove_env, vec!["PATH".to_string(), "SECRET".to_string()]);
    }

    #[test]
    fn from_base_env_installs_terminal_defaults() {
        let base_env = vec![("PATH".to_string(), "/usr/bin".to_string())];

        let (env, remove_env) = LaunchEnvironment::from_base_env(base_env).into_parts();

        assert_eq!(env.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert!(remove_env.is_empty());
    }

    #[test]
    fn from_base_env_replaces_dumb_terminal_type() {
        let base_env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("TERM".to_string(), "dumb".to_string()),
            ("COLORTERM".to_string(), String::new()),
        ];

        let (env, _) = LaunchEnvironment::from_base_env(base_env).into_parts();

        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
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
}
