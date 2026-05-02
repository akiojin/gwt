//! Self-update support via GitHub Releases.
//!
//! This module implements:
//! - Update discovery via GitHub Releases (latest)
//! - TTL-based local cache to avoid repeated API calls
//! - User-approved apply flow:
//!   - Portable payload (tar.gz/zip) => extract and replace the running executable, then restart
//!   - Installer payload (.dmg/.pkg/.msi) => run installer with privileges/UAC, then restart
//! - Internal helper modes (`__internal`) to safely apply updates after the parent process exits

use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn is_ci() -> bool {
    std::env::var("CI")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

const DEFAULT_OWNER: &str = "akiojin";
const DEFAULT_REPO: &str = "gwt";
const DEFAULT_TTL: Duration = Duration::from_secs(60 * 60 * 24);
const DEFAULT_API_BASE_URL: &str = "https://api.github.com";
const DOCKER_LINUX_PRIMARY_BINARY_NAME: &str = "gwt";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UpdateCacheFile {
    checked_at: DateTime<Utc>,
    latest_version: Option<String>,
    release_url: Option<String>,
    #[serde(default)]
    portable_asset_url: Option<String>,
    #[serde(default)]
    installer_asset_url: Option<String>,
    /// Legacy cache field (used by older versions).
    #[serde(default)]
    asset_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
/// Current update state exposed to the UI.
pub enum UpdateState {
    /// No update is available (or not yet checked).
    UpToDate {
        /// When the last update check completed (if known).
        #[serde(skip_serializing_if = "Option::is_none")]
        checked_at: Option<DateTime<Utc>>,
    },
    /// A newer version is available on GitHub Releases.
    Available {
        /// Current running version.
        current: String,
        /// Latest available version.
        latest: String,
        /// Release page URL.
        release_url: String,
        /// Preferred payload URL for this platform/install, if present.
        #[serde(skip_serializing_if = "Option::is_none")]
        asset_url: Option<String>,
        /// When this update was last checked.
        checked_at: DateTime<Utc>,
    },
    /// Update check or apply failed (best-effort; the app should keep running).
    Failed {
        /// Human-readable failure message.
        message: String,
        /// When the failure was recorded.
        failed_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartArgsFile {
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize, Clone)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerKind {
    MacDmg,
    MacPkg,
    WindowsMsi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedPayload {
    PortableBinary { path: PathBuf },
    Installer { path: PathBuf, kind: InstallerKind },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledDockerLinuxBundle {
    pub version: String,
    pub gwt_path: PathBuf,
    pub gwtd_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ApplyPlan {
    Portable { url: String },
    Installer { url: String, kind: InstallerKind },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Platform {
    os: String,
    arch: String,
}

impl Platform {
    fn detect() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }
    }

    fn artifact(&self) -> Option<&'static str> {
        match (self.os.as_str(), self.arch.as_str()) {
            ("linux", "x86_64") => Some("linux-x86_64"),
            ("linux", "aarch64") => Some("linux-aarch64"),
            ("macos", "x86_64") => Some("macos-x86_64"),
            ("macos", "aarch64") => Some("macos-arm64"),
            ("windows", "x86_64") => Some("windows-x86_64"),
            _ => None,
        }
    }

    fn binary_name(&self) -> String {
        crate::release_contract::bundle_binary_names(&self.os)
            .and_then(|names| names.into_iter().next())
            .unwrap_or_else(|| {
                if self.os == "windows" {
                    "gwt.exe".to_string()
                } else {
                    "gwt".to_string()
                }
            })
    }

    fn portable_asset_name(&self) -> Option<String> {
        crate::release_contract::portable_asset_name(&self.os, &self.arch)
    }
}

fn normalize_docker_linux_arch(raw: &str) -> Option<&'static str> {
    match raw
        .trim()
        .to_ascii_lowercase()
        .split('/')
        .next()
        .unwrap_or_default()
    {
        "x86_64" | "amd64" | "x64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        _ => None,
    }
}

fn docker_linux_bundle_asset_name(arch: &str) -> Result<String, String> {
    match normalize_docker_linux_arch(arch) {
        Some("x86_64") => Ok("gwt-linux-x86_64.tar.gz".to_string()),
        Some("aarch64") => Ok("gwt-linux-aarch64.tar.gz".to_string()),
        Some(other) => Ok(format!("gwt-linux-{other}.tar.gz")),
        None => Err(format!(
            "Unsupported Docker Linux bundle architecture: {arch}. Expected x86_64/amd64 or aarch64/arm64"
        )),
    }
}

#[derive(Debug, Clone)]
pub struct UpdateManager {
    current_version: Version,
    owner: String,
    repo: String,
    ttl: Duration,
    api_base_url: String,
    cache_path: PathBuf,
    updates_dir: PathBuf,
    client: Client,
}

impl Default for UpdateManager {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateManager {
    /// Create a new update manager for the current running version.
    pub fn new() -> Self {
        let current_version =
            Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0));

        let cache_path = crate::paths::gwt_update_cache_path();
        let updates_dir = crate::paths::gwt_updates_dir();

        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&format!("gwt/{}", env!("CARGO_PKG_VERSION")))
                .unwrap_or_else(|_| HeaderValue::from_static("gwt")),
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );

        let client = Client::builder()
            .default_headers(headers)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            current_version,
            owner: DEFAULT_OWNER.to_string(),
            repo: DEFAULT_REPO.to_string(),
            ttl: DEFAULT_TTL,
            api_base_url: DEFAULT_API_BASE_URL.to_string(),
            cache_path,
            updates_dir,
            client,
        }
    }

    #[cfg(test)]
    fn with_api_base_url(mut self, api_base_url: impl Into<String>) -> Self {
        self.api_base_url = api_base_url.into();
        self
    }

    #[cfg(test)]
    fn with_cache_path(mut self, path: PathBuf) -> Self {
        self.cache_path = path;
        self
    }

    #[cfg(test)]
    fn with_updates_dir(mut self, dir: PathBuf) -> Self {
        self.updates_dir = dir;
        self
    }

    pub fn check(&self, force: bool) -> UpdateState {
        self.check_for_executable(force, None)
    }

    pub fn check_for_executable(&self, force: bool, current_exe: Option<&Path>) -> UpdateState {
        let now = Utc::now();
        let cache = read_cache(&self.cache_path).ok();

        if !force {
            if let Some(cache) = &cache {
                if now
                    .signed_duration_since(cache.checked_at)
                    .to_std()
                    .ok()
                    .is_some_and(|age| age < self.ttl)
                {
                    return self.state_from_cache(cache, current_exe);
                }
            }
        }

        match self.fetch_latest_release() {
            Ok(release) => {
                let Some(latest_ver) = parse_tag_version(&release.tag_name) else {
                    return UpdateState::Failed {
                        message: format!(
                            "Failed to parse release tag as version: {}",
                            release.tag_name
                        ),
                        failed_at: now,
                    };
                };

                let platform = Platform::detect();

                let portable_asset_url = platform
                    .portable_asset_name()
                    .and_then(|name| release.assets.iter().find(|a| a.name == name))
                    .map(|a| a.browser_download_url.clone());

                let installer_asset_url = find_installer_asset_url(&platform, &release.assets);

                let asset_url = choose_apply_plan(
                    &platform,
                    current_exe,
                    portable_asset_url.as_deref(),
                    installer_asset_url.as_deref(),
                )
                .map(|p| match p {
                    ApplyPlan::Portable { url } => url,
                    ApplyPlan::Installer { url, .. } => url,
                });

                let cache_file = UpdateCacheFile {
                    checked_at: now,
                    latest_version: Some(latest_ver.to_string()),
                    release_url: Some(release.html_url.clone()),
                    portable_asset_url,
                    installer_asset_url,
                    asset_url: asset_url.clone(),
                };
                let _ = write_cache(&self.cache_path, &cache_file);

                if latest_ver > self.current_version {
                    UpdateState::Available {
                        current: self.current_version.to_string(),
                        latest: latest_ver.to_string(),
                        release_url: release.html_url,
                        asset_url,
                        checked_at: now,
                    }
                } else {
                    UpdateState::UpToDate {
                        checked_at: Some(now),
                    }
                }
            }
            Err(err) => {
                if !force {
                    if let Some(cache) = &cache {
                        return self.state_from_cache(cache, current_exe);
                    }
                }
                UpdateState::Failed {
                    message: err,
                    failed_at: now,
                }
            }
        }
    }

    pub fn prepare_update(&self, latest: &str, asset_url: &str) -> Result<PreparedPayload, String> {
        let update_dir = self
            .updates_dir
            .join(format!("v{}", latest.trim().trim_start_matches('v')));
        fs::create_dir_all(&update_dir).map_err(|e| format!("Failed to create update dir: {e}"))?;

        let asset_name = asset_name_from_url(asset_url).unwrap_or_else(|| "gwt-update".to_string());
        let dest = update_dir.join(&asset_name);

        self.download_asset(asset_url, &dest)?;

        let dest_str = dest.to_string_lossy().to_string();
        if dest_str.ends_with(".tar.gz") || dest_str.ends_with(".zip") {
            let extract_dir = update_dir.join("extract");
            let _ = fs::remove_dir_all(&extract_dir);
            fs::create_dir_all(&extract_dir)
                .map_err(|e| format!("Failed to create extract dir: {e}"))?;
            extract_archive(&dest, &extract_dir)?;
            let platform = Platform::detect();
            let binary_name = platform.binary_name();
            let (binary_path, daemon_path) =
                find_extracted_bundle_binaries(&extract_dir, &binary_name)?;
            ensure_executable(&binary_path)?;
            ensure_executable(&daemon_path)?;
            return Ok(PreparedPayload::PortableBinary { path: binary_path });
        }

        if dest_str.ends_with(".pkg") {
            return Ok(PreparedPayload::Installer {
                path: dest,
                kind: InstallerKind::MacPkg,
            });
        }

        if dest_str.ends_with(".dmg") {
            return Ok(PreparedPayload::Installer {
                path: dest,
                kind: InstallerKind::MacDmg,
            });
        }

        if dest_str.ends_with(".msi") {
            return Ok(PreparedPayload::Installer {
                path: dest,
                kind: InstallerKind::WindowsMsi,
            });
        }

        // Portable direct binary.
        ensure_executable(&dest)?;
        Ok(PreparedPayload::PortableBinary { path: dest })
    }

    pub fn install_latest_docker_linux_bundle(
        &self,
        target_arch: &str,
        target_gwt: &Path,
        target_gwtd: &Path,
    ) -> Result<InstalledDockerLinuxBundle, String> {
        let release = self.fetch_latest_release()?;
        let version = parse_tag_version(&release.tag_name)
            .ok_or_else(|| {
                format!(
                    "Failed to parse release tag as version: {}",
                    release.tag_name
                )
            })?
            .to_string();
        let asset_name = docker_linux_bundle_asset_name(target_arch)?;
        let asset_url = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .map(|asset| asset.browser_download_url.as_str())
            .ok_or_else(|| {
                format!(
                    "Latest release {} does not include required Docker bundle asset {}",
                    release.html_url, asset_name
                )
            })?;

        self.install_docker_linux_bundle_from_url(
            &version,
            &asset_name,
            asset_url,
            target_gwt,
            target_gwtd,
        )
    }

    fn install_docker_linux_bundle_from_url(
        &self,
        version: &str,
        asset_name: &str,
        asset_url: &str,
        target_gwt: &Path,
        target_gwtd: &Path,
    ) -> Result<InstalledDockerLinuxBundle, String> {
        let update_dir = self
            .updates_dir
            .join("docker")
            .join(format!("v{}", version.trim().trim_start_matches('v')));
        fs::create_dir_all(&update_dir).map_err(|e| format!("Failed to create update dir: {e}"))?;

        let dest = update_dir.join(asset_name);
        self.download_asset(asset_url, &dest)?;

        let extract_dir = update_dir.join("extract");
        let _ = fs::remove_dir_all(&extract_dir);
        fs::create_dir_all(&extract_dir)
            .map_err(|e| format!("Failed to create extract dir: {e}"))?;
        extract_archive(&dest, &extract_dir)?;
        let (gwt_source, gwtd_source) =
            find_extracted_bundle_binaries(&extract_dir, DOCKER_LINUX_PRIMARY_BINARY_NAME)?;
        ensure_executable(&gwt_source)?;
        ensure_executable(&gwtd_source)?;
        replace_executables_with_retry(&[
            (target_gwtd, gwtd_source.as_path()),
            (target_gwt, gwt_source.as_path()),
        ])?;

        Ok(InstalledDockerLinuxBundle {
            version: version.to_string(),
            gwt_path: target_gwt.to_path_buf(),
            gwtd_path: target_gwtd.to_path_buf(),
        })
    }

    fn download_asset(&self, asset_url: &str, dest: &Path) -> Result<(), String> {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create payload dir: {e}"))?;
        }

        let res = self
            .client
            .get(asset_url)
            .send()
            .map_err(|e| format!("Download failed: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("Download failed with status {}", res.status()));
        }

        let mut file =
            fs::File::create(dest).map_err(|e| format!("Failed to create payload file: {e}"))?;
        let mut reader = res;
        io::copy(&mut reader, &mut file).map_err(|e| format!("Failed to write payload: {e}"))?;

        let size = fs::metadata(dest).map(|m| m.len()).unwrap_or_default();
        if size == 0 {
            let _ = fs::remove_file(dest);
            return Err("Downloaded payload is empty".to_string());
        }
        Ok(())
    }

    pub fn write_restart_args_file(&self, path: &Path, args: Vec<String>) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Invalid args file path".to_string())?;
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create args dir: {e}"))?;
        let cwd = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .to_string_lossy()
            .to_string();
        write_json_atomic(path, &RestartArgsFile { args, cwd })
            .map_err(|e| format!("Failed to write args file: {e}"))
    }

    pub fn read_restart_args_file(path: &Path) -> Result<Vec<String>, String> {
        let bytes = fs::read(path).map_err(|e| format!("Failed to read args file: {e}"))?;
        let parsed: RestartArgsFile = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Failed to parse args file: {e}"))?;
        Ok(parsed.args)
    }

    pub fn spawn_internal_apply_update(
        &self,
        helper_exe: &Path,
        old_pid: u32,
        target_exe: &Path,
        new_exe: &Path,
        args_file: &Path,
    ) -> Result<(), String> {
        std::process::Command::new(helper_exe)
            .arg("__internal")
            .arg("apply-update")
            .arg("--old-pid")
            .arg(old_pid.to_string())
            .arg("--target")
            .arg(target_exe)
            .arg("--source")
            .arg(new_exe)
            .arg("--args-file")
            .arg(args_file)
            .spawn()
            .map_err(|e| format!("Failed to spawn update helper: {e}"))?;
        Ok(())
    }

    pub fn spawn_internal_run_installer(
        &self,
        helper_exe: &Path,
        old_pid: u32,
        target_exe: &Path,
        installer: &Path,
        installer_kind: InstallerKind,
        args_file: &Path,
    ) -> Result<(), String> {
        std::process::Command::new(helper_exe)
            .arg("__internal")
            .arg("run-installer")
            .arg("--old-pid")
            .arg(old_pid.to_string())
            .arg("--target")
            .arg(target_exe)
            .arg("--installer")
            .arg(installer)
            .arg("--installer-kind")
            .arg(match installer_kind {
                InstallerKind::MacDmg => "mac_dmg",
                InstallerKind::MacPkg => "mac_pkg",
                InstallerKind::WindowsMsi => "windows_msi",
            })
            .arg("--args-file")
            .arg(args_file)
            .spawn()
            .map_err(|e| format!("Failed to spawn installer helper: {e}"))?;
        Ok(())
    }

    pub fn make_helper_copy(&self, current_exe: &Path, latest: &str) -> Result<PathBuf, String> {
        let update_dir = self
            .updates_dir
            .join(format!("v{}", latest.trim().trim_start_matches('v')));
        fs::create_dir_all(&update_dir).map_err(|e| format!("Failed to create update dir: {e}"))?;

        #[cfg(windows)]
        let helper_name = "gwt-update-helper.exe".to_string();
        #[cfg(not(windows))]
        let helper_name = current_exe
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| format!("{s}.update-helper"))
            .unwrap_or_else(|| "gwt.update-helper".to_string());
        let helper_path = update_dir.join(helper_name);

        fs::copy(current_exe, &helper_path)
            .map_err(|e| format!("Failed to copy update helper: {e}"))?;
        Ok(helper_path)
    }

    pub fn cache_path(&self) -> &Path {
        &self.cache_path
    }

    pub fn updates_dir(&self) -> &Path {
        &self.updates_dir
    }

    fn fetch_latest_release(&self) -> Result<GitHubRelease, String> {
        let url = format!(
            "{}/repos/{}/{}/releases/latest",
            self.api_base_url.trim_end_matches('/'),
            self.owner,
            self.repo
        );
        let res = self
            .client
            .get(&url)
            .send()
            .map_err(|e| format!("Failed to fetch latest release: {e}"))?;
        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch latest release: status {}",
                res.status()
            ));
        }
        res.json::<GitHubRelease>()
            .map_err(|e| format!("Failed to parse GitHub release JSON: {e}"))
    }

    fn state_from_cache(&self, cache: &UpdateCacheFile, current_exe: Option<&Path>) -> UpdateState {
        let checked_at = cache.checked_at;

        let Some(latest_str) = cache.latest_version.as_deref() else {
            return UpdateState::UpToDate {
                checked_at: Some(checked_at),
            };
        };
        let Ok(latest_ver) = Version::parse(latest_str) else {
            return UpdateState::UpToDate {
                checked_at: Some(checked_at),
            };
        };

        if latest_ver > self.current_version {
            let release_url = cache.release_url.clone().unwrap_or_else(|| {
                format!(
                    "https://github.com/{}/{}/releases/tag/v{}",
                    self.owner, self.repo, latest_ver
                )
            });

            let platform = Platform::detect();
            let portable = cache
                .portable_asset_url
                .as_deref()
                .or(cache.asset_url.as_deref());
            let installer = cache.installer_asset_url.as_deref();
            let asset_url =
                choose_apply_plan(&platform, current_exe, portable, installer).map(|p| match p {
                    ApplyPlan::Portable { url } => url,
                    ApplyPlan::Installer { url, .. } => url,
                });
            UpdateState::Available {
                current: self.current_version.to_string(),
                latest: latest_ver.to_string(),
                release_url,
                asset_url,
                checked_at,
            }
        } else {
            UpdateState::UpToDate {
                checked_at: Some(checked_at),
            }
        }
    }
}

