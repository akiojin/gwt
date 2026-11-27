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
    let environment =
        LaunchEnvironment::from_active_profile(config_path, gwt_agent::LaunchRuntimeTarget::Host)?
            .into_parts();
    latest_with_environment(descriptor, &environment, cwd)
        .map_err(|error| redact_output(&error, &environment.0))
}

/// Only routes whose metadata describes the updater's distribution/channel.
pub fn supports_updates(descriptor: &BuiltinAgentDescriptor) -> bool {
    descriptor.id == AgentId::Antigravity
        || (descriptor.id != AgentId::ClaudeCode && descriptor.distribution.npm_package().is_some())
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
    let mut installed_path = None;
    let operation = (|| -> Result<String, String> {
        let mut checked_target = None;
        let mut update_source = None;
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
                if descriptor.id == AgentId::Antigravity {
                    ("agy".into(), vec!["update".into()])
                } else if descriptor.id == AgentId::GrokBuild {
                    // Issue #7819: update through the route that owns the
                    // installed binary; npm must not overwrite another
                    // installer's binary or symlink.
                    let executable = detected.as_ref().map(|agent| agent.path.as_path());
                    let package = descriptor.distribution.npm_package().unwrap_or_default();
                    let source = executable.map_or(GrokInstallSource::Other, |path| {
                        grok_install_source(
                            path,
                            package,
                            home_directory(&environment.0).as_deref(),
                        )
                    });
                    update_source = Some(source);
                    match source {
                        GrokInstallSource::OfficialInstaller => (
                            descriptor.command.into(),
                            vec!["update".into(), "--version".into(), latest.clone()],
                        ),
                        GrokInstallSource::NpmGlobal => (
                            "npm".into(),
                            vec!["install".into(), "-g".into(), format!("{package}@{latest}")],
                        ),
                        GrokInstallSource::Other => {
                            return Err(format!(
                                "Cannot update {}: {} was not installed by npm or the Grok installer (~/.grok), so gwt will not overwrite it. Update it with the tool that installed it.",
                                descriptor.display_name,
                                executable.map_or_else(
                                    || descriptor.command.to_string(),
                                    |path| std::fs::canonicalize(path)
                                        .unwrap_or_else(|_| path.to_path_buf())
                                        .display()
                                        .to_string(),
                                ),
                            ));
                        }
                    }
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
            if let Some(source) = update_source {
                return Err(format!(
                    "{} update from {} failed. Ran `{}`. {} Next: {}",
                    descriptor.display_name,
                    source.label(),
                    std::iter::once(program.as_str())
                        .chain(args.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" "),
                    process_failure(&output),
                    source.next_step(),
                ));
            }
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
        if action == MaintenanceAction::Install {
            installed_path = Some(after.path);
        }
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
        Err(message) => {
            result.message = message;
            // Issue #7819 AC-3: a failed update reports the version that is
            // still installed instead of leaving "after" unknown.
            if action == MaintenanceAction::Update
                && result.after_version.is_none()
                && detected.is_some()
            {
                result.after_version = AgentDetector::detect_by_command_with_environment(
                    agent_id,
                    &environment.0,
                    &environment.1,
                    Some(cwd),
                )
                .and_then(|agent| agent.version);
            }
        }
    }
    result.message = redact_output(&result.message, &environment.0);
    result.before_version = result
        .before_version
        .map(|value| redact_output(&value, &environment.0));
    result.after_version = result
        .after_version
        .map(|value| redact_output(&value, &environment.0));
    if result.success {
        if let Some(path) = installed_path {
            result.message.push_str(". ");
            match register_install_directory_on_shell_path(descriptor, &path, &environment.0) {
                ShellPathRegistration::Registered { files } => result
                    .message
                    .push_str(&shell_path_registered_message(descriptor, &files, true)),
                ShellPathRegistration::AlreadyRegistered { files } => result
                    .message
                    .push_str(&shell_path_registered_message(descriptor, &files, false)),
                ShellPathRegistration::Skipped => {
                    result
                        .message
                        .push_str(&shell_path_guidance(descriptor, &path, &environment.0))
                }
            }
        }
    }
    result
}

/// Who owns an installed `grok` executable (Issue #7819).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrokInstallSource {
    /// `npm install -g` (the binary resolves into `node_modules/<package>`).
    NpmGlobal,
    /// xAI's own installer, which keeps the binary under `~/.grok`.
    OfficialInstaller,
    /// Anything else; gwt leaves it untouched.
    Other,
}

impl GrokInstallSource {
    fn label(self) -> &'static str {
        match self {
            Self::NpmGlobal => "the npm global install",
            Self::OfficialInstaller => "the Grok installer (~/.grok)",
            Self::Other => "an unrecognized installation",
        }
    }

    fn next_step(self) -> &'static str {
        match self {
            Self::NpmGlobal => "check npm's global prefix and its write permissions in the active profile, then retry.",
            Self::OfficialInstaller => "run `grok update` in a terminal to see the installer's full output, fix the reported problem, then retry.",
            Self::Other => "update it with the tool that installed it.",
        }
    }
}

