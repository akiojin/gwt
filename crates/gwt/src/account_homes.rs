//! Account credential homes and positive sign-in evidence (Issue #7840).
//!
//! macOS research for the first account-pool slice:
//!
//! - R1: Claude Code 2.1.331 `auth status --json` reported a signed-in default
//!   and a signed-out empty `CLAUDE_CONFIG_DIR`. Explicitly setting
//!   `CLAUDE_CONFIG_DIR=~/.claude` also changes the config lookup on this build,
//!   so the default probe leaves it unset. The CLI can use its Keychain entry
//!   even without `.credentials.json`.
//! - R2: Codex 0.171.0 app-server resumed a synthetic conversation with a
//!   separate `CODEX_HOME` and a shared `sessions/` link. SQLite stayed local to
//!   that home.
//! - R3: Codex's config write preserved the `config.toml` link. gwt's atomic
//!   writes need the approved link target for both the config lock and write.
//! - R4: gh 2.103.3 found the same credential with the default and an empty
//!   `GH_CONFIG_DIR`. Copilot CLI prefers its own saved sign-in to gh fallback.
//!   The default probe asks the Copilot CLI itself for its authenticated login.
//!   An isolated account uses its own gwt Keychain item and the CLI's highest
//!   precedence token environment variable. Grok Build 1.0.92 uses `auth.json`
//!   in its home and supports `grok login --device-auth`. In a temporary
//!   `GROK_HOME`, `grok inspect --json` found that home's `config.toml`.
//! - R5: registration previously took the union of installed tools and project
//!   launch tools. That excluded a signed-in Grok absent from both inventories.

use std::{
    collections::HashMap,
    fs, io,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use gwt_agent::{resolve_agent_id, AgentId};
use gwt_core::process_console::{
    self, spawn_logged_blocking_with_deadline, ProcessKind, SpawnOptions,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub const COPILOT_TOKEN_ENV: &str = "COPILOT_GITHUB_TOKEN";
/// Account name is gwt's pool account ID. This namespace is separate from
/// the provider's own credentials and is also used by the enrollment slice.
pub const COPILOT_KEYCHAIN_SERVICE: &str = "gwt.accounts.copilot";
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const COPILOT_ADOPTION_OVERRIDES: &[&str] = &[
    COPILOT_TOKEN_ENV,
    "COPILOT_GH_HOST",
    "COPILOT_HOME",
    "COPILOT_PROVIDER_API_KEY",
    "COPILOT_PROVIDER_API_KEY_COMMAND",
    "COPILOT_PROVIDER_BASE_URL",
    "COPILOT_PROVIDERS_CONFIG",
    "GH_CONFIG_DIR",
    "GH_ENTERPRISE_TOKEN",
    "GH_HOST",
    "GH_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "GITHUB_TOKEN",
    "XDG_CONFIG_HOME",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    pub identity: Option<String>,
}

pub fn home_env_key(tool: &str) -> Option<&'static str> {
    match resolve_agent_id(tool)? {
        AgentId::ClaudeCode => Some("CLAUDE_CONFIG_DIR"),
        AgentId::Codex => Some("CODEX_HOME"),
        AgentId::GrokBuild => Some("GROK_HOME"),
        AgentId::Copilot => Some("GH_CONFIG_DIR"),
        _ => None,
    }
}

/// `None` for the default account leaves the CLI's environment intact.
pub fn account_launch_env(tool: &str, home: Option<&Path>) -> Option<(&'static str, String)> {
    Some((home_env_key(tool)?, home?.to_str()?.to_owned()))
}

fn local_home_name(agent: &AgentId) -> Option<&'static str> {
    match agent {
        AgentId::ClaudeCode => Some(".claude"),
        AgentId::Codex => Some(".codex"),
        AgentId::GrokBuild => Some(".grok"),
        _ => None,
    }
}

