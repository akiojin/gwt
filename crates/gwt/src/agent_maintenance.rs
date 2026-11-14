//! Host-side installation and updates for the supported agent catalog.

use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};

use gwt_agent::{
    AgentDetector, AgentId, BuiltinAgentDescriptor, DistributionRoute, LaunchEnvironment,
};
use gwt_core::process_console::{
    spawn_logged_blocking_with_deadline, ProcessKind, SpawnOptions, SpawnOutput,
};
use serde::{Deserialize, Serialize};

const METADATA_TIMEOUT: Duration = Duration::from_secs(15);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceAction {
    Install,
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceResult {
    pub agent_id: String,
    pub action: MaintenanceAction,
    pub before_version: Option<String>,
    pub after_version: Option<String>,
    pub success: bool,
    pub message: String,
}

pub fn latest_version(
    agent_id: &str,
    config_path: &Path,
    cwd: &Path,
) -> Result<Option<String>, String> {
    let descriptor = gwt_agent::builtin_agent_descriptor_for_command(agent_id)
        .ok_or_else(|| "Choose an agent from the supported catalog".to_string())?;
    if descriptor.distribution.npm_package().is_none() {
        return Ok(None);
    }
    let environment =
        LaunchEnvironment::from_active_profile(config_path, gwt_agent::LaunchRuntimeTarget::Host)?
            .into_parts();
    latest_with_environment(descriptor, &environment, cwd)
}

/// Execute only catalog-owned commands. The runtime owns admission against live agents.
pub fn run_maintenance(
    agent_id: &str,
    action: MaintenanceAction,
    config_path: &Path,
    cwd: &Path,
) -> MaintenanceResult {
    let mut result = MaintenanceResult {
        agent_id: agent_id.to_string(),
        action,
        before_version: None,
        after_version: None,
        success: false,
        message: String::new(),
    };
    let Some(descriptor) = gwt_agent::builtin_agent_descriptor_for_command(agent_id) else {
        result.message = "Choose an agent from the supported catalog".into();
        return result;
    };
    let environment = match LaunchEnvironment::from_active_profile(
        config_path,
        gwt_agent::LaunchRuntimeTarget::Host,
    ) {
        Ok(environment) => environment.into_parts(),
        Err(error) => {
            result.message = format!("Cannot load the active Host profile: {error}");
            return result;
        }
    };
    let detected = AgentDetector::detect_by_command_with_environment(
        agent_id,
        &environment.0,
        &environment.1,
        Some(cwd),
    );
    result.before_version = detected.as_ref().and_then(|agent| agent.version.clone());
    let operation = (|| -> Result<String, String> {
        let mut checked_target = None;
        let (program, args) = match action {
            MaintenanceAction::Install => install_command(descriptor).ok_or_else(|| {
                "No installer is available; install this CLI manually".to_string()
            })?,
            MaintenanceAction::Update => {
                if detected.is_none() {
                    return Err(format!(
                        "{} is not installed. Install it first.",
                        descriptor.display_name
                    ));
                }
                let current = result
                    .before_version
                    .as_deref()
                    .and_then(parse_version)
                    .ok_or_else(|| format!(
                        "Cannot determine the current {} version. Check `{} --version` and PATH before updating.",
                        descriptor.display_name, descriptor.command
                    ))?;
                let latest = latest_with_environment(descriptor, &environment, cwd)?
                    .ok_or_else(|| format!(
                        "Latest-version metadata is unavailable for {}. Update it manually using its official installation instructions.",
                        descriptor.display_name
                    ))?;
                let latest_semver = parse_version(&latest).ok_or_else(|| {
                    "Latest-version metadata is not a semantic version".to_string()
                })?;
                if !latest_semver.cmp_precedence(&current).is_gt() {
                    result.after_version = result.before_version.clone();
                    return Ok(format!(
                        "Latest · {} · installed {} (latest available {})",
                        descriptor.display_name,
                        result
                            .before_version
                            .as_deref()
                            .unwrap_or("version unavailable"),
                        latest,
                    ));
                }
                checked_target = Some(latest_semver);
                if descriptor.id == AgentId::ClaudeCode {
                    ("claude".into(), vec!["update".into()])
                } else {
                    let package = descriptor
                        .distribution
                        .npm_package()
                        .ok_or_else(|| "No supported updater is available".to_string())?;
                    (
                        "npm".into(),
                        vec!["install".into(), "-g".into(), format!("{package}@{latest}")],
                    )
                }
            }
        };
        let output = execute(&program, &args, &environment, cwd, INSTALL_TIMEOUT)?;
        if !output.success() {
            return Err(format!(
                "{} Check installer prerequisites and the active profile's permissions/PATH, then retry.",
                process_failure(&output)
            ));
        }
        let after = AgentDetector::detect_by_command_with_environment(
            agent_id,
            &environment.0,
            &environment.1,
            Some(cwd),
        ).ok_or_else(|| format!(
            "The installer finished, but {} is not detected. Add `{}` to the active profile's PATH and restart gwt, then try again.",
            descriptor.display_name, descriptor.command
        ))?;
        result.after_version = after.version;
        if let Some(target) = checked_target {
            let actual = result
                .after_version
                .as_deref()
                .and_then(parse_version)
                .ok_or_else(|| format!(
                    "Cannot verify the updated {} version. Check `{} --version`, the active profile's PATH, and npm global prefix for npm installs, then retry.",
                    descriptor.display_name, descriptor.command
                ))?;
            if actual.cmp_precedence(&target).is_lt() {
                return Err(format!(
                    "The updater finished, but {} reports {} below checked version {}. Check the active profile's PATH and the updater's installation location (npm global prefix for npm installs), then retry.",
                    descriptor.display_name, actual, target
                ));
            }
        }
        Ok(match action {
            MaintenanceAction::Install => format!(
                "Installed {} · {}",
                descriptor.display_name,
                result
                    .after_version
                    .as_deref()
                    .unwrap_or("version unavailable"),
            ),
            MaintenanceAction::Update => format!(
                "Update finished · {} · {} → {}",
                descriptor.display_name,
                result
                    .before_version
                    .as_deref()
                    .unwrap_or("version unavailable"),
                result
                    .after_version
                    .as_deref()
                    .unwrap_or("version unavailable"),
            ),
        })
    })();
    match operation {
        Ok(message) => {
            result.success = true;
            result.message = message;
        }
        Err(message) => result.message = message,
    }
    result.message = redact_output(&result.message, &environment.0);
    result.before_version = result
        .before_version
        .map(|value| redact_output(&value, &environment.0));
    result.after_version = result
        .after_version
        .map(|value| redact_output(&value, &environment.0));
    result
}

fn install_command(descriptor: &BuiltinAgentDescriptor) -> Option<(String, Vec<String>)> {
    if descriptor.id == AgentId::ClaudeCode {
        let installer = if cfg!(windows) {
            r#"powershell -NoProfile -Command "irm https://claude.ai/install.ps1 | iex""#
        } else {
            "curl -fsSL https://claude.ai/install.sh | bash"
        };
        return Some(shell_command(installer.to_string()));
    }
    match descriptor.distribution {
        DistributionRoute::Npm { package } => Some((
            "npm".into(),
            vec!["install".into(), "-g".into(), format!("{package}@latest")],
        )),
        DistributionRoute::GhExtension { repository } => Some((
            "gh".into(),
            vec!["extension".into(), "install".into(), repository.into()],
        )),
        distribution => distribution.install_shell_command().map(shell_command),
    }
}

fn shell_command(command: String) -> (String, Vec<String>) {
    let (program, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    (program.into(), vec![flag.into(), command])
}

/// Compare vendor CLI version output with registry metadata, including prereleases.
pub fn update_available(installed: &str, latest: &str) -> bool {
    match (parse_version(installed), parse_version(latest)) {
        (Some(installed), Some(latest)) => latest.cmp_precedence(&installed).is_gt(),
        _ => false,
    }
}

/// Unknown versions never count as current; build metadata does not change precedence.
pub fn version_is_current(installed: &str, latest: &str) -> bool {
    match (parse_version(installed), parse_version(latest)) {
        (Some(installed), Some(latest)) => !latest.cmp_precedence(&installed).is_gt(),
        _ => false,
    }
}

fn parse_version(value: &str) -> Option<semver::Version> {
    value.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '.' | '-' | '+')
        });
        semver::Version::parse(token.strip_prefix('v').unwrap_or(token)).ok()
    })
}

