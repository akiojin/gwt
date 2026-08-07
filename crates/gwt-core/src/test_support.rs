//! Test-only helpers shared across gwt crates (SPEC-3016 FR-003).
//!
//! Canonical home for process-global test machinery: [`env_lock`] serializes
//! tests that mutate environment variables, [`ScopedEnvVar`] restores an
//! environment variable when dropped, and [`ScopedGwtHome`] isolates gwt home
//! path resolution without process-wide `HOME` mutation. gwt-core unit tests
//! reach this module via `crate::test_support`; dependent crates enable the
//! `test-support` cargo feature from their dev-dependencies. gwt-only
//! machinery (the fake `gh` harness and CLI fixtures) stays in
//! `crates/gwt/src/cli/test_support.rs`.

use std::{
    cell::RefCell,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
#[cfg(windows)]
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    process::{Child, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Process-wide lock serializing tests that read or mutate environment
/// variables. Lock this before constructing a [`ScopedEnvVar`].
pub fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

thread_local! {
    static GWT_HOME_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Returns the thread-local gwt home override used by tests.
pub fn gwt_home_override() -> Option<PathBuf> {
    GWT_HOME_OVERRIDE.with(|value| value.borrow().clone())
}

/// RAII guard that overrides the gwt home root for the current test thread.
///
/// Prefer this over mutating `HOME` for in-process tests. Environment
/// variables are process-global, so changing them in one parallel test can
/// make unrelated tests write into the real user home.
pub struct ScopedGwtHome {
    previous: Option<PathBuf>,
}

impl ScopedGwtHome {
    pub fn set(path: impl AsRef<Path>) -> Self {
        let next = path.as_ref().to_path_buf();
        let previous = GWT_HOME_OVERRIDE.with(|value| value.replace(Some(next)));
        Self { previous }
    }
}

impl Drop for ScopedGwtHome {
    fn drop(&mut self) {
        GWT_HOME_OVERRIDE.with(|value| {
            value.replace(self.previous.take());
        });
    }
}

/// RAII guard that sets or removes one environment variable and restores the
/// previous value on drop. Hold the [`env_lock`] mutex for the guard's whole
/// lifetime; the guard itself does not lock.
pub struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl ScopedEnvVar {
    /// Sets `key` to `value`, remembering the previous value for restore.
    pub fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }

    /// Removes `key`, remembering the previous value for restore.
    pub fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

/// Real executable fixture for Windows-only integration tests of the Bun
/// global Claude Code layout reported in Issue #3290.
#[cfg(windows)]
pub struct WindowsBunClaudeFixture {
    pub profile: PathBuf,
    pub bun_bin: PathBuf,
    pub bun_exe: PathBuf,
    pub placeholder: PathBuf,
    pub wrapper: PathBuf,
    pub native: PathBuf,
}

#[cfg(windows)]
impl WindowsBunClaudeFixture {
    /// Build the Unicode-profile fixture using the installed Node runtime as a
    /// real PE launcher. The copied `bun.exe` executes `cli-wrapper.cjs`, so
    /// production callers exercise resolver output through `CreateProcess`
    /// instead of stopping at plan inspection.
    pub fn create(root: &Path, version: &str) -> std::io::Result<Self> {
        let node = which::which("node.exe").map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("node.exe is required for the Windows Bun fixture: {error}"),
            )
        })?;
        let profile = root.join("ユーザー 太郎");
        let bun_bin = profile.join(".bun").join("bin");
        let package = profile
            .join(".bun")
            .join("install")
            .join("global")
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        let package_bin = package.join("bin");
        let optional_package = package
            .parent()
            .expect("scoped package has a parent")
            .join("claude-code-win32-x64");
        std::fs::create_dir_all(&bun_bin)?;
        std::fs::create_dir_all(&package_bin)?;
        std::fs::create_dir_all(&optional_package)?;
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"@anthropic-ai/claude-code","bin":{"claude":"bin/claude.exe"}}"#,
        )?;

        let bun_exe = bun_bin.join("bun.exe");
        std::fs::copy(&node, &bun_exe)?;
        std::fs::copy(&node, bun_bin.join("claude.exe"))?;

        let placeholder = package_bin.join("claude.exe");
        std::fs::write(
            &placeholder,
            b"echo Error: native binary not installed. Run postinstall.\r\n",
        )?;
        let wrapper = package.join("cli-wrapper.cjs");
        let output = serde_json::to_string(&format!("{version} (Claude Code)"))
            .map_err(std::io::Error::other)?;
        std::fs::write(&wrapper, format!("console.log({output});\n"))?;

        let native = optional_package.join("claude.exe");
        std::fs::copy(&node, &native)?;

        Ok(Self {
            profile,
            bun_bin,
            bun_exe,
            placeholder,
            wrapper,
            native,
        })
    }

    /// Remove both safe redirect targets, leaving only the text placeholder.
    pub fn remove_safe_targets(&self) -> std::io::Result<()> {
        std::fs::remove_file(&self.wrapper)?;
        std::fs::remove_file(&self.native)
    }
}