/// Return positive local credential evidence. In particular, a provider's
/// installation or config directory alone does not establish a sign-in.
pub fn sign_in_state(tool: &str, home: Option<&Path>) -> Option<SignIn> {
    let agent = resolve_agent_id(tool)?;
    let user_home = gwt_config::home::user_home()?;
    if agent == AgentId::Copilot {
        if home.is_some() {
            return None;
        }
        let path = user_home.to_str()?.to_owned();
        let env = HashMap::from([
            ("HOME".to_string(), path.clone()),
            ("USERPROFILE".to_string(), path),
        ]);
        let remove_env: Vec<String> = COPILOT_ADOPTION_OVERRIDES
            .iter()
            .map(|key| (*key).into())
            .collect();
        let executable = copilot_executable(&env, &remove_env, &user_home)?;
        return copilot_status(&executable, None, &env, &remove_env, &user_home).map(|identity| {
            SignIn {
                identity: Some(identity),
            }
        });
    }

    // Antigravity has a single native keyring profile and no documented
    // non-interactive identity probe. Opening it to test sign-in can trigger
    // browser login, so it continues to use the existing launch path.
    let default_home = user_home.join(local_home_name(&agent)?);
    let effective_home = home.unwrap_or(&default_home);
    if home.is_some()
        && (!effective_home.is_dir()
            || credential_paths(&agent).iter().any(|name| {
                effective_home
                    .join(name)
                    .symlink_metadata()
                    .is_ok_and(|metadata| is_link(&metadata))
            }))
    {
        return None;
    }
    let signed_in = has_file_sign_in(tool, effective_home) || cli_sign_in(&agent, home, &user_home);
    signed_in.then_some(SignIn { identity: None })
}

fn has_file_sign_in(tool: &str, home: &Path) -> bool {
    let read = |name| {
        let path = home.join(name);
        if !path.is_file() {
            return None;
        }
        let bytes = fs::read(path).ok()?;
        serde_json::from_slice::<Value>(&bytes).ok()
    };
    match resolve_agent_id(tool) {
        Some(AgentId::ClaudeCode) => read(".credentials.json").is_some_and(|auth| {
            has_secret(&auth["claudeAiOauth"]["accessToken"])
                || has_secret(&auth["claudeAiOauth"]["refreshToken"])
        }),
        Some(AgentId::Codex) => {
            home.join("auth.json").is_file()
                && matches!(
                    crate::issue_monitor::provider_auth_state(
                        "codex",
                        &crate::issue_monitor::ProviderAuthProbe {
                            codex_home: Some(home),
                            ..Default::default()
                        }
                    ),
                    crate::issue_monitor::ProviderAuthState::Authenticated
                )
        }
        Some(AgentId::GrokBuild) => read("auth.json").is_some_and(|auth| {
            auth.as_object().is_some_and(|profiles| {
                profiles.values().any(|profile| {
                    has_secret(&profile["key"]) || has_secret(&profile["refresh_token"])
                })
            })
        }),
        _ => false,
    }
}

fn credential_paths(agent: &AgentId) -> &[&str] {
    match agent {
        AgentId::ClaudeCode => &[".credentials.json", ".claude.json"],
        AgentId::Codex | AgentId::GrokBuild => &["auth.json"],
        _ => &[],
    }
}

fn has_secret(value: &Value) -> bool {
    value.as_str().is_some_and(|value| !value.trim().is_empty())
}

fn cli_sign_in(agent: &AgentId, home: Option<&Path>, user_home: &Path) -> bool {
    if cfg!(test) || gwt_config::home::gwt_home_override().is_some() {
        return false;
    }
    let (args, credential_overrides): (&[&str], &[&str]) = match agent {
        AgentId::ClaudeCode => (
            &["auth", "status", "--json"],
            &[
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "CLAUDE_CODE_OAUTH_TOKEN",
            ],
        ),
        AgentId::Codex => (&["login", "status"], &["OPENAI_API_KEY", "CODEX_API_KEY"]),
        _ => return false,
    };
    // On Claude Code an auth-status request can initialize a missing
    // configuration. Only probe a home with existing Claude state.
    if agent == &AgentId::ClaudeCode && !home.unwrap_or(user_home).join(".claude.json").is_file() {
        return false;
    }
    let Some(key) = home_env_key(agent.command()) else {
        return false;
    };
    let mut options = SpawnOptions::new("Check local CLI sign-in")
        .env("HOME", user_home)
        .env("USERPROFILE", user_home)
        .current_dir(user_home.to_path_buf())
        .capture(true, false)
        .forward_output(false);
    options = if let Some(home) = home {
        options.env(key, home)
    } else {
        options.env_remove(key)
    };
    for key in credential_overrides {
        options = options.env_remove(key);
    }
    let result = spawn_logged_blocking_with_deadline(
        &process_console::global(),
        ProcessKind::AgentBootstrap,
        agent.command(),
        args,
        options,
        Instant::now() + PROBE_TIMEOUT,
    );
    result.is_ok_and(|output| {
        output.success()
            && (agent != &AgentId::ClaudeCode
                || serde_json::from_str::<Value>(&output.stdout)
                    .is_ok_and(|body| body["loggedIn"].as_bool() == Some(true)))
    })
}