fn home_directory(environment: &HashMap<String, String>) -> Option<std::path::PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|key| {
            environment
                .get(*key)
                .filter(|value| !value.is_empty())
                .cloned()
        })
        .or_else(|| gwt_core::env::var("HOME").ok())
        .map(std::path::PathBuf::from)
}

fn grok_install_source(executable: &Path, package: &str, home: Option<&Path>) -> GrokInstallSource {
    let resolved = std::fs::canonicalize(executable).unwrap_or_else(|_| executable.to_path_buf());
    let package_directory = Path::new("node_modules").join(package);
    // Unix npm links resolve into `node_modules/<package>`; Windows npm shims
    // sit next to `node_modules/<package>` in the npm prefix.
    if resolved
        .ancestors()
        .any(|ancestor| ancestor.ends_with(&package_directory))
        || executable
            .parent()
            .is_some_and(|directory| directory.join(&package_directory).is_dir())
    {
        return GrokInstallSource::NpmGlobal;
    }
    if let Some(home) = home {
        let installer_root = home.join(".grok");
        let canonical_root =
            std::fs::canonicalize(&installer_root).unwrap_or(installer_root.clone());
        if executable.starts_with(&installer_root) || resolved.starts_with(&canonical_root) {
            return GrokInstallSource::OfficialInstaller;
        }
    }
    GrokInstallSource::Other
}

fn version_check_command(descriptor: &BuiltinAgentDescriptor) -> String {
    std::iter::once(descriptor.command)
        .chain(descriptor.version_prefix_args.iter().copied())
        .chain([descriptor.version_flag])
        .collect::<Vec<_>>()
        .join(" ")
}

/// The detected installation directory, or `None` when it cannot be shown or
/// written safely because redaction would change it (it embeds a secret
/// environment value or protected data).
fn safe_install_directory(
    executable: &Path,
    environment: &HashMap<String, String>,
) -> Option<String> {
    let directory = executable.parent().unwrap_or(executable).to_string_lossy();
    // Only this detected filesystem location uses the audit classifier.
    // Installer/version output retains the stricter blanket env redaction.
    let mut safe_directory =
        gwt_core::process_console::redact_line(&gwt_core::process_console::strip_ansi(&directory));
    for (_, value) in environment
        .iter()
        .filter(|(key, value)| gwt_agent::is_secret_env_key(key) && !value.is_empty())
    {
        safe_directory = safe_directory.replace(value, gwt_core::process_console::REDACTED);
    }
    (safe_directory == directory.as_ref()).then(|| directory.into_owned())
}

/// Outcome of trying to make a freshly installed CLI runnable by name from
/// the user's own terminal (Issue #7496). Registration only ever appends to
/// the shell startup files of the shell named by `SHELL`, only for
/// home-local installs (e.g. `~/.local/bin`), and is idempotent; every other
/// situation is `Skipped` and the caller falls back to printed guidance.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ShellPathRegistration {
    Registered { files: Vec<String> },
    AlreadyRegistered { files: Vec<String> },
    Skipped,
}