fn latest_with_environment(
    descriptor: &BuiltinAgentDescriptor,
    environment: &(HashMap<String, String>, Vec<String>),
    cwd: &Path,
) -> Result<Option<String>, String> {
    let Some(package) = descriptor.distribution.npm_package() else {
        return Ok(None);
    };
    let args = [
        "view",
        package,
        "version",
        "--json",
        "--fetch-timeout=5000",
        "--fetch-retries=0",
    ]
    .map(str::to_string);
    let output = execute("npm", &args, environment, cwd, METADATA_TIMEOUT)?;
    if !output.success() {
        return Err(redact_output(
            &format!(
                "Cannot check the latest version: {}",
                process_failure(&output)
            ),
            &environment.0,
        ));
    }
    let version: String = serde_json::from_str(output.stdout.trim()).map_err(|_| {
        "Latest-version metadata was invalid; try again when npm registry access is available"
            .to_string()
    })?;
    let version = semver::Version::parse(version.trim()).map_err(|_| {
        "Latest-version metadata was not a semantic version; update was not started".to_string()
    })?;
    Ok(Some(version.to_string()))
}

fn execute(
    program: &str,
    args: &[String],
    environment: &(HashMap<String, String>, Vec<String>),
    cwd: &Path,
    timeout: Duration,
) -> Result<SpawnOutput, String> {
    let mut options = SpawnOptions::new(format!("agent maintenance: {program}"))
        .current_dir(cwd)
        .inherit_env(false)
        .forward_output(false);
    for (key, value) in &environment.0 {
        options = options.env(key, value);
    }
    for key in &environment.1 {
        options = options.env_remove(key);
    }
    spawn_logged_blocking_with_deadline(
        &gwt_core::process_console::global(),
        ProcessKind::AgentBootstrap,
        program,
        args,
        options,
        Instant::now() + timeout,
    )
    .map_err(|error| {
        redact_output(
            &format!(
                "Cannot run {program}: {error}. Check the active profile's PATH and try again."
            ),
            &environment.0,
        )
    })
}