/// Verified provider program and optional in-memory credential for its child.
pub struct VerifiedCopilot {
    pub executable: PathBuf,
    pub token: Option<String>,
}

/// Validate the installed program and same credential source the child
/// will use. An isolated token leaves this function only after login matches.
pub fn verified_copilot_launch(
    account_id: &str,
    home: Option<&Path>,
    expected_identity: &str,
    env: &HashMap<String, String>,
    remove_env: &[String],
    cwd: &Path,
) -> Result<VerifiedCopilot, &'static str> {
    let trusted_home = gwt_config::home::user_home().ok_or("Could not locate the user home")?;
    let installed = copilot_executable(&HashMap::new(), &[], &trusted_home);
    let executable = copilot_executable(env, remove_env, cwd)
        .filter(|candidate| installed.as_ref() == Some(candidate))
        .filter(|candidate| {
            cwd.canonicalize()
                .is_ok_and(|root| !candidate.starts_with(root))
        })
        .ok_or("The launch does not use the installed Copilot executable")?;
    let (token, env_override) = if let Some(home) = home {
        let (key, value) =
            account_launch_env("gh", Some(home)).ok_or("Copilot credential home is not valid")?;
        let token = keychain_copilot_token(account_id)
            .ok_or("Copilot account has no Keychain credential")?;
        let mut env = env.clone();
        env.insert(key.into(), value);
        (Some(token), Some(env))
    } else {
        (None, None)
    };
    let token = confirm_copilot_identity(expected_identity, token, |token| {
        copilot_status(
            &executable,
            token,
            env_override.as_ref().unwrap_or(env),
            remove_env,
            cwd,
        )
    })?;
    Ok(VerifiedCopilot { executable, token })
}

fn confirm_copilot_identity(
    expected: &str,
    token: Option<String>,
    status: impl FnOnce(Option<&str>) -> Option<String>,
) -> Result<Option<String>, &'static str> {
    let actual = status(token.as_deref()).ok_or("Could not verify the Copilot account")?;
    if !expected.eq_ignore_ascii_case(&actual) {
        return Err("Copilot authentication belongs to a different account");
    }
    Ok(token)
}

#[cfg(target_os = "macos")]
fn keychain_copilot_token(account_id: &str) -> Option<String> {
    if cfg!(test)
        || gwt_config::home::gwt_home_override().is_some()
        || !normal_component(account_id)
    {
        return None;
    }
    let output = spawn_logged_blocking_with_deadline(
        &process_console::global(),
        ProcessKind::AgentBootstrap,
        "/usr/bin/security",
        &[
            "find-generic-password",
            "-s",
            COPILOT_KEYCHAIN_SERVICE,
            "-a",
            account_id,
            "-w",
        ],
        SpawnOptions::new("Read Copilot account credential")
            .capture(true, false)
            .forward_output(false),
        Instant::now() + PROBE_TIMEOUT,
    )
    .ok()?;
    if !output.success() {
        return None;
    }
    let token = output.stdout.trim().to_owned();
    (!token.is_empty()).then_some(token)
}

#[cfg(not(target_os = "macos"))]
fn keychain_copilot_token(_account_id: &str) -> Option<String> {
    None
}