/// Shell startup files (relative to the home directory) that a new terminal
/// of the given shell reads, paired with the line that puts `directory` on
/// its PATH. Returns `None` for shells gwt does not register into.
fn shell_startup_entries(shell_name: &str, directory: &str) -> Option<Vec<(String, String)>> {
    let quoted = format!("'{}'", directory.replace('\'', "'\\''"));
    match shell_name {
        "zsh" => Some(vec![(
            ".zshrc".into(),
            format!("export PATH={quoted}:\"$PATH\""),
        )]),
        "bash" => Some(vec![
            (".bashrc".into(), format!("export PATH={quoted}:\"$PATH\"")),
            (
                ".bash_profile".into(),
                format!("export PATH={quoted}:\"$PATH\""),
            ),
        ]),
        "fish" => Some(vec![(
            ".config/fish/config.fish".into(),
            format!("fish_add_path --path {quoted}"),
        )]),
        _ => None,
    }
}

fn register_install_directory_on_shell_path(
    descriptor: &BuiltinAgentDescriptor,
    executable: &Path,
    environment: &HashMap<String, String>,
) -> ShellPathRegistration {
    if cfg!(windows) {
        return ShellPathRegistration::Skipped;
    }
    let Some(directory) = safe_install_directory(executable, environment) else {
        return ShellPathRegistration::Skipped;
    };
    let Some(home) = environment
        .get("HOME")
        .filter(|value| !value.is_empty())
        .cloned()
        .or_else(|| gwt_core::env::var("HOME").ok())
    else {
        return ShellPathRegistration::Skipped;
    };
    let home_path = Path::new(&home);
    let directory_path = Path::new(&directory);
    // Only home-local installs (e.g. ~/.local/bin, ~/.cargo/bin) are missing
    // from a terminal's PATH; system locations are already reachable and
    // must not be written into personal startup files.
    if directory_path == home_path || !directory_path.starts_with(home_path) {
        return ShellPathRegistration::Skipped;
    }
    let Some(shell) = environment.get("SHELL").filter(|value| !value.is_empty()) else {
        return ShellPathRegistration::Skipped;
    };
    let shell_name = Path::new(shell)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Some(entries) = shell_startup_entries(&shell_name, &directory) else {
        return ShellPathRegistration::Skipped;
    };
    let marker = format!(
        "# Added by gwt so `{}` runs by name in new terminals",
        descriptor.command
    );
    let mut written = Vec::new();
    let mut present = Vec::new();
    for (relative, line) in entries {
        let file = home_path.join(&relative);
        let existing = std::fs::read_to_string(&file).unwrap_or_default();
        if existing.contains(&directory) {
            present.push(format!("~/{relative}"));
            continue;
        }
        if let Some(parent) = file.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return ShellPathRegistration::Skipped;
            }
        }
        let mut updated = existing;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(&marker);
        updated.push('\n');
        updated.push_str(&line);
        updated.push('\n');
        if std::fs::write(&file, updated).is_err() {
            return ShellPathRegistration::Skipped;
        }
        written.push(format!("~/{relative}"));
    }
    if written.is_empty() {
        ShellPathRegistration::AlreadyRegistered { files: present }
    } else {
        ShellPathRegistration::Registered { files: written }
    }
}

fn shell_path_registered_message(
    descriptor: &BuiltinAgentDescriptor,
    files: &[String],
    added: bool,
) -> String {
    let check = version_check_command(descriptor);
    let files = files.join(" and ");
    if added {
        format!(
            "Added the installation directory to {files} so new terminals can run `{}` by name. Open a new terminal and run `{check}` to verify.",
            descriptor.command
        )
    } else {
        format!(
            "The installation directory is already on the PATH setup in {files}, so new terminals can run `{}` by name. Open a new terminal and run `{check}` to verify.",
            descriptor.command
        )
    }
}