/// One request captured by [`WindowsNpmRegistryFixture`].
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmRegistryRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
}

/// Credential-free loopback npm registry for deterministic Windows agent
/// launch tests (SPEC-1921 Phase 75).
///
/// The fixture publishes minimal executable Codex and Claude packages at one
/// exact version, writes an npmrc below a Unicode `USERPROFILE`, and places a
/// failing `bunx.cmd` tripwire before the real npm toolchain on `PATH`.
#[cfg(windows)]
pub struct WindowsNpmRegistryFixture {
    pub profile: PathBuf,
    pub npmrc: PathBuf,
    pub npm_cache: PathBuf,
    pub tripwire_dir: PathBuf,
    pub bunx_marker: PathBuf,
    pub registry_url: String,
    pub exact_version: String,
    requests: Arc<Mutex<Vec<NpmRegistryRequest>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    address: SocketAddr,
}

#[cfg(windows)]
impl WindowsNpmRegistryFixture {
    pub fn create(root: &Path) -> std::io::Result<Self> {
        let profile = root.join("Phase 75 ユーザー");
        let npm_cache = profile.join("npm キャッシュ");
        let npmrc = profile.join(".npmrc");
        let tripwire_dir = root.join("runner tripwire");
        let bunx_marker = root.join("bunx-was-invoked.txt");
        std::fs::create_dir_all(&npm_cache)?;
        std::fs::create_dir_all(&tripwire_dir)?;

        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let registry_url = format!("http://127.0.0.1:{}/", address.port());
        let exact_version = "75.0.0".to_string();
        std::fs::write(
            &npmrc,
            format!(
                "registry={registry_url}\ncache={}\naudit=false\nfund=false\nupdate-notifier=false\n",
                npm_cache.display()
            ),
        )?;
        std::fs::write(
            tripwire_dir.join("bunx.cmd"),
            format!(
                "@echo off\r\n>\"{}\" echo bunx invoked\r\nexit /b 97\r\n",
                bunx_marker.display()
            ),
        )?;

        let packages = Arc::new(phase75_packages(&registry_url, &exact_version)?);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("phase75-loopback-npm".to_string())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let connection_packages = Arc::clone(&packages);
                            let connection_requests = Arc::clone(&thread_requests);
                            let _ = std::thread::Builder::new()
                                .name("phase75-loopback-npm-connection".to_string())
                                .spawn(move || {
                                    let _ = serve_npm_request(
                                        stream,
                                        &connection_packages,
                                        &connection_requests,
                                    );
                                });
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock
                                    | std::io::ErrorKind::Interrupted
                                    | std::io::ErrorKind::ConnectionAborted
                                    | std::io::ErrorKind::ConnectionReset
                                    | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })?;

        Ok(Self {
            profile,
            npmrc,
            npm_cache,
            tripwire_dir,
            bunx_marker,
            registry_url,
            exact_version,
            requests,
            stop,
            thread: Some(thread),
            address,
        })
    }

    /// Environment values required by the production resolver and the npm
    /// child processes. The ambient npm credential variables are intentionally
    /// not copied into this map.
    pub fn launch_env(&self) -> HashMap<String, String> {
        let inherited_path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![self.tripwire_dir.clone()];
        paths.extend(std::env::split_paths(&inherited_path));
        let path = std::env::join_paths(paths)
            .expect("Phase 75 fixture PATH entries must be representable")
            .to_string_lossy()
            .into_owned();
        HashMap::from([
            ("PATH".to_string(), path),
            (
                "USERPROFILE".to_string(),
                self.profile.to_string_lossy().into_owned(),
            ),
            (
                "HOME".to_string(),
                self.profile.to_string_lossy().into_owned(),
            ),
            (
                "NPM_CONFIG_USERCONFIG".to_string(),
                self.npmrc.to_string_lossy().into_owned(),
            ),
            ("NPM_CONFIG_REGISTRY".to_string(), self.registry_url.clone()),
            (
                "NPM_CONFIG_CACHE".to_string(),
                self.npm_cache.to_string_lossy().into_owned(),
            ),
            ("NPM_CONFIG_AUDIT".to_string(), "false".to_string()),
            ("NPM_CONFIG_FUND".to_string(), "false".to_string()),
            ("NPM_CONFIG_PROGRESS".to_string(), "false".to_string()),
            ("NPM_CONFIG_LOGLEVEL".to_string(), "verbose".to_string()),
            (
                "NPM_CONFIG_UPDATE_NOTIFIER".to_string(),
                "false".to_string(),
            ),
        ])
    }

    pub fn requests(&self) -> Vec<NpmRegistryRequest> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[cfg(windows)]