pub fn internal_apply_update(
    old_pid: u32,
    target_exe: &Path,
    source_exe: &Path,
    args_file: &Path,
) -> Result<(), String> {
    wait_for_pid_exit(old_pid, Duration::from_secs(300))?;
    let args = UpdateManager::read_restart_args_file(args_file)?;
    replace_bundle_executables(target_exe, source_exe)?;

    std::process::Command::new(target_exe)
        .args(args)
        .spawn()
        .map_err(|e| format!("Failed to restart: {e}"))?;
    Ok(())
}

pub fn internal_run_installer(
    old_pid: u32,
    target_exe: &Path,
    installer: &Path,
    installer_kind: InstallerKind,
    args_file: &Path,
) -> Result<(), String> {
    wait_for_pid_exit(old_pid, Duration::from_secs(300))?;

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let restart_exe = match installer_kind {
            InstallerKind::MacDmg => {
                #[cfg(target_os = "macos")]
                {
                    let target_app =
                        run_macos_dmg_installer_with_privileges(installer, target_exe)?;
                    app_bundle_executable_path(&target_app, target_exe)
                        .unwrap_or_else(|| target_exe.to_path_buf())
                }
                #[cfg(not(target_os = "macos"))]
                {
                    return Err("mac_dmg installer can only run on macOS".to_string());
                }
            }
            InstallerKind::MacPkg => {
                #[cfg(target_os = "macos")]
                {
                    run_macos_pkg_installer_with_privileges(installer)?;
                    resolve_macos_restart_executable(Path::new("/Applications"), target_exe, None)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    return Err("mac_pkg installer can only run on macOS".to_string());
                }
            }
            InstallerKind::WindowsMsi => {
                #[cfg(target_os = "windows")]
                {
                    run_windows_msi_with_uac(installer)?;
                    resolve_windows_restart_executable(target_exe)
                }
                #[cfg(not(target_os = "windows"))]
                {
                    return Err("windows_msi installer can only run on Windows".to_string());
                }
            }
        };

        let restart_exe = fs::canonicalize(&restart_exe).unwrap_or(restart_exe);
        let args = UpdateManager::read_restart_args_file(args_file)?;
        std::process::Command::new(&restart_exe)
            .args(args)
            .spawn()
            .map_err(|e| format!("Failed to restart: {e}"))?;
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (target_exe, installer, installer_kind, args_file);
        Err("installer updates are not supported on this platform".to_string())
    }
}

fn read_cache(path: &Path) -> Result<UpdateCacheFile, String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read update cache: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Failed to parse update cache: {e}"))
}

fn write_cache(path: &Path, cache: &UpdateCacheFile) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid update cache path".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("Failed to create cache dir: {e}"))?;
    write_json_atomic(path, cache).map_err(|e| format!("Failed to write update cache: {e}"))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("invalid path"))?;
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("update")
    ));
    let bytes = serde_json::to_vec(value).map_err(|e| io::Error::other(e.to_string()))?;
    fs::write(&tmp, bytes)?;
    let _ = fs::remove_file(path); // Remove existing file first (needed on Windows)
    fs::rename(&tmp, path)?;
    Ok(())
}

fn parse_tag_version(tag: &str) -> Option<Version> {
    let trimmed = tag.trim();
    let v = trimmed.strip_prefix('v').unwrap_or(trimmed);
    Version::parse(v).ok()
}