fn process_failure(output: &SpawnOutput) -> String {
    let detail = if output.stderr.trim().is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    format!(
        "Command exited with status {}: {}",
        output
            .exit_code
            .map_or_else(|| "unknown".into(), |code| code.to_string()),
        detail.trim()
    )
}

fn redact_output(value: &str, environment: &HashMap<String, String>) -> String {
    let stripped = gwt_core::process_console::strip_ansi(value);
    let mut redacted = gwt_core::process_console::redact_line(&stripped);
    let mut values = environment
        .values()
        .filter(|value| value.chars().count() >= 8)
        .collect::<Vec<_>>();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for value in values {
        redacted = redacted.replace(value, gwt_core::process_console::REDACTED);
    }
    redacted.chars().take(1600).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_commands_come_from_the_supported_catalog() {
        for command in ["codex", "grok", "opencode", "openclaw"] {
            let descriptor = gwt_agent::builtin_agent_descriptor_for_command(command).unwrap();
            assert_eq!(
                install_command(descriptor),
                Some((
                    "npm".into(),
                    vec![
                        "install".into(),
                        "-g".into(),
                        format!("{}@latest", descriptor.distribution.npm_package().unwrap()),
                    ],
                ))
            );
        }
        let copilot = gwt_agent::builtin_agent_descriptor_for_command("gh").unwrap();
        assert_eq!(
            install_command(copilot),
            Some((
                "gh".into(),
                vec![
                    "extension".into(),
                    "install".into(),
                    "github/gh-copilot".into()
                ],
            ))
        );
        for command in ["agy", "hermes"] {
            let descriptor = gwt_agent::builtin_agent_descriptor_for_command(command).unwrap();
            let (_, args) = install_command(descriptor).unwrap();
            assert_eq!(
                args.last(),
                descriptor.distribution.install_shell_command().as_ref()
            );
        }
        let claude = gwt_agent::builtin_agent_descriptor_for_command("claude").unwrap();
        let (_, args) = install_command(claude).unwrap();
        assert!(args.last().unwrap().contains("https://claude.ai/install."));
        assert!(!args.last().unwrap().contains("npm"));
    }

    #[test]
    fn update_comparison_accepts_cli_prefixes_and_semver_prereleases() {
        assert!(update_available("codex-cli 1.2.3", "1.2.4"));
        assert!(update_available("v1.2.3-beta.1 (Claude Code)", "1.2.3"));
        assert!(!update_available("1.2.3", "1.2.3-beta.1"));
        assert!(!update_available("unknown", "1.2.3"));
        assert!(version_is_current("grok 2.0.0", "2.0.0"));
        assert!(version_is_current("codex-cli 2.0.1", "2.0.0"));
        assert!(!version_is_current("unknown", "2.0.0"));
    }

    #[cfg(unix)]
    struct Fixture {
        root: tempfile::TempDir,
        config: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl Fixture {
        fn new(npm_script: &str, installed: Option<&str>) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let root = tempfile::tempdir().unwrap();
            let config = root.path().join("config.toml");
            let mut settings = gwt_config::Settings::default();
            settings.profiles.profiles[0]
                .env_vars
                .insert("PATH".into(), root.path().to_string_lossy().into_owned());
            settings.save(&config).unwrap();
            let npm = root.path().join("npm");
            std::fs::write(&npm, format!("#!/bin/sh\n{npm_script}\n")).unwrap();
            std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
            if let Some(script) = installed {
                let codex = root.path().join("codex");
                std::fs::write(&codex, format!("#!/bin/sh\n{script}\n")).unwrap();
                std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            Self { root, config }
        }

        fn run(&self, action: MaintenanceAction) -> MaintenanceResult {
            run_maintenance("codex", action, &self.config, self.root.path())
        }

        fn calls(&self) -> String {
            std::fs::read_to_string(self.root.path().join("calls")).unwrap_or_default()
        }
    }

    #[cfg(unix)]
    #[test]
    fn install_succeeds_when_the_cli_is_detected_without_a_version() {
        let fixture = Fixture::new(
            "printf '%s\\n' \"$*\" >> calls\nprintf '#!/bin/sh\\nexit 1\\n' > codex\n/bin/chmod +x codex",
            None,
        );
        let result = fixture.run(MaintenanceAction::Install);
        assert!(result.success, "{}", result.message);
        assert_eq!(result.before_version, None);
        assert_eq!(result.after_version, None);
        assert!(result.message.contains("version unavailable"));
        assert_eq!(fixture.calls().trim(), "install -g @openai/codex@latest");
    }

    #[cfg(unix)]
    #[test]
    fn installer_failure_reports_its_exit_status() {
        let fixture = Fixture::new("printf 'permission denied\\n' >&2\nexit 7", None);
        let result = fixture.run(MaintenanceAction::Install);
        assert!(!result.success);
        assert!(result.message.contains("7"));
        assert!(result.message.contains("permission denied"));
    }

    #[cfg(unix)]
    #[test]
    fn profile_environment_reaches_the_installer_and_secrets_are_redacted() {
        let fixture = Fixture::new("printf '%s' \"$SERVICE_TOKEN\" >&2\nexit 1", None);
        let mut settings = gwt_config::Settings::load_from_path(&fixture.config).unwrap();
        settings.profiles.profiles[0]
            .env_vars
            .insert("SERVICE_TOKEN".into(), "private-maintenance-token".into());
        settings.save(&fixture.config).unwrap();
        let result = fixture.run(MaintenanceAction::Install);
        assert!(!result.success);
        assert!(result.message.contains("redacted"), "{}", result.message);
        assert!(!result.message.contains("private-maintenance-token"));
    }

    #[cfg(unix)]
    #[test]
    fn successful_installer_without_a_detected_cli_explains_path_recovery() {
        let fixture = Fixture::new("exit 0", None);
        let result = fixture.run(MaintenanceAction::Install);
        assert!(!result.success);
        assert!(result.message.contains("PATH"));
        assert!(result.message.contains("restart"));
    }

    #[cfg(unix)]
    #[test]
    fn latest_metadata_uses_the_active_profile_and_bounded_npm_request() {
        let fixture = Fixture::new(
            "printf '%s\\n' \"$*\" >> calls\nprintf '\"1.2.3\"\\n'",
            None,
        );
        assert_eq!(
            latest_version("codex", &fixture.config, fixture.root.path()).unwrap(),
            Some("1.2.3".into())
        );
        assert_eq!(
            fixture.calls().trim(),
            "view @openai/codex version --json --fetch-timeout=5000 --fetch-retries=0"
        );
        assert_eq!(
            latest_version("hermes", &fixture.config, fixture.root.path()).unwrap(),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn update_already_latest_or_newer_never_runs_an_installer() {
        for latest in ["1.2.3", "1.2.2"] {
            let fixture = Fixture::new(
                &format!("printf '%s\\n' \"$*\" >> calls\nprintf '\"{latest}\"\\n'"),
                Some("printf 'codex-cli 1.2.3\\n'"),
            );
            let result = fixture.run(MaintenanceAction::Update);
            assert!(result.success, "{}", result.message);
            assert!(result.message.contains("Latest"));
            assert_eq!(result.before_version.as_deref(), Some("codex-cli 1.2.3"));
            assert_eq!(result.after_version, result.before_version);
            assert!(!fixture.calls().contains("install"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn update_installs_the_checked_exact_version_and_reports_fresh_detection() {
        let fixture = Fixture::new(
            "printf '%s\\n' \"$*\" >> calls\nif [ \"$1\" = view ]; then\n  printf '\"1.2.4\"\\n'\nelse\n  printf '#!/bin/sh\\nprintf \"codex-cli 1.2.4\\\\n\"\\n' > codex\nfi",
            Some("printf 'codex-cli 1.2.3\\n'"),
        );
        let result = fixture.run(MaintenanceAction::Update);
        assert!(result.success, "{}", result.message);
        assert_eq!(result.before_version.as_deref(), Some("codex-cli 1.2.3"));
        assert_eq!(result.after_version.as_deref(), Some("codex-cli 1.2.4"));
        assert!(fixture.calls().contains("install -g @openai/codex@1.2.4"));
    }

    #[cfg(unix)]
    #[test]
    fn updater_exit_success_does_not_hide_an_older_binary_on_the_active_path() {
        let fixture = Fixture::new(
            "if [ \"$1\" = view ]; then printf '\"1.2.4\"\\n'; fi",
            Some("printf 'codex-cli 1.2.3\\n'"),
        );
        let result = fixture.run(MaintenanceAction::Update);
        assert!(!result.success, "{}", result.message);
        assert_eq!(result.before_version.as_deref(), Some("codex-cli 1.2.3"));
        assert_eq!(result.after_version.as_deref(), Some("codex-cli 1.2.3"));
        assert!(result.message.contains("PATH"));
        assert!(result.message.contains("npm global prefix"));
    }

    #[cfg(unix)]
    #[test]
    fn updater_exit_success_requires_a_verifiable_installed_version() {
        let fixture = Fixture::new(
            "if [ \"$1\" = view ]; then printf '\"1.2.4\"\\n'; else printf '#!/bin/sh\\nexit 1\\n' > codex; fi",
            Some("printf 'codex-cli 1.2.3\\n'"),
        );
        let result = fixture.run(MaintenanceAction::Update);
        assert!(!result.success, "{}", result.message);
        assert_eq!(result.before_version.as_deref(), Some("codex-cli 1.2.3"));
        assert_eq!(result.after_version, None);
        assert!(result.message.contains("Cannot verify"));
        assert!(result.message.contains("PATH"));
    }

    #[cfg(unix)]
    #[test]
    fn update_with_an_unknown_current_version_fails_closed() {
        let fixture = Fixture::new("printf '%s\\n' \"$*\" >> calls", Some("exit 1"));
        let result = fixture.run(MaintenanceAction::Update);
        assert!(!result.success);
        assert!(result.message.contains("version"));
        assert!(!fixture.calls().contains("install"));
    }
}