impl Drop for WindowsNpmRegistryFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(100));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Running real `gwt.exe` instance isolated below a caller-owned Unicode
/// profile. Phase 75 uses this to prove the public `/ws` and ConPTY boundary
/// without attaching to a production or already-running gwt process.
#[cfg(windows)]
pub struct WindowsRealGwtFixture {
    child: Option<Child>,
    pub home: PathBuf,
    pub workspace: PathBuf,
    pub http_url: String,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

#[cfg(windows)]
impl WindowsRealGwtFixture {
    pub fn start(
        root: &Path,
        gwt_bin: &Path,
        home: &Path,
        workspace: &Path,
        env: &HashMap<String, String>,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(root)?;
        let gwt_home = home.join(".gwt");
        std::fs::create_dir_all(&gwt_home)?;
        std::fs::create_dir_all(workspace)?;
        std::fs::write(
            gwt_home.join("config.toml"),
            "[board]\noauth_redirect_port = 0\n",
        )?;
        let index_fixture = root.join("index-status.json");
        std::fs::write(
            &index_fixture,
            r#"{"state":"ready","detail":"Phase 75 Windows E2E"}"#,
        )?;
        let url_path = root.join("gwt-browser-url.txt");
        let stdout_path = root.join("gwt-stdout.log");
        let stderr_path = root.join("gwt-stderr.log");
        let stdout = std::fs::File::create(&stdout_path)?;
        let stderr = std::fs::File::create(&stderr_path)?;
        let mut command = crate::process::hidden_command(gwt_bin);
        crate::process::scrub_git_env(&mut command);
        command
            .args(["--no-tray", "--no-open", "--port", "0"])
            .current_dir(workspace)
            .envs(env)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("XDG_CONFIG_HOME", home.join("xdg-config"))
            .env("XDG_CACHE_HOME", home.join("xdg-cache"))
            .env("XDG_DATA_HOME", home.join("xdg-data"))
            .env("XDG_STATE_HOME", home.join("xdg-state"))
            .env("CI", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GWT_BROWSER_URL_FILE", &url_path)
            .env("GWT_INDEX_TEST_FIXTURE", &index_fixture)
            .env_remove("GWT_SESSION_ID")
            .env_remove("GWT_FORCE_NEW_INSTANCE")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        let child = command.spawn()?;
        Self::wait_ready(child, home, workspace, url_path, stdout_path, stderr_path)
    }

    pub fn public_ws_url(&self) -> String {
        self.http_url
            .replacen("http://", "ws://", 1)
            .trim_end_matches('/')
            .to_string()
            + "/ws"
    }

    pub fn stderr(&self) -> String {
        std::fs::read_to_string(&self.stderr_path)
            .unwrap_or_else(|error| format!("<unreadable: {error}>"))
    }

    fn wait_ready(
        child: Child,
        home: &Path,
        workspace: &Path,
        url_path: PathBuf,
        stdout_path: PathBuf,
        stderr_path: PathBuf,
    ) -> std::io::Result<Self> {
        let mut fixture = Self {
            child: Some(child),
            home: home.to_path_buf(),
            workspace: workspace.to_path_buf(),
            http_url: String::new(),
            stdout_path,
            stderr_path,
        };
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if let Some(status) = fixture
                .child
                .as_mut()
                .expect("running gwt child")
                .try_wait()?
            {
                return Err(std::io::Error::other(format!(
                    "gwt exited before readiness ({status})\nstdout:\n{}\nstderr:\n{}",
                    fixture.stdout(),
                    fixture.stderr()
                )));
            }
            if let Ok(raw_url) = std::fs::read_to_string(&url_path) {
                let url = raw_url.trim();
                if !url.is_empty() && http_health_is_ready(url) {
                    fixture.http_url = url.to_string();
                    return Ok(fixture);
                }
            }
            if Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "gwt did not become ready\nstdout:\n{}\nstderr:\n{}",
                        fixture.stdout(),
                        fixture.stderr()
                    ),
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn stdout(&self) -> String {
        std::fs::read_to_string(&self.stdout_path)
            .unwrap_or_else(|error| format!("<unreadable: {error}>"))
    }
}

#[cfg(windows)]
impl Drop for WindowsRealGwtFixture {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

#[cfg(windows)]
fn http_health_is_ready(base_url: &str) -> bool {
    let Some(authority) = base_url
        .strip_prefix("http://")
        .and_then(|value| value.trim_end_matches('/').split('/').next())
    else {
        return false;
    };
    let Ok(address) = authority.parse::<SocketAddr>() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    if write!(
        stream,
        "GET /healthz HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
    )
    .is_err()
    {
        return false;
    }
    let mut response = String::new();
    stream.read_to_string(&mut response).is_ok()
        && response
            .lines()
            .next()
            .is_some_and(|line| line.contains(" 200 "))
}

#[cfg(windows)]
struct FakeNpmPackage {
    metadata: Vec<u8>,
    tarball_path: String,
    tarball: Vec<u8>,
}

#[cfg(windows)]
fn phase75_packages(
    registry_url: &str,
    version: &str,
) -> std::io::Result<HashMap<String, FakeNpmPackage>> {
    [
        ("@openai/codex", "codex"),
        ("@anthropic-ai/claude-code", "claude"),
    ]
    .into_iter()
    .map(|(package, binary)| {
        let archive_name = format!("{}-{version}.tgz", binary);
        let tarball_path = format!("/{package}/-/{archive_name}");
        let tarball_url = format!("{}{package}/-/{archive_name}", registry_url);
        let bin = serde_json::Map::from_iter([(
            binary.to_string(),
            serde_json::Value::String(format!("bin/{binary}.js")),
        )]);
        let version_metadata = serde_json::json!({
            "name": package,
            "version": version,
            "bin": bin,
            "dist": { "tarball": tarball_url }
        });
        let versions = serde_json::Map::from_iter([(version.to_string(), version_metadata)]);
        let metadata = serde_json::to_vec(&serde_json::json!({
            "name": package,
            "dist-tags": { "latest": version },
            "versions": versions
        }))
        .map_err(std::io::Error::other)?;
        let tarball = fake_npm_tarball(package, binary, version)?;
        Ok((
            package.to_string(),
            FakeNpmPackage {
                metadata,
                tarball_path,
                tarball,
            },
        ))
    })
    .collect()
}

#[cfg(windows)]
fn fake_npm_tarball(package: &str, binary: &str, version: &str) -> std::io::Result<Vec<u8>> {
    let mut compressed = Vec::new();
    let encoder = flate2::GzBuilder::new()
        .mtime(0)
        .write(&mut compressed, flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    let bin = serde_json::Map::from_iter([(
        binary.to_string(),
        serde_json::Value::String(format!("bin/{binary}.js")),
    )]);
    let package_json = serde_json::to_vec(&serde_json::json!({
        "name": package,
        "version": version,
        "bin": bin
    }))
    .map_err(std::io::Error::other)?;
    append_tar_bytes(&mut archive, "package/package.json", &package_json, 0o644)?;
    let script = format!(
        "#!/usr/bin/env node\n\
const fs = require('fs');\n\
const path = require('path');\n\
const childProcess = require('child_process');\n\
const version = {version:?};\n\
const packageName = {package:?};\n\
if (process.argv.includes('--version')) {{ console.log(version); process.exit(0); }}\n\
const capture = process.env.GWT_PHASE75_CAPTURE;\n\
if (!capture) {{ console.error('GWT_PHASE75_CAPTURE is required'); process.exit(2); }}\n\
const hookPath = packageName === '@openai/codex'\n\
  ? path.join(process.cwd(), '.codex', 'hooks.json')\n\
  : path.join(process.cwd(), '.claude', 'settings.local.json');\n\
const hookConfig = JSON.parse(fs.readFileSync(hookPath, 'utf8'));\n\
const groups = hookConfig.hooks && hookConfig.hooks.SessionStart;\n\
const hookCommand = groups && groups[0] && groups[0].hooks && groups[0].hooks[0] && groups[0].hooks[0].command;\n\
if (!hookCommand) {{ console.error('generated SessionStart hook is missing'); process.exit(3); }}\n\
const providerSessionId = process.env.GWT_PHASE75_PROVIDER_SESSION_ID || 'phase75-provider-session';\n\
const hook = childProcess.spawnSync(hookCommand, [], {{\n\
  cwd: process.cwd(),\n\
  env: process.env,\n\
  input: JSON.stringify({{ session_id: providerSessionId, cwd: process.cwd() }}),\n\
  encoding: 'utf8',\n\
  shell: true\n\
}});\n\
const receipt = {{\n\
  argv: process.argv.slice(2),\n\
  package: packageName,\n\
  version,\n\
  cwd: process.cwd(),\n\
  project_root: process.env.GWT_PROJECT_ROOT,\n\
  gwt_session_id: process.env.GWT_SESSION_ID,\n\
  runtime_file: process.env.GWT_SESSION_RUNTIME_PATH ? path.basename(process.env.GWT_SESSION_RUNTIME_PATH) : null,\n\
  gwt_bin_name: process.env.GWT_BIN_PATH ? path.basename(process.env.GWT_BIN_PATH) : null,\n\
  gwt_bin_path: process.env.GWT_BIN_PATH || null,\n\
  npm_exec_identity: process.env.npm_execpath ? path.basename(process.env.npm_execpath) : null,\n\
  tty: Boolean(process.stdout.isTTY),\n\
  hook_status: hook.status,\n\
  hook_stdout_present: Boolean(hook.stdout),\n\
  hook_stderr_present: Boolean(hook.stderr),\n\
  hook_stdout: hook.stdout || null,\n\
  hook_stderr: hook.stderr || null,\n\
  hook_error: hook.error ? String(hook.error) : null,\n\
  codex_thread_id: process.env.CODEX_THREAD_ID || null,\n\
  provider_session_id: providerSessionId,\n\
  hook_forward_token_present: Boolean(process.env.GWT_HOOK_FORWARD_TOKEN)\n\
}};\n\
fs.writeFileSync(capture, JSON.stringify(receipt));\n\
if (hook.status !== 0) {{ console.error('SessionStart hook failed', hook.status); process.exit(4); }}\n\
console.log('phase75-agent-ready');\n\
const holdOpenMs = Number(process.env.GWT_PHASE75_HOLD_OPEN_MS || '0');\n\
if (Number.isFinite(holdOpenMs) && holdOpenMs > 0) {{\n\
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, holdOpenMs);\n\
}}\n"
    );
    append_tar_bytes(
        &mut archive,
        &format!("package/bin/{binary}.js"),
        script.as_bytes(),
        0o755,
    )?;
    archive.finish()?;
    archive.into_inner()?.finish()?;
    Ok(compressed)
}

#[cfg(windows)]
fn append_tar_bytes<W: Write>(
    archive: &mut tar::Builder<W>,
    path: &str,
    bytes: &[u8],
    mode: u32,
) -> std::io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(mode);
    header.set_mtime(0);
    header.set_cksum();
    archive.append_data(&mut header, path, bytes)
}

#[cfg(windows)]
fn serve_npm_request(
    mut stream: TcpStream,
    packages: &HashMap<String, FakeNpmPackage>,
    requests: &Arc<Mutex<Vec<NpmRegistryRequest>>>,
) -> std::io::Result<()> {
    // The listener is nonblocking so the fixture can observe its stop flag.
    // On Windows an accepted socket can retain that mode; a read racing the
    // client's first bytes would then return WouldBlock and dropping the
    // stream surfaces as ECONNRESET in npm. Connection workers use bounded
    // blocking I/O instead.
    stream.set_nonblocking(false)?;
    // npm's HTTP agent may preconnect a socket several seconds before it
    // writes the request. A short timeout turns that valid idle connection
    // into ECONNRESET and sends npm into its long retry backoff. Keep the
    // deterministic fixture tolerant of the same connection reuse window as
    // a real registry while still bounding abandoned clients.
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..count]);
        if raw.windows(4).any(|window| window == b"\r\n\r\n") || raw.len() > 64 * 1024 {
            break;
        }
    }
    let request = String::from_utf8_lossy(&raw);
    let mut lines = request.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let path = request_line.next().unwrap_or_default().to_string();
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect::<HashMap<_, _>>();
    requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(NpmRegistryRequest {
            method,
            path: path.clone(),
            headers,
        });