fn copilot_executable(
    env: &HashMap<String, String>,
    remove_env: &[String],
    cwd: &Path,
) -> Option<PathBuf> {
    let value = |key| {
        if let Some((_, value)) = env.iter().find(|(name, _)| env_key_matches(name, key)) {
            return (!value.is_empty()).then(|| std::ffi::OsString::from(value));
        }
        if remove_env
            .iter()
            .any(|removed| env_key_matches(removed, key))
        {
            return None;
        }
        std::env::var_os(key).filter(|value| !value.is_empty())
    };
    if let Some(path) = value("PATH") {
        for directory in std::env::split_paths(&path) {
            if let Ok(binary) = which::which_in("copilot", Some(&directory), cwd) {
                return if directory.is_absolute() {
                    binary.canonicalize().ok()
                } else {
                    None
                };
            }
        }
    }

    let gh_data = value("XDG_DATA_HOME")
        .map(PathBuf::from)
        .map(|path| path.join("gh"))
        .or_else(|| {
            if cfg!(windows) {
                return value("LOCALAPPDATA").map(|path| PathBuf::from(path).join("GitHub CLI"));
            }
            None
        })
        .or_else(|| {
            value(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|path| PathBuf::from(path).join(".local/share/gh"))
        })?;
    let name = if cfg!(windows) {
        "copilot.exe"
    } else {
        "copilot"
    };
    let binary = gh_data.join("copilot").join(name);
    if binary.is_absolute() && binary.is_file() {
        return binary.canonicalize().ok();
    }
    None
}

fn env_key_matches(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn copilot_status(
    executable: &Path,
    token: Option<&str>,
    env: &HashMap<String, String>,
    remove_env: &[String],
    cwd: &Path,
) -> Option<String> {
    if cfg!(test) || gwt_config::home::gwt_home_override().is_some() {
        return None;
    }
    let run = || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?
            .block_on(async {
                tokio::time::timeout(
                    PROBE_TIMEOUT,
                    copilot_status_rpc(executable, token, env, remove_env, cwd),
                )
                .await
                .ok()
                .flatten()
            })
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        return std::thread::scope(|scope| scope.spawn(run).join().ok().flatten());
    }
    run()
}

async fn copilot_status_rpc(
    executable: &Path,
    token: Option<&str>,
    env: &HashMap<String, String>,
    remove_env: &[String],
    cwd: &Path,
) -> Option<String> {
    let mut request = gwt_core::process::ProcessPlanRequest::new(executable)
        .args(["--server", "--stdio"])
        .current_dir(cwd);
    for key in remove_env {
        request = request.env_remove(key);
    }
    for (key, value) in env {
        request = request.env(key, value);
    }
    request = request
        .env("COPILOT_GH", "true")
        .env("COPILOT_AUTO_UPDATE", "false")
        .env("COPILOT_ALLOW_ALL", "false")
        .env("GITHUB_COPILOT_PROMPT_MODE_EXTENSIONS", "false")
        .env("GITHUB_COPILOT_PROMPT_MODE_REPO_HOOKS", "false")
        .env("GITHUB_COPILOT_PROMPT_MODE_WORKSPACE_MCP", "false");
    if let Some(token) = token {
        request = request.env(COPILOT_TOKEN_ENV, token);
    }
    // The gh wrapper downloads a missing executable. Probing the same
    // already available binary avoids installation during startup.
    let mut child = gwt_core::process::resolved_tokio_command(request)
        .ok()?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut input = child.stdin.take()?;
    let mut output = BufReader::new(child.stdout.take()?);
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"auth.getStatus","params":{}}"#;
    let frame = format!("Content-Length: {}\r\n\r\n{request}", request.len());
    input.write_all(frame.as_bytes()).await.ok()?;

    // A small bounded response contains only status metadata. Ignore
    // notifications until the reply to our request is received.
    for _ in 0..8 {
        let mut length = None;
        for _ in 0..8 {
            let mut header = String::new();
            (&mut output).take(256).read_line(&mut header).await.ok()?;
            if header == "\r\n" {
                break;
            }
            if let Some(value) = header.strip_prefix("Content-Length:") {
                length = value.trim().parse::<usize>().ok();
            }
        }
        let length = length.filter(|length| *length <= 16 * 1024)?;
        let mut body = vec![0; length];
        output.read_exact(&mut body).await.ok()?;
        let response = serde_json::from_slice::<Value>(&body).ok()?;
        if response["id"].as_u64() == Some(1) {
            return parse_copilot_status(&response, token.is_some());
        }
    }
    None
}