fn shell_path_guidance(
    descriptor: &BuiltinAgentDescriptor,
    executable: &Path,
    environment: &HashMap<String, String>,
) -> String {
    let check = version_check_command(descriptor);
    let Some(directory) = safe_install_directory(executable, environment) else {
        return format!(
            "gwt detection does not verify your terminal's PATH. The installation directory contains protected data and cannot be shown safely. Find the CLI's installation directory in your profile and add it to your terminal's PATH, then open a new terminal and run `{check}`."
        );
    };
    let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
    let setup = if cfg!(windows) {
        format!("add `{directory}` to your user Path environment variable in Windows Settings")
    } else {
        let directory = quote(&directory);
        format!(
            "add `export PATH={directory}:\"$PATH\"` to ~/.zshrc (zsh) or ~/.bashrc (bash; login shells may use ~/.bash_profile), or run `fish_add_path -- {directory}` in fish"
        )
    };
    format!(
        "gwt detection does not verify your terminal's PATH. If you get command not found, {setup}. Open a new terminal and run `{check}`."
    )
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
    // npm metadata cannot prove the native updater's effective channel, which
    // may include managed/project settings. Never mutate or select a channel.
    if descriptor.id == AgentId::ClaudeCode {
        return Err("Claude Code's effective native release channel cannot be verified. Run `claude update` manually; installation remains available.".into());
    }
    if descriptor.id == AgentId::Hermes {
        return Err("Hermes's installed update channel and commit cannot be matched to release metadata. Run `hermes update` manually.".into());
    }
    if descriptor.id == AgentId::Copilot {
        return Err("GitHub Copilot distribution cannot be verified: gh copilot may be a built-in CLI or an extension from another repository/host. Update the installed distribution manually.".into());
    }
    if descriptor.id == AgentId::Antigravity {
        let platform = antigravity_platform().ok_or_else(||
            "Antigravity update metadata is unavailable for this platform. Run `agy update` manually.".to_string())?;
        let url = format!("https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/{platform}.json");
        let args = [
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "5",
            "--max-time",
            "10",
            &url,
        ]
        .map(str::to_string);
        let output = execute("curl", &args, environment, cwd, METADATA_TIMEOUT)?;
        let metadata = native_metadata(&output)?;
        return metadata_version(metadata.get("version").and_then(serde_json::Value::as_str))
            .map(Some);
    }
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

fn antigravity_platform() -> Option<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        _ => return None,
    };
    let suffix = if cfg!(all(target_os = "linux", target_env = "musl")) {
        "_musl"
    } else {
        ""
    };
    Some(format!("{os}_{arch}{suffix}"))
}

fn native_metadata(output: &SpawnOutput) -> Result<serde_json::Value, String> {
    if !output.success() {
        return Err(format!("Cannot check the latest version: {}. Check network access and provider authentication, then retry.", process_failure(output)));
    }
    serde_json::from_str(output.stdout.trim())
        .map_err(|_| "Latest-version metadata was invalid; update was not started. Retry when the official provider is available.".into())
}