fn asset_name_from_url(url: &str) -> Option<String> {
    url.split('/')
        .next_back()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn find_installer_asset_url(platform: &Platform, assets: &[GitHubAsset]) -> Option<String> {
    match platform.os.as_str() {
        "macos" => {
            if let Some(asset_name) = crate::release_contract::installer_asset_name(&platform.os) {
                if let Some(asset) = assets
                    .iter()
                    .find(|a| a.name.eq_ignore_ascii_case(&asset_name))
                {
                    return Some(asset.browser_download_url.clone());
                }
            }

            // Legacy release flow: prefer any DMG for macOS.
            if let Some(asset) = assets.iter().find(|a| {
                let lower = a.name.to_ascii_lowercase();
                lower.ends_with(".dmg") && asset_matches_arch(&lower, &platform.arch)
            }) {
                return Some(asset.browser_download_url.clone());
            }

            // Legacy release flow fallback: signed PKG with old naming.
            if let Some(artifact) = platform.artifact() {
                let legacy_pkg_name = format!("gwt-{artifact}.pkg");
                if let Some(asset) = assets.iter().find(|a| a.name == legacy_pkg_name) {
                    return Some(asset.browser_download_url.clone());
                }
            }

            if let Some(asset) = assets
                .iter()
                .find(|a| a.name.to_ascii_lowercase().ends_with(".dmg"))
            {
                return Some(asset.browser_download_url.clone());
            }

            assets
                .iter()
                .find(|a| a.name.to_ascii_lowercase().ends_with(".pkg"))
                .map(|a| a.browser_download_url.clone())
        }
        "windows" => {
            if let Some(asset_name) = crate::release_contract::installer_asset_name(&platform.os) {
                if let Some(asset) = assets
                    .iter()
                    .find(|a| a.name.eq_ignore_ascii_case(&asset_name))
                {
                    return Some(asset.browser_download_url.clone());
                }
            }

            // Legacy naming fallback.
            if let Some(asset) = assets
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case("gwt-wix-windows-x86_64.msi"))
            {
                return Some(asset.browser_download_url.clone());
            }

            if let Some(asset) = assets
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case("gwt-windows-x86_64.msi"))
            {
                return Some(asset.browser_download_url.clone());
            }

            if let Some(asset) = assets.iter().find(|a| {
                let lower = a.name.to_ascii_lowercase();
                lower.ends_with(".msi") && asset_matches_arch(&lower, &platform.arch)
            }) {
                return Some(asset.browser_download_url.clone());
            }

            assets
                .iter()
                .find(|a| a.name.to_ascii_lowercase().ends_with(".msi"))
                .map(|a| a.browser_download_url.clone())
        }
        _ => None,
    }
}

fn asset_matches_arch(asset_name_lower: &str, arch: &str) -> bool {
    match arch {
        "aarch64" => asset_name_lower.contains("aarch64") || asset_name_lower.contains("arm64"),
        "x86_64" => {
            asset_name_lower.contains("x86_64")
                || asset_name_lower.contains("x64")
                || asset_name_lower.contains("amd64")
        }
        _ => true,
    }
}

fn installer_kind_for_url(platform: &Platform, installer_url: &str) -> Option<InstallerKind> {
    let lower = installer_url.to_ascii_lowercase();
    match platform.os.as_str() {
        "macos" if lower.ends_with(".dmg") => Some(InstallerKind::MacDmg),
        "macos" if lower.ends_with(".pkg") => Some(InstallerKind::MacPkg),
        "windows" if lower.ends_with(".msi") => Some(InstallerKind::WindowsMsi),
        _ => None,
    }
}

fn normalize_windows_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn windows_paths_equal(lhs: &Path, rhs: &Path) -> bool {
    normalize_windows_path(lhs) == normalize_windows_path(rhs)
}

fn windows_per_user_install_executable(target_exe: &Path) -> Option<PathBuf> {
    let file_name = target_exe.file_name()?;
    std::env::var_os("LOCALAPPDATA").map(|root| {
        PathBuf::from(root)
            .join("Programs")
            .join("GWT")
            .join(file_name)
    })
}

fn is_windows_per_user_install_executable(current_exe: &Path) -> bool {
    windows_per_user_install_executable(current_exe)
        .is_some_and(|expected| windows_paths_equal(&expected, current_exe))
}

fn is_windows_legacy_program_files_executable(current_exe: &Path) -> bool {
    let normalized = normalize_windows_path(current_exe);
    normalized.ends_with("\\gwt\\gwt.exe")
        && (normalized.contains("\\program files\\")
            || normalized.contains("\\program files (x86)\\"))
}

fn windows_should_prefer_installer(current_exe: &Path) -> bool {
    is_windows_per_user_install_executable(current_exe)
        || is_windows_legacy_program_files_executable(current_exe)
}

fn choose_apply_plan(
    platform: &Platform,
    current_exe: Option<&Path>,
    portable_url: Option<&str>,
    installer_url: Option<&str>,
) -> Option<ApplyPlan> {
    if platform.os == "windows" {
        if let (Some(current_exe), Some(url)) = (current_exe, installer_url) {
            if windows_should_prefer_installer(current_exe) {
                if let Some(kind) = installer_kind_for_url(platform, url) {
                    return Some(ApplyPlan::Installer {
                        url: url.to_string(),
                        kind,
                    });
                }
            }
        }
    }

    let running_from_app_bundle =
        current_exe.and_then(app_bundle_from_executable).is_some() || current_exe.is_none();
    let writable = current_exe
        .and_then(|p| p.parent())
        .and_then(|dir| is_dir_writable(dir).ok())
        .unwrap_or(true);

    choose_apply_plan_with_writable(
        platform,
        running_from_app_bundle,
        writable,
        portable_url,
        installer_url,
    )
}

fn choose_apply_plan_with_writable(
    platform: &Platform,
    running_from_app_bundle: bool,
    writable: bool,
    portable_url: Option<&str>,
    installer_url: Option<&str>,
) -> Option<ApplyPlan> {
    // macOS: prefer installer when available to preserve codesign/notarization integrity.
    if platform.os == "macos" {
        if running_from_app_bundle {
            if let Some(url) = installer_url {
                if let Some(kind) = installer_kind_for_url(platform, url) {
                    return Some(ApplyPlan::Installer {
                        url: url.to_string(),
                        kind,
                    });
                }
            }
        } else if writable {
            if let Some(url) = portable_url {
                return Some(ApplyPlan::Portable {
                    url: url.to_string(),
                });
            }
        }
    }

    // If we cannot replace in-place, prefer installer when available.
    if !writable {
        if let Some(url) = installer_url {
            if let Some(kind) = installer_kind_for_url(platform, url) {
                return Some(ApplyPlan::Installer {
                    url: url.to_string(),
                    kind,
                });
            }
        }
        return None;
    }

    if let Some(url) = portable_url {
        return Some(ApplyPlan::Portable {
            url: url.to_string(),
        });
    }

    if let Some(url) = installer_url {
        if let Some(kind) = installer_kind_for_url(platform, url) {
            return Some(ApplyPlan::Installer {
                url: url.to_string(),
                kind,
            });
        }
    }

    None
}

fn is_dir_writable(dir: &Path) -> Result<bool, String> {
    let _ = fs::create_dir_all(dir);
    let probe = dir.join(format!(".gwt_write_probe_{}", std::process::id()));
    let result = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map(|_| true)
        .or_else(|e| {
            if matches!(e.kind(), io::ErrorKind::PermissionDenied) {
                Ok(false)
            } else {
                Err(e)
            }
        })
        .map_err(|e| format!("Failed to probe dir writability: {e}"))?;
    if result {
        let _ = fs::remove_file(&probe);
    }
    Ok(result)
}

fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let name = archive_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();

    if name.ends_with(".tar.gz") {
        let file =
            fs::File::open(archive_path).map_err(|e| format!("Failed to open archive: {e}"))?;
        let decoder = GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(dest_dir)
            .map_err(|e| format!("Failed to unpack tar.gz: {e}"))?;
        return Ok(());
    }

    if name.ends_with(".zip") {
        let file =
            fs::File::open(archive_path).map_err(|e| format!("Failed to open archive: {e}"))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Failed to read zip: {e}"))?;
        zip.extract(dest_dir)
            .map_err(|e| format!("Failed to extract zip: {e}"))?;
        return Ok(());
    }

    Err(format!("Unsupported archive format: {name}"))
}

fn find_extracted_binary(extract_dir: &Path, binary_name: &str) -> Result<Option<PathBuf>, String> {
    // Expected layout: dist/gwt-<artifact>/<binary>
    let mut candidates = Vec::<PathBuf>::new();
    for entry in fs::read_dir(extract_dir).map_err(|e| format!("Failed to read dir: {e}"))? {
        let entry = entry.map_err(|e| format!("Failed to read dir entry: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            candidates.push(path.join(binary_name));
        } else if path.file_name().and_then(|n| n.to_str()) == Some(binary_name) {
            candidates.push(path);
        }
    }

    for c in candidates {
        if c.exists() {
            return Ok(Some(c));
        }
    }

    // Fallback: deep search.
    let mut stack = vec![extract_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        // Ignore unreadable directories and continue searching other paths.
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some(binary_name) {
                return Ok(Some(path));
            }
        }
    }

    Ok(None)
}

fn find_extracted_bundle_binaries(
    extract_dir: &Path,
    primary_binary_name: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let Some(primary_path) = find_extracted_binary(extract_dir, primary_binary_name)? else {
        return Err(format!(
            "Extracted payload does not contain expected binary: {primary_binary_name}"
        ));
    };

    let daemon_binary_name = companion_binary_name(primary_binary_name);
    let Some(daemon_path) = find_extracted_binary(extract_dir, &daemon_binary_name)? else {
        return Err(format!(
            "Extracted payload does not contain expected daemon binary: {daemon_binary_name}"
        ));
    };

    Ok((primary_path, daemon_path))
}

fn ensure_executable(_path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = _path;
        let mode = fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode())
            .unwrap_or(0o755);
        let mut perms = fs::metadata(path)
            .map_err(|e| format!("Failed to read metadata: {e}"))?
            .permissions();
        perms.set_mode(mode | 0o111);
        let _ = fs::set_permissions(path, perms);
    }
    Ok(())
}