fn parse_copilot_status(response: &Value, require_env_token: bool) -> Option<String> {
    let result = response.get("result")?;
    if result["isAuthenticated"].as_bool() != Some(true) {
        return None;
    }
    if require_env_token && !matches!(result["authType"].as_str(), Some("env" | "token")) {
        return None;
    }
    let login = result["login"].as_str()?.trim();
    let host = result["host"].as_str().unwrap_or("github.com");
    let host = host
        .strip_prefix("https://")
        .unwrap_or(host)
        .trim_end_matches('/');
    // Include the host because a login alone is ambiguous across GitHub
    // installations. Keep status values safe to display and persist.
    let is_login =
        |character: u8| character.is_ascii_alphanumeric() || matches!(character, b'-' | b'_');
    let is_host =
        |character: u8| character.is_ascii_alphanumeric() || matches!(character, b'-' | b'.');
    (!login.is_empty()
        && login.len() <= 128
        && login.bytes().all(is_login)
        && !host.is_empty()
        && host.len() <= 253
        && host.bytes().all(is_host))
    .then(|| format!("{host}/{login}").to_ascii_lowercase())
}

fn normal_component(name: &str) -> bool {
    !name.contains(['/', '\\', ':', '\0'])
        && !name.ends_with(['.', ' '])
        && matches!(
            (
                Path::new(name).components().next(),
                Path::new(name).components().count()
            ),
            (Some(Component::Normal(_)), 1)
        )
}

fn user_home() -> io::Result<PathBuf> {
    gwt_config::home::user_home()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Could not locate the user home"))
}

/// Prepare the account's home and link only the provider's shared paths.
/// Missing optional settings are skipped; history is shared from first use.
pub fn prepare(tool: &str, account_id: &str) -> io::Result<PathBuf> {
    prepare_at(&user_home()?, tool, account_id)
}

/// Remove only the exact gwt-managed home for this account.
pub fn remove(tool: &str, account_id: &str) -> io::Result<()> {
    remove_at(&user_home()?, tool, account_id)
}

/// Confirm a stored home is the expected existing directory for this account.
pub fn is_managed_home(tool: &str, account_id: &str, home: &Path) -> bool {
    user_home()
        .and_then(|root| managed_home(&root, tool, account_id, false))
        .is_ok_and(|managed| managed.is_some_and(|(_, path)| path == home))
}

fn prepare_at(user_home: &Path, tool: &str, account_id: &str) -> io::Result<PathBuf> {
    let (agent, destination) = managed_home(user_home, tool, account_id, true)?
        .expect("creating the managed directory always yields a path");
    if let Some(name) = local_home_name(&agent) {
        let source = user_home.join(name);
        let (settings, history): (&[&str], &str) = match agent {
            AgentId::ClaudeCode => (
                &[
                    "settings.json",
                    "CLAUDE.md",
                    "agents",
                    "commands",
                    "skills",
                    "plugins",
                ],
                "projects",
            ),
            AgentId::Codex => (
                &["config.toml", "AGENTS.md", "prompts", "skills", "rules"],
                "sessions",
            ),
            AgentId::GrokBuild => (&["config.toml", "skills"], "sessions"),
            _ => unreachable!("only tools with a local home share settings"),
        };
        let history_path = source.join(history);
        fs::create_dir_all(&history_path)?;
        for name in settings.iter().copied().chain(std::iter::once(history)) {
            let source = source.join(name);
            let target = destination.join(name);
            if source.exists()
                && target
                    .symlink_metadata()
                    .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
            {
                link_shared(&source.canonicalize()?, &target)?;
            }
        }
    }
    Ok(destination)
}

