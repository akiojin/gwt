//! Launch environment composition for host and container agent processes.

use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

use crate::types::LaunchRuntimeTarget;

const GWT_PROJECT_ROOT_ENV: &str = "GWT_PROJECT_ROOT";

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
        ensure_terminal_env(&mut env);
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
            LaunchRuntimeTarget::Host => Self::from_active_profile_with_base(
                config_path,
                std::env::vars().collect::<Vec<_>>(),
            ),
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
        ensure_terminal_env(&mut base_env);

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
            LaunchRuntimeTarget::Host => Self::from_active_profile_with_base(config_path, base_env),
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

fn ensure_terminal_env(env: &mut HashMap<String, String>) {
    env.entry("TERM".to_string())
        .or_insert_with(|| "xterm-256color".to_string());
    env.entry("COLORTERM".to_string())
        .or_insert_with(|| "truecolor".to_string());
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
}