fn wait_for_pid_exit(pid: u32, timeout: Duration) -> Result<(), String> {
    let started = std::time::Instant::now();
    while is_process_running(pid) {
        if started.elapsed() > timeout {
            return Err(format!("Timed out waiting for process {pid} to exit"));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

fn is_process_running(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
        );
        crate::process::hidden_command("powershell")
            .args(["-NoProfile", "-Command", &script])
            .status()
            .map(|s: std::process::ExitStatus| s.success())
            .unwrap_or(false)
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

#[cfg(any(test, target_os = "macos"))]
fn sh_single_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[cfg(target_os = "macos")]
fn escape_applescript_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\"', "\\\"")
}

#[cfg(target_os = "macos")]
fn run_shell_with_admin_privileges(shell_cmd: &str) -> Result<(), String> {
    let applescript_cmd = format!(
        "do shell script \"{}\" with administrator privileges",
        escape_applescript_string(shell_cmd)
    );
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(applescript_cmd)
        .status()
        .map_err(|e| format!("Failed to run privileged command via osascript: {e}"))?;
    if !status.success() {
        return Err(format!("osascript command exited with {status}"));
    }
    Ok(())
}

fn app_bundle_from_executable(target_exe: &Path) -> Option<PathBuf> {
    target_exe
        .ancestors()
        .find(|p| p.extension() == Some(OsStr::new("app")))
        .map(Path::to_path_buf)
}

#[cfg(any(test, target_os = "macos"))]
fn app_bundle_executable_path(app_bundle: &Path, target_exe: &Path) -> Option<PathBuf> {
    let contents_dir = app_bundle.join("Contents").join("MacOS");
    if let Some(file_name) = target_exe.file_name() {
        let candidate = contents_dir.join(file_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let mut entries = fs::read_dir(&contents_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    entries.sort();
    entries.into_iter().next()
}

#[cfg(any(test, target_os = "macos"))]
fn find_matching_app_bundle(root: &Path, target_exe: &Path) -> Option<PathBuf> {
    let expected_name = target_exe.file_name()?;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.extension() == Some(OsStr::new("app")) {
                if path
                    .join("Contents")
                    .join("MacOS")
                    .join(expected_name)
                    .exists()
                {
                    return Some(path);
                }
                continue;
            }
            stack.push(path);
        }
    }
    None
}

#[cfg(any(test, target_os = "macos"))]
fn resolve_macos_restart_executable(
    applications_dir: &Path,
    target_exe: &Path,
    preferred_bundle_name: Option<&OsStr>,
) -> PathBuf {
    if let Some(bundle_name) = preferred_bundle_name {
        let preferred_bundle = applications_dir.join(bundle_name);
        if let Some(restart_exe) = app_bundle_executable_path(&preferred_bundle, target_exe) {
            return restart_exe;
        }
    }
    if let Some(bundle) = find_matching_app_bundle(applications_dir, target_exe) {
        if let Some(restart_exe) = app_bundle_executable_path(&bundle, target_exe) {
            return restart_exe;
        }
    }
    target_exe.to_path_buf()
}

#[cfg(any(test, target_os = "windows"))]
fn resolve_windows_restart_executable(target_exe: &Path) -> PathBuf {
    if let Some(preferred) = windows_per_user_install_executable(target_exe) {
        if preferred.exists() {
            return preferred;
        }
    }
    target_exe.to_path_buf()
}

#[cfg(any(test, target_os = "macos"))]
fn build_macos_dmg_install_shell_cmd(
    source_app: &Path,
    target_app: &Path,
    temp_app: &Path,
    backup_app: &Path,
) -> String {
    let source = sh_single_quote(&source_app.to_string_lossy());
    let target = sh_single_quote(&target_app.to_string_lossy());
    let temp = sh_single_quote(&temp_app.to_string_lossy());
    let backup = sh_single_quote(&backup_app.to_string_lossy());
    format!(
        "set -e\n\
         /bin/rm -rf {temp} {backup}\n\
         /usr/bin/ditto {source} {temp}\n\
         if [ -e {target} ]; then\n\
           /bin/mv {target} {backup}\n\
         fi\n\
         if ! /bin/mv {temp} {target}; then\n\
           if [ -e {backup} ] && [ ! -e {target} ]; then\n\
             /bin/mv {backup} {target} || true\n\
           fi\n\
           /bin/rm -rf {temp}\n\
           exit 1\n\
         fi\n\
         /bin/rm -rf {backup}"
    )
}

#[cfg(target_os = "macos")]
fn macos_swap_bundle_paths(target_app: &Path) -> Result<(PathBuf, PathBuf), String> {
    let parent = target_app
        .parent()
        .ok_or_else(|| "Target app bundle path has no parent dir".to_string())?;
    let stem = target_app
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("gwt");
    let pid = std::process::id();
    let temp_app = parent.join(format!(".{stem}.gwt-update-{pid}.new.app"));
    let backup_app = parent.join(format!(".{stem}.gwt-update-{pid}.old.app"));
    Ok((temp_app, backup_app))
}

#[cfg(target_os = "macos")]
fn find_first_app_bundle(root: &Path) -> Result<Option<PathBuf>, String> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| format!("Failed to read dir: {e}"))? {
            let entry = entry.map_err(|e| format!("Failed to read dir entry: {e}"))?;
            let path = entry.path();
            if path.is_dir() {
                if path.extension() == Some(OsStr::new("app")) {
                    return Ok(Some(path));
                }
                stack.push(path);
            }
        }
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
fn run_macos_pkg_installer_with_privileges(installer: &Path) -> Result<(), String> {
    let installer_path = installer.to_string_lossy().to_string();
    let shell_cmd = format!(
        "/usr/sbin/installer -pkg {} -target /",
        sh_single_quote(&installer_path)
    );
    run_shell_with_admin_privileges(&shell_cmd)
}

#[cfg(target_os = "macos")]
fn run_macos_dmg_installer_with_privileges(
    installer: &Path,
    target_exe: &Path,
) -> Result<PathBuf, String> {
    let mount_dir = std::env::temp_dir().join(format!("gwt-update-dmg-{}", std::process::id()));
    let _ = fs::remove_dir_all(&mount_dir);
    fs::create_dir_all(&mount_dir).map_err(|e| format!("Failed to create mount dir: {e}"))?;

    let attach_status = std::process::Command::new("hdiutil")
        .arg("attach")
        .arg(installer)
        .arg("-nobrowse")
        .arg("-readonly")
        .arg("-mountpoint")
        .arg(&mount_dir)
        .status()
        .map_err(|e| format!("Failed to mount dmg: {e}"))?;
    if !attach_status.success() {
        let _ = fs::remove_dir_all(&mount_dir);
        return Err(format!("hdiutil attach exited with {attach_status}"));
    }

    let install_result: Result<PathBuf, String> = (|| {
        let source_app = find_first_app_bundle(&mount_dir)?
            .ok_or_else(|| "Mounted dmg does not contain an .app bundle".to_string())?;
        let source_name = source_app
            .file_name()
            .ok_or_else(|| "Mounted app bundle has an invalid name".to_string())?;
        let target_app = app_bundle_from_executable(target_exe)
            .unwrap_or_else(|| PathBuf::from("/Applications").join(source_name));
        let (temp_app, backup_app) = macos_swap_bundle_paths(&target_app)?;

        let shell_cmd =
            build_macos_dmg_install_shell_cmd(&source_app, &target_app, &temp_app, &backup_app);
        run_shell_with_admin_privileges(&shell_cmd)?;
        Ok(target_app)
    })();

    let detach_status = std::process::Command::new("hdiutil")
        .arg("detach")
        .arg(&mount_dir)
        .arg("-force")
        .status()
        .map_err(|e| format!("Failed to unmount dmg: {e}"))?;
    let _ = fs::remove_dir_all(&mount_dir);

    let target_app = install_result?;
    if !detach_status.success() {
        return Err(format!("hdiutil detach exited with {detach_status}"));
    }
    Ok(target_app)
}

#[cfg(any(test, target_os = "windows"))]
fn windows_msi_argument_list(installer: &Path) -> Vec<String> {
    vec![
        "/i".to_string(),
        installer.to_string_lossy().to_string(),
        "/passive".to_string(),
        "GWT_ALLOW_LEGACY_MIGRATION=1".to_string(),
    ]
}

#[cfg(target_os = "windows")]
fn run_windows_msi_with_uac(installer: &Path) -> Result<(), String> {
    // Trigger UAC for msiexec via PowerShell.
    let arg_list = windows_msi_argument_list(installer)
        .into_iter()
        .map(|arg| format!("'{}'", arg.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let args = format!("Start-Process msiexec.exe -Verb RunAs -Wait -ArgumentList @({arg_list})");
    let status = std::process::Command::new("powershell")
        .arg("-NoProfile")
        .arg("-Command")
        .arg(args)
        .status()
        .map_err(|e| format!("Failed to run msiexec: {e}"))?;
    if !status.success() {
        return Err(format!("msiexec exited with {status}"));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct PlannedReplacement {
    target: PathBuf,
    backup: PathBuf,
    tmp: PathBuf,
    had_target: bool,
}

fn replace_bundle_executables(target_exe: &Path, source_exe: &Path) -> Result<(), String> {
    let target_daemon = companion_binary_path(target_exe)?;
    let source_daemon = companion_binary_path(source_exe)?;
    replace_executables_with_retry(&[
        (target_daemon.as_path(), source_daemon.as_path()),
        (target_exe, source_exe),
    ])
}

fn replace_executables_with_retry(pairs: &[(&Path, &Path)]) -> Result<(), String> {
    // Windows: file replacement can fail while the parent app is still shutting down.
    const MAX_RETRIES: usize = 200;
    const SLEEP_MS: u64 = 50;

    for attempt in 0..MAX_RETRIES {
        let replacements = stage_replacement_plan(pairs)?;
        match apply_replacement_plan(&replacements) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt + 1 == MAX_RETRIES {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(SLEEP_MS));
            }
        }
    }

    Err("Failed to replace executable".to_string())
}

fn companion_binary_name(primary_binary_name: &str) -> String {
    if primary_binary_name.ends_with(".exe") {
        "gwtd.exe".to_string()
    } else {
        "gwtd".to_string()
    }
}

fn companion_binary_path(primary_binary_path: &Path) -> Result<PathBuf, String> {
    let file_name = primary_binary_path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| "Target executable has invalid filename".to_string())?;
    Ok(primary_binary_path.with_file_name(companion_binary_name(file_name)))
}

fn stage_replacement(target_exe: &Path, source_exe: &Path) -> Result<PlannedReplacement, String> {
    let source_meta = fs::metadata(source_exe).map_err(|e| format!("Source missing: {e}"))?;
    if source_meta.len() == 0 {
        return Err(format!(
            "Source executable is empty: {}",
            source_exe.display()
        ));
    }

    let target_dir = target_exe
        .parent()
        .ok_or_else(|| "Target executable path has no parent dir".to_string())?;
    fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to ensure target dir exists: {e}"))?;

    let file_name = target_exe
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| "Target executable has invalid filename".to_string())?;
    let tmp_name = format!(".{file_name}.gwt-update-{}.tmp", std::process::id());
    let tmp_path = target_dir.join(tmp_name);
    let _ = fs::remove_file(&tmp_path);
    fs::copy(source_exe, &tmp_path).map_err(|e| format!("Failed to copy new executable: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(target_exe)
            .ok()
            .map(|m| m.permissions().mode())
            .unwrap_or(0o755);
        let mut perms = fs::metadata(&tmp_path)
            .map_err(|e| format!("Failed to read tmp metadata: {e}"))?
            .permissions();
        perms.set_mode(mode | 0o111);
        let _ = fs::set_permissions(&tmp_path, perms);
    }

    let backup_path = target_dir.join(format!("{file_name}.old"));
    let _ = fs::remove_file(&backup_path);

    Ok(PlannedReplacement {
        target: target_exe.to_path_buf(),
        backup: backup_path,
        tmp: tmp_path,
        had_target: target_exe.exists(),
    })
}

fn stage_replacement_plan(pairs: &[(&Path, &Path)]) -> Result<Vec<PlannedReplacement>, String> {
    let mut staged = Vec::with_capacity(pairs.len());
    for (target, source) in pairs {
        match stage_replacement(target, source) {
            Ok(replacement) => staged.push(replacement),
            Err(err) => {
                cleanup_staged_replacements(&staged);
                return Err(err);
            }
        }
    }
    Ok(staged)
}

fn cleanup_staged_replacements(replacements: &[PlannedReplacement]) {
    for replacement in replacements {
        let _ = fs::remove_file(&replacement.tmp);
    }
}

fn rollback_applied_replacements(replacements: &[PlannedReplacement]) {
    for replacement in replacements.iter().rev() {
        let _ = fs::remove_file(&replacement.target);
        if replacement.had_target && replacement.backup.exists() {
            let _ = fs::rename(&replacement.backup, &replacement.target);
        }
    }
}

fn apply_replacement_plan(replacements: &[PlannedReplacement]) -> Result<(), String> {
    apply_replacement_plan_with(replacements, replace_paths)
}

fn apply_replacement_plan_with<F>(
    replacements: &[PlannedReplacement],
    mut replace_one: F,
) -> Result<(), String>
where
    F: FnMut(&Path, &Path, &Path) -> io::Result<()>,
{
    for (idx, replacement) in replacements.iter().enumerate() {
        if let Err(err) = replace_one(&replacement.target, &replacement.backup, &replacement.tmp) {
            let _ = fs::remove_file(&replacement.tmp);
            cleanup_staged_replacements(&replacements[idx + 1..]);
            rollback_applied_replacements(&replacements[..idx]);
            return Err(format!("Failed to replace executable: {err}"));
        }
    }
    Ok(())
}

fn replace_paths(target_exe: &Path, backup_path: &Path, tmp_path: &Path) -> io::Result<()> {
    let had_target = target_exe.exists();
    if had_target {
        let _ = fs::remove_file(backup_path);
        fs::rename(target_exe, backup_path)?;
    }
    if let Err(err) = fs::rename(tmp_path, target_exe) {
        // Roll back: restore the original so the app is not left without an executable.
        if had_target && !target_exe.exists() && backup_path.exists() {
            let _ = fs::rename(backup_path, target_exe);
        }
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use std::{
        io::{Cursor, Read, Write},
        net::TcpListener,
        path::Path,
        sync::Mutex,
        thread,
    };

    use super::*;

    #[cfg(target_os = "windows")]
    use std::time::Duration as StdDuration;

    #[cfg(test)]
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn serve_once(path: &str, status: &str, content_type: &str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let path = path.trim_start_matches('/').to_string();
        let status = status.to_string();
        let content_type = content_type.to_string();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0u8; 2048];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            stream.write_all(&body).expect("write body");
        });
        format!("http://{addr}/{path}")
    }

    fn zip_body(file_name: &str, contents: &[u8]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        writer
            .start_file(file_name, zip::write::FileOptions::<()>::default())
            .expect("start zip file");
        writer.write_all(contents).expect("write zip body");
        let daemon_name = companion_binary_name(file_name);
        writer
            .start_file(daemon_name, zip::write::FileOptions::<()>::default())
            .expect("start daemon zip file");
        writer
            .write_all(b"zip-daemon")
            .expect("write daemon zip body");
        writer.finish().expect("finish zip").into_inner()
    }

    fn tar_gz_bundle_body(primary: &[u8], daemon: &[u8]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let encoder = flate2::write::GzEncoder::new(cursor, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);

        let mut header = tar::Header::new_gnu();
        header.set_size(primary.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, "dist/gwt-linux-x86_64/gwt", primary)
            .expect("append gwt");

        let mut daemon_header = tar::Header::new_gnu();
        daemon_header.set_size(daemon.len() as u64);
        daemon_header.set_mode(0o755);
        daemon_header.set_cksum();
        archive
            .append_data(&mut daemon_header, "dist/gwt-linux-x86_64/gwtd", daemon)
            .expect("append gwtd");

        archive
            .into_inner()
            .expect("finish tar")
            .finish()
            .expect("finish gzip")
            .into_inner()
    }

    #[test]
    fn parse_tag_version_accepts_v_prefix() {
        assert_eq!(parse_tag_version("v1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_tag_version("1.2.3"), Some(Version::new(1, 2, 3)));
    }

    #[test]
    fn asset_name_from_url_extracts_filename() {
        assert_eq!(
            asset_name_from_url("https://example.com/a/b/c.exe"),
            Some("c.exe".to_string())
        );
    }

    #[test]
    fn check_uses_fresh_cache_without_network() {
        let temp = tempfile::tempdir().unwrap();
        let mgr = UpdateManager::new()
            .with_api_base_url("http://127.0.0.1:9")
            .with_cache_path(temp.path().join("update-check.json"))
            .with_updates_dir(temp.path().join("updates"));

        let cache = UpdateCacheFile {
            checked_at: Utc::now(),
            latest_version: Some("999.0.0".to_string()),
            release_url: Some("https://example.com/release".to_string()),
            portable_asset_url: None,
            installer_asset_url: None,
            asset_url: Some("https://example.com/asset".to_string()),
        };
        write_cache(mgr.cache_path(), &cache).unwrap();

        let state = mgr.check(false);
        match state {
            UpdateState::Available { latest, .. } => assert_eq!(latest, "999.0.0"),
            _ => panic!("expected Available from cache, got {state:?}"),
        }
    }

    #[test]
    fn force_check_bypasses_cache_and_can_fail() {
        let temp = tempfile::tempdir().unwrap();
        let mgr = UpdateManager::new()
            .with_api_base_url("http://127.0.0.1:9")
            .with_cache_path(temp.path().join("update-check.json"))
            .with_updates_dir(temp.path().join("updates"));

        let cache = UpdateCacheFile {
            checked_at: Utc::now(),
            latest_version: Some("999.0.0".to_string()),
            release_url: Some("https://example.com/release".to_string()),
            portable_asset_url: None,
            installer_asset_url: None,
            asset_url: Some("https://example.com/asset".to_string()),
        };
        write_cache(mgr.cache_path(), &cache).unwrap();

        let state = mgr.check(true);
        assert!(matches!(state, UpdateState::Failed { .. }));
    }

    #[test]
    fn check_fetches_release_and_caches_available_or_invalid_tags() {
        let temp = tempfile::tempdir().unwrap();
        let platform = Platform::detect();
        let portable_name = platform
            .portable_asset_name()
            .unwrap_or_else(|| "gwt-linux-x86_64.tar.gz".to_string());
        let portable_url = "https://example.com/downloads/portable";
        let release_body = format!(
            r#"{{
  "tag_name": "v99.0.0",
  "html_url": "https://github.com/akiojin/gwt/releases/tag/v99.0.0",
  "assets": [
    {{
      "name": "{portable_name}",
      "browser_download_url": "{portable_url}"
    }}
  ]
}}"#
        );
        let base_url = serve_once(
            "/repos/akiojin/gwt/releases/latest",
            "200 OK",
            "application/json",
            release_body.into_bytes(),
        );
        let mgr = UpdateManager::new()
            .with_api_base_url(base_url)
            .with_cache_path(temp.path().join("update-check.json"))
            .with_updates_dir(temp.path().join("updates"));

        let state = mgr.check(true);
        match state {
            UpdateState::Available {
                latest,
                release_url,
                asset_url,
                ..
            } => {
                assert_eq!(latest, "99.0.0");
                assert_eq!(
                    release_url,
                    "https://github.com/akiojin/gwt/releases/tag/v99.0.0"
                );
                assert_eq!(asset_url.as_deref(), Some(portable_url));
            }
            other => panic!("expected available update, got {other:?}"),
        }

        let cached = read_cache(mgr.cache_path()).expect("cache");
        assert_eq!(cached.latest_version.as_deref(), Some("99.0.0"));
        assert_eq!(cached.asset_url.as_deref(), Some(portable_url));

        let bad_base_url = serve_once(
            "/repos/akiojin/gwt/releases/latest",
            "200 OK",
            "application/json",
            br#"{"tag_name":"not-a-semver","html_url":"https://example.com/release","assets":[]}"#
                .to_vec(),
        );
        let bad_mgr = UpdateManager::new()
            .with_api_base_url(bad_base_url)
            .with_cache_path(temp.path().join("bad-update-check.json"))
            .with_updates_dir(temp.path().join("bad-updates"));
        let failed = bad_mgr.check(true);
        assert!(matches!(
            failed,
            UpdateState::Failed { ref message, .. }
                if message.contains("Failed to parse release tag as version")
        ));
    }

    #[test]
    fn prepare_update_handles_direct_binary_archives_installers_and_errors() {
        let temp = tempfile::tempdir().unwrap();
        let mgr = UpdateManager::new()
            .with_cache_path(temp.path().join("update-check.json"))
            .with_updates_dir(temp.path().join("updates"));

        let direct_url = serve_once(
            "/gwt.bin",
            "200 OK",
            "application/octet-stream",
            b"bin".to_vec(),
        );
        let direct = mgr
            .prepare_update("99.0.0", &direct_url)
            .expect("direct binary");
        match direct {
            PreparedPayload::PortableBinary { path } => {
                assert_eq!(fs::read(path).unwrap(), b"bin");
            }
            other => panic!("expected direct binary payload, got {other:?}"),
        }

        let binary_name = Platform::detect().binary_name();
        let archive_url = serve_once(
            "/gwt.zip",
            "200 OK",
            "application/zip",
            zip_body(&binary_name, b"zip-bin"),
        );
        let archive = mgr.prepare_update("99.0.1", &archive_url).expect("archive");
        match archive {
            PreparedPayload::PortableBinary { path } => {
                assert_eq!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some(binary_name.as_str())
                );
                assert_eq!(fs::read(path).unwrap(), b"zip-bin");
            }
            other => panic!("expected extracted archive payload, got {other:?}"),
        }

        let installer_url = serve_once(
            "/gwt.pkg",
            "200 OK",
            "application/octet-stream",
            b"pkg".to_vec(),
        );
        let installer = mgr
            .prepare_update("99.0.2", &installer_url)
            .expect("installer");
        assert_eq!(
            installer,
            PreparedPayload::Installer {
                path: temp.path().join("updates").join("v99.0.2").join("gwt.pkg"),
                kind: InstallerKind::MacPkg,
            }
        );

        let empty_url = serve_once(
            "/empty.bin",
            "200 OK",
            "application/octet-stream",
            Vec::new(),
        );
        let empty_err = mgr.prepare_update("99.0.3", &empty_url).unwrap_err();
        assert!(empty_err.contains("Downloaded payload is empty"));

        let status_err_url = serve_once(
            "/missing.bin",
            "404 Not Found",
            "text/plain",
            b"missing".to_vec(),
        );
        let status_err = mgr.prepare_update("99.0.4", &status_err_url).unwrap_err();
        assert!(status_err.contains("Download failed with status 404"));
    }

    #[test]
    fn restart_args_and_helper_process_helpers_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let mgr = UpdateManager::new()
            .with_cache_path(temp.path().join("update-check.json"))
            .with_updates_dir(temp.path().join("updates"));

        let args_path = temp.path().join("restart").join("args.json");
        mgr.write_restart_args_file(&args_path, vec!["--version".to_string()])
            .expect("write restart args");
        assert_eq!(
            UpdateManager::read_restart_args_file(&args_path).expect("read restart args"),
            vec!["--version".to_string()]
        );

        let current_exe = temp.path().join("gwt-current.exe");
        fs::write(&current_exe, b"binary").unwrap();
        let helper_copy = mgr
            .make_helper_copy(&current_exe, "99.1.0")
            .expect("helper copy");
        assert_eq!(fs::read(&helper_copy).unwrap(), b"binary");

        mgr.spawn_internal_apply_update(
            Path::new("git"),
            999_999,
            Path::new("target.exe"),
            Path::new("source.exe"),
            &args_path,
        )
        .expect("spawn apply helper");
        mgr.spawn_internal_run_installer(
            Path::new("git"),
            999_999,
            Path::new("target.exe"),
            Path::new("installer.pkg"),
            InstallerKind::MacPkg,
            &args_path,
        )
        .expect("spawn installer helper");
    }

    #[test]
    fn choose_apply_plan_prefers_macos_dmg_installer() {
        let platform = Platform {
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
        };

        let plan = choose_apply_plan(
            &platform,
            None,
            Some("https://example.com/gwt-macos-arm64.tar.gz"),
            Some("https://example.com/gwt_7.1.0_aarch64.dmg"),
        );

        assert_eq!(
            plan,
            Some(ApplyPlan::Installer {
                url: "https://example.com/gwt_7.1.0_aarch64.dmg".to_string(),
                kind: InstallerKind::MacDmg,
            })
        );
    }

    #[test]
    fn portable_asset_name_matches_release_contract() {
        let linux_aarch64 = Platform {
            os: "linux".to_string(),
            arch: "aarch64".to_string(),
        };
        let macos_arm64 = Platform {
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
        };
        let windows_x64 = Platform {
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
        };

        assert_eq!(
            linux_aarch64.portable_asset_name().as_deref(),
            Some("gwt-linux-aarch64.tar.gz")
        );
        assert_eq!(
            macos_arm64.portable_asset_name().as_deref(),
            Some("gwt-macos-arm64.tar.gz")
        );
        assert_eq!(
            windows_x64.portable_asset_name().as_deref(),
            Some("gwt-windows-x86_64.zip")
        );
    }

    #[test]
    fn shared_release_contract_exposes_current_stable_bundle_assets() {
        assert_eq!(
            crate::release_contract::portable_asset_name("windows", "x86_64").as_deref(),
            Some("gwt-windows-x86_64.zip")
        );
        assert_eq!(
            crate::release_contract::installer_asset_name("windows").as_deref(),
            Some("gwt-windows-x86_64.msi")
        );
        assert_eq!(
            crate::release_contract::bundle_binary_names("windows").expect("bundle binaries"),
            vec!["gwt.exe".to_string(), "gwtd.exe".to_string()]
        );
    }

    #[test]
    fn docker_linux_bundle_asset_name_normalizes_common_arch_aliases() {
        assert_eq!(
            docker_linux_bundle_asset_name("amd64").as_deref(),
            Ok("gwt-linux-x86_64.tar.gz")
        );
        assert_eq!(
            docker_linux_bundle_asset_name("arm64/v8").as_deref(),
            Ok("gwt-linux-aarch64.tar.gz")
        );
        assert!(docker_linux_bundle_asset_name("ppc64le").is_err());
    }

    #[test]
    fn choose_apply_plan_prefers_portable_for_macos_cli_install() {
        let temp = tempfile::tempdir().unwrap();
        let current_exe = temp.path().join("gwt");
        let platform = Platform {
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
        };

        let plan = choose_apply_plan(
            &platform,
            Some(&current_exe),
            Some("https://example.com/gwt-macos-arm64.tar.gz"),
            Some("https://example.com/gwt_7.1.0_aarch64.dmg"),
        );

        assert_eq!(
            plan,
            Some(ApplyPlan::Portable {
                url: "https://example.com/gwt-macos-arm64.tar.gz".to_string(),
            })
        );
    }

    #[test]
    fn choose_apply_plan_falls_back_to_installer_for_nonwritable_macos_cli_install() {
        let platform = Platform {
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
        };

        let plan = choose_apply_plan_with_writable(
            &platform,
            false,
            false,
            Some("https://example.com/gwt-macos-arm64.tar.gz"),
            Some("https://example.com/gwt_7.1.0_aarch64.dmg"),
        );

        assert_eq!(
            plan,
            Some(ApplyPlan::Installer {
                url: "https://example.com/gwt_7.1.0_aarch64.dmg".to_string(),
                kind: InstallerKind::MacDmg,
            })
        );
    }

    #[test]
    fn choose_apply_plan_prefers_installer_for_windows_per_user_msi_install() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let local_app_data = temp.path().join("AppData").join("Local");
        let current_exe = local_app_data.join("Programs").join("GWT").join("gwt.exe");
        fs::create_dir_all(current_exe.parent().unwrap()).unwrap();

        let old_local_app_data = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &local_app_data);

        let plan = choose_apply_plan(
            &Platform {
                os: "windows".to_string(),
                arch: "x86_64".to_string(),
            },
            Some(&current_exe),
            Some("https://example.com/gwt-windows-x86_64.zip"),
            Some("https://example.com/gwt-windows-x86_64.msi"),
        );

        match old_local_app_data {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }

        assert_eq!(
            plan,
            Some(ApplyPlan::Installer {
                url: "https://example.com/gwt-windows-x86_64.msi".to_string(),
                kind: InstallerKind::WindowsMsi,
            })
        );
    }

    #[test]
    fn choose_apply_plan_windows_installer_preference_falls_back_to_portable_when_installer_url_is_unsupported(
    ) {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let local_app_data = temp.path().join("AppData").join("Local");
        let current_exe = local_app_data.join("Programs").join("GWT").join("gwt.exe");
        fs::create_dir_all(current_exe.parent().unwrap()).unwrap();

        let old_local_app_data = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &local_app_data);

        let plan = choose_apply_plan(
            &Platform {
                os: "windows".to_string(),
                arch: "x86_64".to_string(),
            },
            Some(&current_exe),
            Some("https://example.com/gwt-windows-x86_64.zip"),
            Some("https://example.com/gwt_7.1.0_aarch64.dmg"),
        );

        match old_local_app_data {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }

        assert_eq!(
            plan,
            Some(ApplyPlan::Portable {
                url: "https://example.com/gwt-windows-x86_64.zip".to_string(),
            })
        );
    }

    #[test]
    fn choose_apply_plan_prefers_installer_for_legacy_program_files_windows_install() {
        let temp = tempfile::tempdir().unwrap();
        let current_exe = temp
            .path()
            .join("Program Files")
            .join("GWT")
            .join("gwt.exe");
        fs::create_dir_all(current_exe.parent().unwrap()).unwrap();

        let plan = choose_apply_plan(
            &Platform {
                os: "windows".to_string(),
                arch: "x86_64".to_string(),
            },
            Some(&current_exe),
            Some("https://example.com/gwt-windows-x86_64.zip"),
            Some("https://example.com/gwt-windows-x86_64.msi"),
        );

        assert_eq!(
            plan,
            Some(ApplyPlan::Installer {
                url: "https://example.com/gwt-windows-x86_64.msi".to_string(),
                kind: InstallerKind::WindowsMsi,
            })
        );
    }

    #[test]
    fn resolve_macos_restart_executable_scans_applications_for_matching_binary() {
        let temp = tempfile::tempdir().unwrap();
        let other_bundle = temp.path().join("Other.app").join("Contents").join("MacOS");
        fs::create_dir_all(&other_bundle).unwrap();
        fs::write(other_bundle.join("other"), b"bin").unwrap();

        let gwt_bundle = temp.path().join("GWT.app").join("Contents").join("MacOS");
        fs::create_dir_all(&gwt_bundle).unwrap();
        fs::write(gwt_bundle.join("gwt"), b"bin").unwrap();

        let restart_exe =
            resolve_macos_restart_executable(temp.path(), Path::new("/usr/local/bin/gwt"), None);

        assert_eq!(restart_exe, gwt_bundle.join("gwt"));
    }

    #[test]
    fn resolve_windows_restart_executable_prefers_per_user_install_after_migration() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let local_app_data = temp.path().join("AppData").join("Local");
        let migrated_exe = local_app_data.join("Programs").join("GWT").join("gwt.exe");
        fs::create_dir_all(migrated_exe.parent().unwrap()).unwrap();
        fs::write(&migrated_exe, b"new-binary").unwrap();

        let old_local_app_data = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &local_app_data);

        let restart_exe = resolve_windows_restart_executable(
            &temp
                .path()
                .join("Program Files")
                .join("GWT")
                .join("gwt.exe"),
        );

        match old_local_app_data {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }

        assert_eq!(restart_exe, migrated_exe);
    }

    #[test]
    fn windows_msi_argument_list_allows_legacy_migration() {
        assert_eq!(
            windows_msi_argument_list(Path::new("C:/temp/gwt-windows-x86_64.msi")),
            vec![
                "/i".to_string(),
                "C:/temp/gwt-windows-x86_64.msi".to_string(),
                "/passive".to_string(),
                "GWT_ALLOW_LEGACY_MIGRATION=1".to_string(),
            ]
        );
    }

    #[test]
    fn build_macos_dmg_install_shell_cmd_swaps_after_successful_copy() {
        let script = build_macos_dmg_install_shell_cmd(
            Path::new("/Volumes/GWT/GWT.app"),
            Path::new("/Applications/GWT.app"),
            Path::new("/Applications/.gwt-update-new.app"),
            Path::new("/Applications/.gwt-update-old.app"),
        );

        assert!(script.contains("ditto '/Volumes/GWT/GWT.app' '/Applications/.gwt-update-new.app'"));
        assert!(script.contains("mv '/Applications/GWT.app' '/Applications/.gwt-update-old.app'"));
        assert!(script.contains("mv '/Applications/.gwt-update-new.app' '/Applications/GWT.app'"));
        assert!(!script.contains("rm -rf '/Applications/GWT.app'"));
    }

    #[test]
    fn find_installer_asset_url_prefers_contract_windows_msi() {
        let platform = Platform {
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
        };
        let assets = vec![
            GitHubAsset {
                name: "gwt_7.1.0_x64_en-US.msi".to_string(),
                browser_download_url: "https://example.com/tauri.msi".to_string(),
            },
            GitHubAsset {
                name: "gwt-wix-windows-x86_64.msi".to_string(),
                browser_download_url: "https://example.com/wix.msi".to_string(),
            },
            GitHubAsset {
                name: "gwt-windows-x86_64.msi".to_string(),
                browser_download_url: "https://example.com/current.msi".to_string(),
            },
        ];

        let url = find_installer_asset_url(&platform, &assets);
        assert_eq!(url.as_deref(), Some("https://example.com/current.msi"));
    }

    #[test]
    fn find_installer_asset_url_prefers_macos_arch_specific_dmg() {
        let platform = Platform {
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
        };
        let assets = vec![
            GitHubAsset {
                name: "gwt_7.1.0_x64.dmg".to_string(),
                browser_download_url: "https://example.com/macos-x64.dmg".to_string(),
            },
            GitHubAsset {
                name: "gwt_7.1.0_aarch64.dmg".to_string(),
                browser_download_url: "https://example.com/macos-arm64.dmg".to_string(),
            },
        ];

        let url = find_installer_asset_url(&platform, &assets);
        assert_eq!(url.as_deref(), Some("https://example.com/macos-arm64.dmg"));
    }

    #[test]
    fn find_extracted_bundle_binaries_requires_primary_and_daemon() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("gwt"), b"bundle-gwt").unwrap();

        let err = find_extracted_bundle_binaries(temp.path(), "gwt")
            .expect_err("bundle without gwtd companion must fail");
        assert!(
            err.contains("gwtd"),
            "missing daemon companion must be named in the error, got: {err}"
        );
    }

    #[test]
    fn apply_replacement_plan_rolls_back_when_later_bundle_swap_fails() {
        let temp = tempfile::tempdir().unwrap();
        let install_dir = temp.path().join("install");
        let bundle_dir = temp.path().join("bundle");
        fs::create_dir_all(&install_dir).unwrap();
        fs::create_dir_all(&bundle_dir).unwrap();

        let target_gwt = install_dir.join("gwt");
        let target_gwtd = install_dir.join("gwtd");
        let source_gwt = bundle_dir.join("gwt");
        let source_gwtd = bundle_dir.join("gwtd");

        fs::write(&target_gwt, b"old-gwt").unwrap();
        fs::write(&target_gwtd, b"old-gwtd").unwrap();
        fs::write(&source_gwt, b"new-gwt").unwrap();
        fs::write(&source_gwtd, b"new-gwtd").unwrap();

        let replacements = stage_replacement_plan(&[
            (target_gwt.as_path(), source_gwt.as_path()),
            (target_gwtd.as_path(), source_gwtd.as_path()),
        ])
        .expect("stage bundle replacements");

        let mut calls = 0usize;
        let err = apply_replacement_plan_with(&replacements, |target, backup, tmp| {
            calls += 1;
            if calls == 1 {
                replace_paths(target, backup, tmp)
            } else {
                Err(io::Error::other("simulated second swap failure"))
            }
        })
        .expect_err("second replacement failure must bubble up");

        assert!(
            err.contains("simulated second swap failure"),
            "expected injected failure, got: {err}"
        );
        assert_eq!(fs::read_to_string(&target_gwt).unwrap(), "old-gwt");
        assert_eq!(fs::read_to_string(&target_gwtd).unwrap(), "old-gwtd");
    }

    #[cfg(unix)]
    #[test]
    fn find_extracted_binary_skips_unreadable_dirs() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let readable = root.join("readable");
        let nested = readable.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let expected = nested.join("gwt");
        fs::write(&expected, b"bin").unwrap();

        let unreadable = root.join("unreadable");
        fs::create_dir_all(&unreadable).unwrap();
        let mut unreadable_perms = fs::metadata(&unreadable).unwrap().permissions();
        unreadable_perms.set_mode(0o000);
        fs::set_permissions(&unreadable, unreadable_perms).unwrap();

        let found = find_extracted_binary(root, "gwt");

        // Restore permissions so tempfile cleanup can remove the directory.
        let mut restore_perms = fs::metadata(&unreadable).unwrap().permissions();
        restore_perms.set_mode(0o755);
        let _ = fs::set_permissions(&unreadable, restore_perms);

        assert_eq!(found.unwrap(), Some(expected));
    }

    #[test]
    fn cache_round_trip_and_invalid_json_are_reported() {
        let temp = tempfile::tempdir().unwrap();
        let cache_path = temp.path().join("cache").join("update.json");
        let cache = UpdateCacheFile {
            checked_at: Utc.with_ymd_and_hms(2026, 4, 20, 10, 0, 0).unwrap(),
            latest_version: Some("9.9.0".to_string()),
            release_url: Some("https://github.com/akiojin/gwt/releases/tag/v9.9.0".to_string()),
            portable_asset_url: Some("https://example.com/gwt-linux-x86_64.tar.gz".to_string()),
            installer_asset_url: Some("https://example.com/gwt-windows-x86_64.msi".to_string()),
            asset_url: Some("https://example.com/legacy.zip".to_string()),
        };

        write_cache(&cache_path, &cache).unwrap();
        let loaded = read_cache(&cache_path).unwrap();
        assert_eq!(loaded.latest_version.as_deref(), Some("9.9.0"));
        assert_eq!(
            loaded.portable_asset_url.as_deref(),
            Some("https://example.com/gwt-linux-x86_64.tar.gz")
        );
        assert_eq!(
            loaded.installer_asset_url.as_deref(),
            Some("https://example.com/gwt-windows-x86_64.msi")
        );

        let invalid_path = temp.path().join("broken.json");
        fs::write(&invalid_path, b"{not-json").unwrap();
        let err = read_cache(&invalid_path).unwrap_err();
        assert!(err.contains("Failed to parse update cache"));
    }

    #[test]
    fn state_from_cache_covers_missing_invalid_and_fallback_release_data() {
        let temp = tempfile::tempdir().unwrap();
        let mgr = UpdateManager::default()
            .with_cache_path(temp.path().join("cache").join("update.json"))
            .with_updates_dir(temp.path().join("updates"));
        let checked_at = Utc.with_ymd_and_hms(2026, 4, 20, 11, 0, 0).unwrap();

        let missing = UpdateCacheFile {
            checked_at,
            latest_version: None,
            release_url: None,
            portable_asset_url: None,
            installer_asset_url: None,
            asset_url: None,
        };
        assert_eq!(
            mgr.state_from_cache(&missing, None),
            UpdateState::UpToDate {
                checked_at: Some(checked_at),
            }
        );

        let invalid = UpdateCacheFile {
            latest_version: Some("not-a-version".to_string()),
            ..missing.clone()
        };
        assert_eq!(
            mgr.state_from_cache(&invalid, None),
            UpdateState::UpToDate {
                checked_at: Some(checked_at),
            }
        );

        let older = UpdateCacheFile {
            latest_version: Some("0.1.0".to_string()),
            ..missing.clone()
        };
        assert_eq!(
            mgr.state_from_cache(&older, None),
            UpdateState::UpToDate {
                checked_at: Some(checked_at),
            }
        );

        let newer = UpdateCacheFile {
            latest_version: Some("99.0.0".to_string()),
            release_url: None,
            portable_asset_url: Some("https://example.com/gwt-portable.tar.gz".to_string()),
            installer_asset_url: Some("https://example.com/gwt-installer.msi".to_string()),
            asset_url: Some("https://example.com/gwt-legacy.zip".to_string()),
            ..missing
        };
        let exe_dir = temp.path().join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let exe_path = exe_dir.join("gwt");
        std::fs::write(&exe_path, b"").unwrap();
        match mgr.state_from_cache(&newer, Some(&exe_path)) {
            UpdateState::Available {
                current,
                latest,
                release_url,
                asset_url,
                checked_at: seen_at,
            } => {
                assert_eq!(current, env!("CARGO_PKG_VERSION"));
                assert_eq!(latest, "99.0.0");
                assert_eq!(seen_at, checked_at);
                assert_eq!(
                    release_url,
                    "https://github.com/akiojin/gwt/releases/tag/v99.0.0"
                );
                assert_eq!(
                    asset_url.as_deref(),
                    Some("https://example.com/gwt-portable.tar.gz")
                );
            }
            other => panic!("expected cached available update, got {other:?}"),
        }
    }

    #[test]
    fn installer_url_selection_covers_legacy_and_unsupported_platforms() {
        let macos = Platform {
            os: "macos".to_string(),
            arch: "x86_64".to_string(),
        };
        let mac_assets = vec![GitHubAsset {
            name: "gwt-macos-x86_64.pkg".to_string(),
            browser_download_url: "https://example.com/macos.pkg".to_string(),
        }];
        assert_eq!(
            find_installer_asset_url(&macos, &mac_assets).as_deref(),
            Some("https://example.com/macos.pkg")
        );

        let windows = Platform {
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
        };
        let windows_assets = vec![GitHubAsset {
            name: "gwt-windows-x86_64.msi".to_string(),
            browser_download_url: "https://example.com/windows.msi".to_string(),
        }];
        assert_eq!(
            find_installer_asset_url(&windows, &windows_assets).as_deref(),
            Some("https://example.com/windows.msi")
        );

        let linux = Platform {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        };
        assert!(find_installer_asset_url(&linux, &windows_assets).is_none());
    }

    #[test]
    fn asset_and_installer_matching_cover_arch_and_suffix_variants() {
        assert!(asset_matches_arch("gwt-aarch64.dmg", "aarch64"));
        assert!(asset_matches_arch("gwt-amd64.msi", "x86_64"));
        assert!(!asset_matches_arch("gwt-aarch64.dmg", "x86_64"));
        assert!(asset_matches_arch("gwt-anything.pkg", "riscv64"));

        let macos = Platform {
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
        };
        let windows = Platform {
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
        };
        let linux = Platform {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        };

        assert_eq!(
            installer_kind_for_url(&macos, "https://example.com/gwt.dmg"),
            Some(InstallerKind::MacDmg)
        );
        assert_eq!(
            installer_kind_for_url(&macos, "https://example.com/gwt.pkg"),
            Some(InstallerKind::MacPkg)
        );
        assert_eq!(
            installer_kind_for_url(&windows, "https://example.com/gwt.msi"),
            Some(InstallerKind::WindowsMsi)
        );
        assert_eq!(
            installer_kind_for_url(&linux, "https://example.com/gwt.tar.gz"),
            None
        );
    }

    #[test]
    fn choose_apply_plan_respects_platform_and_writability() {
        let linux = Platform {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        };
        assert_eq!(
            choose_apply_plan_with_writable(
                &linux,
                false,
                true,
                Some("https://example.com/gwt-linux.tar.gz"),
                Some("https://example.com/gwt-windows.msi"),
            ),
            Some(ApplyPlan::Portable {
                url: "https://example.com/gwt-linux.tar.gz".to_string(),
            })
        );
        assert_eq!(
            choose_apply_plan_with_writable(
                &Platform {
                    os: "windows".to_string(),
                    arch: "x86_64".to_string(),
                },
                false,
                false,
                Some("https://example.com/gwt-linux.tar.gz"),
                Some("https://example.com/gwt-windows.msi"),
            ),
            Some(ApplyPlan::Installer {
                url: "https://example.com/gwt-windows.msi".to_string(),
                kind: InstallerKind::WindowsMsi,
            })
        );
        assert_eq!(
            choose_apply_plan_with_writable(&linux, false, false, None, None),
            None
        );
    }

    #[test]
    fn archive_and_binary_helpers_cover_error_flat_and_missing_layouts() {
        let temp = tempfile::tempdir().unwrap();
        let unsupported = temp.path().join("gwt.bin");
        fs::write(&unsupported, b"bin").unwrap();
        let err = extract_archive(&unsupported, temp.path()).unwrap_err();
        assert!(err.contains("Unsupported archive format"));

        let flat_binary = temp.path().join("gwt");
        fs::write(&flat_binary, b"bin").unwrap();
        assert_eq!(
            find_extracted_binary(temp.path(), "gwt").unwrap(),
            Some(flat_binary)
        );
        assert_eq!(find_extracted_binary(temp.path(), "missing").unwrap(), None);
    }

    #[test]
    fn prepare_update_handles_tarballs_and_empty_payloads() {
        let temp = tempfile::tempdir().unwrap();
        let mgr = UpdateManager::new()
            .with_cache_path(temp.path().join("update-check.json"))
            .with_updates_dir(temp.path().join("updates"));

        let archive_path = temp.path().join("payload.tar.gz");
        {
            let archive_file = fs::File::create(&archive_path).unwrap();
            let encoder =
                flate2::write::GzEncoder::new(archive_file, flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let binary_name = Platform::detect().binary_name();
            let daemon_name = companion_binary_name(&binary_name);
            let bytes = b"tar-gz-bin";
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, format!("nested/{binary_name}"), &bytes[..])
                .unwrap();
            let daemon_bytes = b"tar-gz-daemon";
            let mut daemon_header = tar::Header::new_gnu();
            daemon_header.set_size(daemon_bytes.len() as u64);
            daemon_header.set_mode(0o755);
            daemon_header.set_cksum();
            archive
                .append_data(
                    &mut daemon_header,
                    format!("nested/{daemon_name}"),
                    &daemon_bytes[..],
                )
                .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }

        let tarball_url = serve_once(
            "/gwt.tar.gz",
            "200 OK",
            "application/gzip",
            fs::read(&archive_path).unwrap(),
        );
        let payload = mgr
            .prepare_update("99.0.5", &tarball_url)
            .expect("tarball payload");
        match payload {
            PreparedPayload::PortableBinary { path } => {
                assert_eq!(fs::read(path).unwrap(), b"tar-gz-bin");
            }
            other => panic!("expected tarball portable payload, got {other:?}"),
        }

        let empty_url = serve_once("/empty.bin", "200 OK", "application/octet-stream", vec![]);
        let err = mgr.prepare_update("99.0.6", &empty_url).unwrap_err();
        assert!(err.contains("Downloaded payload is empty"));
    }

    #[test]
    fn install_latest_docker_linux_bundle_downloads_release_tarball_to_cache_paths() {
        let temp = tempfile::tempdir().unwrap();
        let tarball_url = serve_once(
            "/downloads/gwt-linux-x86_64.tar.gz",
            "200 OK",
            "application/gzip",
            tar_gz_bundle_body(b"docker-gwt", b"docker-gwtd"),
        );
        let release_body = format!(
            r#"{{
  "tag_name": "v99.1.0",
  "html_url": "https://github.com/akiojin/gwt/releases/tag/v99.1.0",
  "assets": [
    {{
      "name": "gwt-linux-x86_64.tar.gz",
      "browser_download_url": "{tarball_url}"
    }}
  ]
}}"#
        );
        let base_url = serve_once("/", "200 OK", "application/json", release_body.into_bytes());
        let mgr = UpdateManager::new()
            .with_api_base_url(base_url)
            .with_cache_path(temp.path().join("update-check.json"))
            .with_updates_dir(temp.path().join("updates"));
        let target_gwt = temp.path().join(".gwt").join("bin").join("gwt-linux");
        let target_gwtd = temp.path().join(".gwt").join("bin").join("gwtd-linux");

        let installed = mgr
            .install_latest_docker_linux_bundle("x86_64", &target_gwt, &target_gwtd)
            .expect("install docker bundle");

        assert_eq!(installed.version, "99.1.0");
        assert_eq!(installed.gwt_path, target_gwt);
        assert_eq!(installed.gwtd_path, target_gwtd);
        assert_eq!(fs::read(&installed.gwt_path).unwrap(), b"docker-gwt");
        assert_eq!(fs::read(&installed.gwtd_path).unwrap(), b"docker-gwtd");
    }

    #[test]
    fn install_latest_docker_linux_bundle_uses_requested_arch_asset() {
        let temp = tempfile::tempdir().unwrap();
        let x64_url = serve_once(
            "/downloads/gwt-linux-x86_64.tar.gz",
            "200 OK",
            "application/gzip",
            tar_gz_bundle_body(b"x64-gwt", b"x64-gwtd"),
        );
        let arm64_url = serve_once(
            "/downloads/gwt-linux-aarch64.tar.gz",
            "200 OK",
            "application/gzip",
            tar_gz_bundle_body(b"arm64-gwt", b"arm64-gwtd"),
        );
        let release_body = format!(
            r#"{{
  "tag_name": "v99.1.1",
  "html_url": "https://github.com/akiojin/gwt/releases/tag/v99.1.1",
  "assets": [
    {{
      "name": "gwt-linux-x86_64.tar.gz",
      "browser_download_url": "{x64_url}"
    }},
    {{
      "name": "gwt-linux-aarch64.tar.gz",
      "browser_download_url": "{arm64_url}"
    }}
  ]
}}"#
        );
        let base_url = serve_once("/", "200 OK", "application/json", release_body.into_bytes());
        let mgr = UpdateManager::new()
            .with_api_base_url(base_url)
            .with_cache_path(temp.path().join("update-check.json"))
            .with_updates_dir(temp.path().join("updates"));
        let target_gwt = temp.path().join(".gwt").join("bin").join("gwt-linux");
        let target_gwtd = temp.path().join(".gwt").join("bin").join("gwtd-linux");

        let installed = mgr
            .install_latest_docker_linux_bundle("aarch64", &target_gwt, &target_gwtd)
            .expect("install docker bundle for aarch64");

        assert_eq!(installed.version, "99.1.1");
        assert_eq!(fs::read(&installed.gwt_path).unwrap(), b"arm64-gwt");
        assert_eq!(fs::read(&installed.gwtd_path).unwrap(), b"arm64-gwtd");
    }

    #[test]
    fn app_bundle_helpers_find_matching_paths() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("GWT.app");
        let macos_dir = bundle.join("Contents").join("MacOS");
        fs::create_dir_all(&macos_dir).unwrap();
        let bundle_exe = macos_dir.join("gwt");
        fs::write(&bundle_exe, b"bin").unwrap();

        let target = Path::new("/usr/local/bin/gwt");
        assert_eq!(
            app_bundle_from_executable(&bundle_exe),
            Some(bundle.clone())
        );
        assert_eq!(
            app_bundle_executable_path(&bundle, target),
            Some(bundle_exe)
        );
        assert_eq!(
            find_matching_app_bundle(temp.path(), target),
            Some(bundle.clone())
        );
        #[cfg(target_os = "macos")]
        assert_eq!(find_first_app_bundle(temp.path()).unwrap(), Some(bundle));
        assert_eq!(sh_single_quote("it's ready"), "'it'\\''s ready'");
    }

    #[test]
    fn app_bundle_helpers_cover_preferred_and_fallback_paths() {
        let temp = tempfile::tempdir().unwrap();
        let apps = temp.path().join("Applications");
        let preferred_dir = apps.join("Preferred.app").join("Contents").join("MacOS");
        fs::create_dir_all(&preferred_dir).unwrap();
        fs::write(preferred_dir.join("gwt"), b"preferred").unwrap();

        let fallback_dir = apps.join("Fallback.app").join("Contents").join("MacOS");
        fs::create_dir_all(&fallback_dir).unwrap();
        fs::write(fallback_dir.join("alt-gwt"), b"fallback").unwrap();

        assert_eq!(
            app_bundle_executable_path(&apps.join("Fallback.app"), Path::new("/usr/local/bin/gwt")),
            Some(fallback_dir.join("alt-gwt"))
        );
        assert_eq!(
            resolve_macos_restart_executable(
                &apps,
                Path::new("/usr/local/bin/gwt"),
                Some(std::ffi::OsStr::new("Preferred.app")),
            ),
            preferred_dir.join("gwt")
        );
        assert_eq!(
            find_matching_app_bundle(&apps, Path::new("/usr/local/bin/missing")),
            None
        );
    }

    #[test]
    fn replace_executables_with_retry_validates_source_and_swaps_files() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp
            .path()
            .join("bin")
            .join(if cfg!(windows) { "gwt.exe" } else { "gwt" });
        let source = temp
            .path()
            .join(if cfg!(windows) { "new.exe" } else { "new" });

        let missing_err = replace_executables_with_retry(&[(&target, &source)]).unwrap_err();
        assert!(missing_err.contains("Source missing"));

        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"old").unwrap();
        fs::write(&source, b"new-binary").unwrap();
        replace_executables_with_retry(&[(&target, &source)]).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new-binary");

        let empty = temp
            .path()
            .join(if cfg!(windows) { "empty.exe" } else { "empty" });
        fs::write(&empty, b"").unwrap();
        let empty_err = replace_executables_with_retry(&[(&target, &empty)]).unwrap_err();
        assert!(empty_err.contains("Source executable is empty"));
    }

    #[test]
    fn replace_paths_rolls_back_when_new_executable_cannot_be_moved() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp
            .path()
            .join(if cfg!(windows) { "gwt.exe" } else { "gwt" });
        let backup = temp.path().join(if cfg!(windows) {
            "gwt.exe.old"
        } else {
            "gwt.old"
        });
        let missing_tmp = temp.path().join(if cfg!(windows) {
            "missing.exe"
        } else {
            "missing"
        });

        fs::write(&target, b"original").unwrap();
        let err = replace_paths(&target, &backup, &missing_tmp).unwrap_err();
        assert!(!err.to_string().is_empty());
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert!(!backup.exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_installer_helpers_cover_timeout_and_platform_errors() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let args_path = temp.path().join("restart").join("args.json");
        let mgr = UpdateManager::new()
            .with_cache_path(temp.path().join("update-check.json"))
            .with_updates_dir(temp.path().join("updates"));
        mgr.write_restart_args_file(&args_path, vec!["--version".to_string()])
            .expect("write restart args");

        assert!(is_process_running(std::process::id()));
        assert!(!is_process_running(999_999));

        let timeout_err =
            wait_for_pid_exit(std::process::id(), StdDuration::from_millis(1)).unwrap_err();
        assert!(timeout_err.contains("Timed out waiting for process"));
        assert!(wait_for_pid_exit(999_999, StdDuration::from_millis(1)).is_ok());

        let mac_pkg_err = internal_run_installer(
            999_999,
            Path::new("gwt.exe"),
            Path::new("installer.pkg"),
            InstallerKind::MacPkg,
            &args_path,
        )
        .unwrap_err();
        assert!(mac_pkg_err.contains("mac_pkg installer can only run on macOS"));

        let mac_dmg_err = internal_run_installer(
            999_999,
            Path::new("gwt.exe"),
            Path::new("installer.dmg"),
            InstallerKind::MacDmg,
            &args_path,
        )
        .unwrap_err();
        assert!(mac_dmg_err.contains("mac_dmg installer can only run on macOS"));
    }
}