fn remove_at(user_home: &Path, tool: &str, account_id: &str) -> io::Result<()> {
    if let Some((_, path)) = managed_home(user_home, tool, account_id, false)? {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn managed_home(
    user_home: &Path,
    tool: &str,
    account_id: &str,
    create: bool,
) -> io::Result<Option<(AgentId, PathBuf)>> {
    let agent = resolve_agent_id(tool)
        .filter(|agent| home_env_key(agent.command()).is_some())
        .filter(|_| normal_component(account_id))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Invalid managed account home")
        })?;
    let mut path = user_home.to_path_buf();
    for name in [".gwt", "accounts", agent.command(), account_id] {
        path.push(name);
        if create {
            let dir = fs::DirBuilder::new();
            #[cfg(unix)]
            let dir = {
                use std::os::unix::fs::DirBuilderExt;
                let mut dir = dir;
                dir.mode(0o700);
                dir
            };
            if let Err(error) = dir.create(&path) {
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(error);
                }
            }
        }
        match path.symlink_metadata() {
            Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Managed account path traverses a link or a non-directory",
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(Some((agent, path)))
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
fn link_shared(source: &Path, destination: &Path) -> io::Result<()> {
    match std::os::unix::fs::symlink(source, destination) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        result => result,
    }
}

#[cfg(windows)]
fn link_shared(source: &Path, destination: &Path) -> io::Result<()> {
    let directory = source.is_dir();
    let link = if directory {
        std::os::windows::fs::symlink_dir(source, destination)
    } else {
        std::os::windows::fs::symlink_file(source, destination)
    };
    let error = match link {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => error,
    };
    let source = gwt_core::paths::normalize_windows_child_process_path(source);
    if directory && create_junction(&source, destination) {
        tracing::info!(reason = %error, "Shared account directory uses a junction");
        return Ok(());
    }
    tracing::warn!(reason = %error, "Shared account path uses a Windows copy");
    let staging = tempfile::tempdir_in(destination.parent().expect("share has a parent"))?;
    let copy = staging.path().join("share");
    copy_shared(&source, &copy)?;
    match fs::rename(&copy, destination) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        result => result,
    }
}