fn metadata_version(value: Option<&str>) -> Result<String, String> {
    value
        .and_then(|value| semver::Version::parse(value.strip_prefix('v').unwrap_or(value)).ok())
        .map(|version| version.to_string())
        .ok_or_else(|| {
            "Latest-version metadata was not a semantic version; update was not started.".into()
        })
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

        fn program(&self, name: &str, script: &str) {
            use std::os::unix::fs::PermissionsExt;
            let path = self.root.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn install_succeeds_when_the_cli_is_detected_without_a_version() {
        let fixture = Fixture::new(
            "printf '%s\\n' \"$*\" >> calls\nprintf '#!/bin/sh\\nexit 1\\n' > codex\n/bin/chmod +x codex",
            None,
        );
        let mut settings = gwt_config::Settings::load_from_path(&fixture.config).unwrap();
        settings.profiles.profiles[0]
            .env_vars
            .insert("HOME".into(), "/different-profile-home".into());
        settings.save(&fixture.config).unwrap();
        let result = fixture.run(MaintenanceAction::Install);
        assert!(result.success, "{}", result.message);
        assert_eq!(result.before_version, None);
        assert_eq!(result.after_version, None);
        assert!(result.message.contains("version unavailable"));
        assert!(
            result.message.contains("terminal's PATH"),
            "{}",
            result.message
        );
        assert!(result.message.contains("command not found"));
        assert!(result.message.contains("~/.zshrc"));
        assert!(result.message.contains("~/.bashrc"));
        assert!(result.message.contains("fish_add_path"));
        assert!(result.message.contains("codex --version"));
        assert!(
            result
                .message
                .contains(fixture.root.path().to_str().unwrap()),
            "{}",
            result.message
        );
        assert_eq!(fixture.calls().trim(), "install -g @openai/codex@latest");
    }

    #[cfg(unix)]
    #[test]
    fn shell_path_instructions_preserve_literal_paths_and_shell_metacharacters() {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join(".local/bin with '$() and `backticks`");
        let environment = HashMap::from([
            ("HOME".into(), home.path().display().to_string()),
            ("PATH".into(), directory.display().to_string()),
        ]);
        let descriptor = gwt_agent::builtin_agent_descriptor_for_command("claude").unwrap();
        let message = shell_path_guidance(descriptor, &directory.join("claude"), &environment);
        assert!(message.contains(home.path().to_str().unwrap()));
        assert!(!message.contains("redacted"), "{message}");
        let command = message
            .split_once('`')
            .unwrap()
            .1
            .split_once("` to ~/.zshrc")
            .unwrap()
            .0;
        let output = gwt_core::process::resolved_command(
            gwt_core::process::ProcessPlanRequest::new("/bin/sh"),
        )
        .unwrap()
        .arg("-c")
        .arg(format!("{command}; printf '%s' \"$PATH\""))
        .env_clear()
        .env("HOME", "/different-terminal-home")
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{}:/usr/bin:/bin", directory.display())
        );
    }

    #[test]
    fn shell_path_instructions_do_not_expose_protected_directory_values() {
        let secret = "private-installation-token";
        let environment = HashMap::from([("SERVICE_TOKEN".into(), secret.into())]);
        let descriptor = gwt_agent::builtin_agent_descriptor_for_command("claude").unwrap();
        let executable = std::path::PathBuf::from("/opt").join(secret).join("claude");
        let message = shell_path_guidance(descriptor, &executable, &environment);
        assert!(!message.contains(secret));
        assert!(!message.contains("export PATH="));
        assert!(message.contains("protected data"));
        assert!(message.contains("claude --version"));
    }

    #[cfg(unix)]
    #[test]
    fn native_claude_install_explains_shell_path_even_when_detection_succeeds() {
        let fixture = Fixture::new("exit 99", None);
        fixture.program(
            "curl",
            r#"/bin/cat <<'INSTALLER'
/bin/mkdir -p "$HOME/.local/bin"
printf '#!/bin/sh\nprintf "2.1.0 (Claude Code)\\n"\n' > "$HOME/.local/bin/claude"
/bin/chmod +x "$HOME/.local/bin/claude"
INSTALLER"#,
        );
        let mut settings = gwt_config::Settings::load_from_path(&fixture.config).unwrap();
        settings.profiles.profiles[0].env_vars.extend([
            ("HOME".into(), fixture.root.path().display().to_string()),
            ("SHELL".into(), "/bin/zsh".into()),
            (
                "PATH".into(),
                format!(
                    "{}:{}/.local/bin:/usr/bin:/bin",
                    fixture.root.path().display(),
                    fixture.root.path().display()
                ),
            ),
        ]);
        settings.save(&fixture.config).unwrap();
        let result = run_maintenance(
            "claude",
            MaintenanceAction::Install,
            &fixture.config,
            fixture.root.path(),
        );
        assert!(result.success, "{}", result.message);
        assert_eq!(result.after_version.as_deref(), Some("2.1.0 (Claude Code)"));
        assert!(
            result
                .message
                .contains("Added the installation directory to ~/.zshrc"),
            "{}",
            result.message
        );
        assert!(result.message.contains("claude --version"));
        assert!(!result.message.contains("redacted"));
        let zshrc = std::fs::read_to_string(fixture.root.path().join(".zshrc")).unwrap();
        assert!(
            zshrc.contains(&format!(
                "export PATH='{}':\"$PATH\"",
                fixture.root.path().join(".local/bin").display()
            )),
            "{zshrc}"
        );
        assert!(!fixture.root.path().join(".bashrc").exists());
    }

    #[cfg(unix)]
    #[test]
    fn shell_path_registration_is_idempotent_and_shell_specific() {
        let home = tempfile::tempdir().unwrap();
        let executable = home.path().join(".local/bin").join("claude");
        let descriptor = gwt_agent::builtin_agent_descriptor_for_command("claude").unwrap();
        let environment = HashMap::from([
            ("HOME".into(), home.path().display().to_string()),
            ("SHELL".into(), "/bin/bash".into()),
        ]);
        let first = register_install_directory_on_shell_path(descriptor, &executable, &environment);
        assert_eq!(
            first,
            ShellPathRegistration::Registered {
                files: vec!["~/.bashrc".into(), "~/.bash_profile".into()]
            }
        );
        let bashrc = std::fs::read_to_string(home.path().join(".bashrc")).unwrap();
        assert!(bashrc.contains(".local/bin"));
        let second =
            register_install_directory_on_shell_path(descriptor, &executable, &environment);
        assert_eq!(
            second,
            ShellPathRegistration::AlreadyRegistered {
                files: vec!["~/.bashrc".into(), "~/.bash_profile".into()]
            }
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join(".bashrc")).unwrap(),
            bashrc
        );
    }

    #[cfg(unix)]
    #[test]
    fn shell_path_registration_skips_system_locations_unknown_shells_and_secrets() {
        let home = tempfile::tempdir().unwrap();
        let descriptor = gwt_agent::builtin_agent_descriptor_for_command("claude").unwrap();
        let base = HashMap::from([
            ("HOME".into(), home.path().display().to_string()),
            ("SHELL".into(), "/bin/zsh".into()),
        ]);
        // Outside the home directory: a system location a terminal already sees.
        assert_eq!(
            register_install_directory_on_shell_path(
                descriptor,
                Path::new("/usr/local/bin/claude"),
                &base
            ),
            ShellPathRegistration::Skipped
        );
        // Unknown shell: never guess a startup file.
        let mut unknown = base.clone();
        unknown.insert("SHELL".into(), "/bin/tcsh".into());
        assert_eq!(
            register_install_directory_on_shell_path(
                descriptor,
                &home.path().join(".local/bin/claude"),
                &unknown
            ),
            ShellPathRegistration::Skipped
        );
        // A directory embedding a secret value must never be written out.
        let secret = "private-installation-token";
        let mut secret_env = base.clone();
        secret_env.insert("SERVICE_TOKEN".into(), secret.into());
        assert_eq!(
            register_install_directory_on_shell_path(
                descriptor,
                &home.path().join(secret).join("claude"),
                &secret_env
            ),
            ShellPathRegistration::Skipped
        );
        assert!(!home.path().join(".zshrc").exists());
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
        assert!(
            latest_version("hermes", &fixture.config, fixture.root.path())
                .unwrap_err()
                .contains("channel")
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_metadata_uses_official_platform_manifest() {
        let fixture = Fixture::new("exit 99", None);
        fixture.program(
            "curl",
            "printf '%s\\n' \"$*\" >> calls\nprintf '{\"version\":\"1.3.5\"}\\n'",
        );
        assert_eq!(
            latest_version("agy", &fixture.config, fixture.root.path())
                .unwrap()
                .as_deref(),
            Some("1.3.5")
        );
        let calls = fixture.calls();
        assert!(
            calls.contains("/manifests/darwin_arm64.json")
                || calls.contains("/manifests/darwin_amd64.json")
                || calls.contains("/manifests/linux_"),
            "{calls}"
        );
        assert!(calls.contains("--max-time 10"));
    }

    #[cfg(unix)]
    #[test]
    fn native_metadata_failure_or_invalid_target_refuses_before_update() {
        for (agent, program, body) in [
            ("agy", "curl", "printf 'offline\\n' >&2; exit 7"),
            ("agy", "curl", "printf '{\"version\":\"unknown\"}'"),
        ] {
            let fixture = Fixture::new("exit 99", None);
            let script = format!("printf '%s\\n' \"$*\" >> calls\nif [ \"$1\" = --version ]; then printf '1.0.0\\n'; else {body}; fi");
            fixture.program(agent, "printf '%s\\n' \"$*\" >> calls\nprintf '1.0.0\\n'");
            fixture.program(program, &script);
            let result = run_maintenance(
                agent,
                MaintenanceAction::Update,
                &fixture.config,
                fixture.root.path(),
            );
            assert!(!result.success, "{agent}: {}", result.message);
            assert!(
                !fixture
                    .calls()
                    .lines()
                    .any(|line| line == "update" || line.starts_with("extension")),
                "{}",
                fixture.calls()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn native_updates_apply_to_the_checked_distribution_and_refresh_version() {
        let fixture = Fixture::new("exit 99", None);
        fixture.program("curl", "printf '{\"version\":\"1.2.0\"}\\n'");
        fixture.program("agy", "printf '%s\\n' \"$*\" >> calls\nif [ \"$1\" = update ]; then : > updated; elif [ -f updated ]; then printf '1.2.0\\n'; else printf '1.0.0\\n'; fi");
        let result = run_maintenance(
            "agy",
            MaintenanceAction::Update,
            &fixture.config,
            fixture.root.path(),
        );
        assert!(result.success, "{}", result.message);
        assert_eq!(result.before_version.as_deref(), Some("1.0.0"));
        assert_eq!(result.after_version.as_deref(), Some("1.2.0"));
        assert!(fixture.calls().lines().any(|line| line == "update"));
    }

    #[cfg(unix)]
    #[test]
    fn unverified_native_routes_refuse_metadata_and_mutation() {
        for (agent, channel) in [
            ("claude", "stable"),
            ("claude", "latest"),
            ("hermes", "stable"),
            ("gh", "enterprise.example.invalid"),
        ] {
            let fixture = Fixture::new("printf 'npm mutation\\n' >> calls", None);
            fixture.program(agent, "if [ \"$1\" = --version ] || [ \"$2\" = --version ]; then printf '1.0.0\\n'; else printf 'native mutation\\n' >> calls; fi");
            let config_dir = fixture.root.path().join("native-config");
            std::fs::create_dir(&config_dir).unwrap();
            std::fs::write(
                config_dir.join("settings.json"),
                format!("{{\"autoUpdatesChannel\":\"{channel}\"}}"),
            )
            .unwrap();
            let mut settings = gwt_config::Settings::load_from_path(&fixture.config).unwrap();
            settings.profiles.profiles[0].env_vars.insert(
                "CLAUDE_CONFIG_DIR".into(),
                config_dir.to_string_lossy().into_owned(),
            );
            settings.profiles.profiles[0]
                .env_vars
                .insert("GH_HOST".into(), channel.into());
            settings.save(&fixture.config).unwrap();
            let reason = if agent == "gh" {
                "distribution"
            } else {
                "channel"
            };
            assert!(latest_version(agent, &fixture.config, fixture.root.path())
                .unwrap_err()
                .contains(reason));
            for _ in 0..2 {
                let result = run_maintenance(
                    agent,
                    MaintenanceAction::Update,
                    &fixture.config,
                    fixture.root.path(),
                );
                assert!(!result.success, "{agent}/{channel}: {}", result.message);
                assert!(result.message.contains(reason), "{}", result.message);
            }
            assert!(fixture.calls().is_empty(), "{}", fixture.calls());
        }
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

    /// Issue #7819: a Grok build at `<home>/<layout>` reached through
    /// `<home>/bin/grok`, the way Homebrew's prefix links the official
    /// installer's binary. npm and the Grok binary log every call to `calls`.
    #[cfg(unix)]
    fn grok_fixture(layout: &str, grok_script: &str) -> Fixture {
        let fixture = Fixture::new(
            "printf 'npm %s\\n' \"$*\" >> \"$HOME/calls\"\nif [ \"$1\" = view ]; then printf '\"1.0.92\"\\n'; fi",
            None,
        );
        let home = fixture.root.path();
        let binary = home.join(layout);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, format!("#!/bin/sh\n{grok_script}\n")).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::os::unix::fs::symlink(&binary, home.join("bin/grok")).unwrap();
        let mut settings = gwt_config::Settings::load_from_path(&fixture.config).unwrap();
        settings.profiles.profiles[0].env_vars.extend([
            ("HOME".into(), home.display().to_string()),
            (
                "PATH".into(),
                format!(
                    "{}:{}:/usr/bin:/bin",
                    home.join("bin").display(),
                    home.display()
                ),
            ),
        ]);
        settings.save(&fixture.config).unwrap();
        fixture
    }

    #[cfg(unix)]
    fn run_grok_update(fixture: &Fixture) -> MaintenanceResult {
        run_maintenance(
            "grok",
            MaintenanceAction::Update,
            &fixture.config,
            fixture.root.path(),
        )
    }

    // Issue #7819 AC-1 / AC-2 / AC-4: the official installer's binary, even
    // through a Homebrew-prefix symlink, updates with `grok update`, never npm.
    #[cfg(unix)]
    #[test]
    fn grok_from_the_official_installer_updates_with_grok_update_not_npm() {
        let fixture = grok_fixture(
            ".grok/downloads/grok-macos-aarch64",
            r#"if [ "$1" = update ]; then
  printf 'grok %s\n' "$*" >> "$HOME/calls"
  printf '#!/bin/sh\nprintf "grok 1.0.92 (abc)\\n"\n' > "$0"
  exit 0
fi
printf 'grok 1.0.77 (50ca579fa996)\n'"#,
        );
        let result = run_grok_update(&fixture);
        assert!(result.success, "{}", result.message);
        assert_eq!(
            result.before_version.as_deref(),
            Some("grok 1.0.77 (50ca579fa996)")
        );
        assert_eq!(result.after_version.as_deref(), Some("grok 1.0.92 (abc)"));
        let calls = fixture.calls();
        assert!(calls.contains("grok update --version 1.0.92"), "{calls}");
        assert!(!calls.contains("npm install"), "{calls}");
    }

    // Issue #7819 AC-1: an npm global install keeps the npm route.
    #[cfg(unix)]
    #[test]
    fn grok_from_npm_global_updates_with_npm() {
        let fixture = grok_fixture(
            "lib/node_modules/@xai-official/grok/bin/grok",
            "printf 'grok 1.0.77\\n'",
        );
        let result = run_grok_update(&fixture);
        assert!(!result.success, "fixture npm never upgrades the binary");
        let calls = fixture.calls();
        assert!(
            calls.contains("npm install -g @xai-official/grok@1.0.92"),
            "{calls}"
        );
        assert!(!calls.contains("grok update"), "{calls}");
    }

    // Issue #7819 AC-2: an unrecognised installation is never overwritten.
    #[cfg(unix)]
    #[test]
    fn grok_from_an_unknown_source_is_left_untouched() {
        let fixture = grok_fixture("opt/grok/grok", "printf 'grok 1.0.77\\n'");
        let result = run_grok_update(&fixture);
        assert!(!result.success);
        let calls = fixture.calls();
        assert!(!calls.contains("npm install"), "{calls}");
        assert!(!calls.contains("grok update"), "{calls}");
        assert!(
            result.message.contains("opt/grok/grok"),
            "{}",
            result.message
        );
        assert!(
            result.message.contains("tool that installed it"),
            "{}",
            result.message
        );
    }

    // Issue #7819 AC-3: a failed update names the source, the command and the
    // next step, and still reports the version that remains installed.
    #[cfg(unix)]
    #[test]
    fn grok_update_failure_reports_source_command_and_unchanged_version() {
        let fixture = grok_fixture(
            ".grok/bin/grok",
            r#"if [ "$1" = update ]; then printf 'download failed\n' >&2; exit 3; fi
printf 'grok 1.0.77 (50ca579fa996)\n'"#,
        );
        let result = run_grok_update(&fixture);
        assert!(!result.success);
        assert!(
            result.message.contains("Grok installer"),
            "{}",
            result.message
        );
        assert!(
            result.message.contains("`grok update --version 1.0.92`"),
            "{}",
            result.message
        );
        assert!(
            result.message.contains("download failed"),
            "{}",
            result.message
        );
        assert!(result.message.contains("retry"), "{}", result.message);
        assert_eq!(
            result.after_version.as_deref(),
            Some("grok 1.0.77 (50ca579fa996)")
        );
    }
}