    let decoded = percent_decode_registry_path(&path);
    let response = packages.values().find_map(|package| {
        if decoded == package.tarball_path {
            Some((
                "200 OK",
                "application/octet-stream",
                package.tarball.as_slice(),
            ))
        } else {
            None
        }
    });
    let response = response.or_else(|| {
        packages.iter().find_map(|(name, package)| {
            (decoded.trim_start_matches('/') == name).then_some((
                "200 OK",
                "application/json",
                package.metadata.as_slice(),
            ))
        })
    });
    let (status, content_type, body) = response.unwrap_or((
        "404 Not Found",
        "application/json",
        b"{\"error\":\"not found\"}".as_slice(),
    ));
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

#[cfg(windows)]
fn percent_decode_registry_path(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if let Some(value) = bytes
                .get(index + 1..index + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                decoded.push(value);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_registry_path_handles_non_ascii_without_losing_valid_decoding() {
        assert_eq!(percent_decode_registry_path("/%€/package"), "/%€/package");
        assert_eq!(
            percent_decode_registry_path("/%40Scope%2FPackage?cache=1"),
            "/@scope/package"
        );
    }

    #[test]
    fn scoped_gwt_home_is_thread_local_and_restores() {
        assert!(gwt_home_override().is_none());
        let home = std::env::temp_dir().join("gwt-test-home-override");

        {
            let _guard = ScopedGwtHome::set(&home);
            assert_eq!(gwt_home_override().as_deref(), Some(home.as_path()));
            std::thread::spawn(|| {
                assert!(gwt_home_override().is_none());
            })
            .join()
            .expect("thread");
        }

        assert!(gwt_home_override().is_none());
    }
}