#[cfg(windows)]
fn create_junction(source: &Path, destination: &Path) -> bool {
    // mklink is a cmd builtin. Keep shell metacharacters out of the command;
    // the copy fallback handles paths that cannot be passed unambiguously.
    let safe = |path: &Path| {
        path.to_str().is_some_and(|path| {
            !path.contains(['"', '%', '!', '^', '&', '|', '<', '>', '(', ')', '\r', '\n'])
        })
    };
    let Some(windows) = std::env::var_os("SystemRoot") else {
        return false;
    };
    let cmd = PathBuf::from(windows).join("System32/cmd.exe");
    safe(source)
        && safe(destination)
        && gwt_core::process::hidden_command(cmd)
            .args(["/D", "/Q", "/V:OFF", "/C", "mklink", "/J"])
            .arg(destination)
            .arg(source)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

#[cfg(windows)]
fn copy_shared(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = source.symlink_metadata()?;
    // Do not follow nested reparse points while copying a shared directory.
    if is_link(&metadata) {
        return Ok(());
    }
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_shared(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        let mut input = fs::File::open(source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        io::copy(&mut input, &mut output)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_supported_tools_have_an_isolated_launch_environment() {
        let home = Path::new("/fixture/account");
        for (tool, key) in [
            ("claude", "CLAUDE_CONFIG_DIR"),
            ("codex", "CODEX_HOME"),
            ("grok", "GROK_HOME"),
            ("gh", "GH_CONFIG_DIR"),
        ] {
            assert_eq!(account_launch_env(tool, None), None);
            assert_eq!(
                account_launch_env(tool, Some(home)),
                Some((key, home.to_str().unwrap().to_string()))
            );
        }
        assert_eq!(account_launch_env("agy", Some(home)), None);
    }

    #[test]
    fn signed_in_files_are_positive_evidence_without_the_cli() {
        let home = tempfile::tempdir().unwrap();
        let cases = [
            (
                "claude",
                ".credentials.json",
                r#"{"claudeAiOauth":{"accessToken":"fixture"}}"#,
            ),
            (
                "codex",
                "auth.json",
                r#"{"tokens":{"refresh_token":"fixture"}}"#,
            ),
            (
                "grok",
                "auth.json",
                r#"{"https://auth.x.ai::fixture":{"key":"fixture"}}"#,
            ),
        ];
        for (tool, file, signed_in) in cases {
            let root = home.path().join(tool);
            fs::create_dir(&root).unwrap();
            let credentials = root.join(file);
            fs::write(&credentials, "{}").unwrap();
            assert!(!has_file_sign_in(tool, &root));
            fs::write(&credentials, signed_in).unwrap();
            assert!(has_file_sign_in(tool, &root));
        }
    }

    #[test]
    fn managed_codex_home_shares_settings_and_history_without_credentials() {
        let home = tempfile::tempdir().unwrap();
        let source = home.path().join(".codex");
        fs::create_dir(&source).unwrap();
        for name in ["config.toml", "auth.json", "state_5.sqlite"] {
            fs::write(source.join(name), "fixture").unwrap();
        }
        let dest = prepare_at(home.path(), "codex", "account-2").unwrap();
        assert_eq!(prepare_at(home.path(), "codex", "account-2").unwrap(), dest);
        assert_eq!(
            fs::read_to_string(dest.join("config.toml")).unwrap(),
            "fixture"
        );
        assert!(dest.join("sessions").is_dir());
        assert!(!dest.join("auth.json").exists());
        assert!(!dest.join("state_5.sqlite").exists());
        remove_at(home.path(), "codex", "account-2").unwrap();
        assert!(!dest.exists());
        assert!(source.join("config.toml").is_file());
        assert!(source.join("sessions").is_dir());
        assert!(source.join("auth.json").is_file());
        assert!(remove_at(home.path(), "codex", "../../.codex").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn remove_refuses_a_symlink_in_the_managed_home_path() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let account = outside.path().join("account");
        fs::create_dir(&account).unwrap();
        let root = home.path().join(".gwt/accounts");
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("codex")).unwrap();

        assert!(remove_at(home.path(), "codex", "account").is_err());
        assert!(prepare_at(home.path(), "codex", "account").is_err());
        assert!(account.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn an_isolated_home_cannot_sign_in_through_a_link_to_another_account() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("isolated");
        fs::create_dir(&home).unwrap();
        let other_credentials = root.path().join("other-auth.json");
        fs::write(
            &other_credentials,
            r#"{"tokens":{"access_token":"fixture"}}"#,
        )
        .unwrap();
        std::os::unix::fs::symlink(other_credentials, home.join("auth.json")).unwrap();
        assert_eq!(sign_in_state("codex", Some(&home)), None);
    }

    #[cfg(unix)]
    #[test]
    fn copilot_executable_rejects_a_relative_path_entry() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let executable = bin.join("copilot");
        fs::File::create(&executable)
            .unwrap()
            .set_permissions(fs::Permissions::from_mode(0o700))
            .unwrap();
        let mut env = HashMap::from([("PATH".into(), bin.to_str().unwrap().into())]);
        assert_eq!(
            copilot_executable(&env, &[], root.path()),
            Some(executable.canonicalize().unwrap())
        );
        env.insert("PATH".into(), "bin".into());
        assert_eq!(copilot_executable(&env, &[], root.path()), None);
    }

    #[test]
    fn copilot_requires_a_current_identity_match_before_releasing_the_token() {
        const DUMMY_TOKEN: &str = "fixture-copilot-token";
        let status = |token: Option<&str>| {
            assert_eq!(token, Some(DUMMY_TOKEN));
            Some("alice".to_string())
        };
        assert_eq!(
            confirm_copilot_identity("Alice", Some(DUMMY_TOKEN.into()), status),
            Ok(Some(DUMMY_TOKEN.into()))
        );
        assert_eq!(
            confirm_copilot_identity("bob", Some(DUMMY_TOKEN.into()), status),
            Err("Copilot authentication belongs to a different account")
        );
        assert_eq!(
            confirm_copilot_identity("alice", Some(DUMMY_TOKEN.into()), |_| None),
            Err("Could not verify the Copilot account")
        );
    }

    #[test]
    fn copilot_auth_status_requires_the_authenticated_sdk_receipt() {
        let result = |fields| serde_json::json!({"jsonrpc":"2.0","id":1,"result":fields});
        assert_eq!(
            parse_copilot_status(
                &result(serde_json::json!({"isAuthenticated":true,"login":"alice"})),
                false
            ),
            Some("github.com/alice".into())
        );
        assert_eq!(
            parse_copilot_status(
                &result(serde_json::json!({"isAuthenticated":false,"login":"alice"})),
                false
            ),
            None
        );
        assert_eq!(
            parse_copilot_status(
                &result(
                    serde_json::json!({"isAuthenticated":true,"authType":"gh-cli","login":"alice"})
                ),
                true
            ),
            None
        );
        assert_eq!(
            parse_copilot_status(
                &result(
                    serde_json::json!({"isAuthenticated":true,"authType":"env","host":"https://fixture.ghe.com","login":"alice"})
                ),
                true
            ),
            Some("fixture.ghe.com/alice".into())
        );
    }
}
