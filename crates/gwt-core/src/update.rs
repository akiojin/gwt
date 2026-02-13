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

const DEFAULT_OWNER: &str = "akiojin";
const DEFAULT_REPO: &str = "gwt";
const DEFAULT_TTL: Duration = Duration::from_secs(60 * 60 * 24);
const DEFAULT_API_BASE_URL: &str = "https://api.github.com";

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
            ("linux", "aarch64") => Some("linux-arm64"),
            ("macos", "x86_64") => Some("macos-x86_64"),
            ("macos", "aarch64") => Some("macos-arm64"),
            ("windows", "x86_64") => Some("windows-x86_64"),
            _ => None,
        }
    }

    fn binary_name(&self) -> String {
        if self.os == "windows" {
            "gwt.exe".to_string()
        } else {
            "gwt".to_string()
        }
    }

    fn portable_asset_name(&self) -> Option<String> {
        let artifact = self.artifact()?;
        if self.os == "windows" {
            Some(format!("gwt-{artifact}.zip"))
        } else {
            Some(format!("gwt-{artifact}.tar.gz"))
        }
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

        let (cache_path, updates_dir) = default_paths();

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
                let latest_ver = match parse_tag_version(&release.tag_name) {
                    Some(v) => v,
                    None => {
                        return UpdateState::Failed {
                            message: format!(
                                "Failed to parse release tag as version: {}",
                                release.tag_name
                            ),
                            failed_at: now,
                        };
                    }
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
                    portable_asset_url: portable_asset_url.clone(),
                    installer_asset_url: installer_asset_url.clone(),
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

        let res = self
            .client
            .get(asset_url)
            .send()
            .map_err(|e| format!("Download failed: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("Download failed with status {}", res.status()));
        }

        let mut file =
            fs::File::create(&dest).map_err(|e| format!("Failed to create payload file: {e}"))?;
        let mut reader = res;
        io::copy(&mut reader, &mut file).map_err(|e| format!("Failed to write payload: {e}"))?;

        let size = fs::metadata(&dest).map(|m| m.len()).unwrap_or_default();
        if size == 0 {
            let _ = fs::remove_file(&dest);
            return Err("Downloaded payload is empty".to_string());
        }

        let dest_str = dest.to_string_lossy().to_string();
        if dest_str.ends_with(".tar.gz") || dest_str.ends_with(".zip") {
            let extract_dir = update_dir.join("extract");
            let _ = fs::remove_dir_all(&extract_dir);
            fs::create_dir_all(&extract_dir)
                .map_err(|e| format!("Failed to create extract dir: {e}"))?;
            extract_archive(&dest, &extract_dir)?;
            let platform = Platform::detect();
            let binary_name = platform.binary_name();
            let Some(binary_path) = find_extracted_binary(&extract_dir, &binary_name)? else {
                return Err(format!(
                    "Extracted payload does not contain expected binary: {binary_name}"
                ));
            };
            ensure_executable(&binary_path)?;
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
        crate::process::command_os(helper_exe)
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
        crate::process::command_os(helper_exe)
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
    replace_executable(target_exe, source_exe)?;

    crate::process::command_os(target_exe)
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
        match installer_kind {
            InstallerKind::MacDmg => {
                #[cfg(target_os = "macos")]
                {
                    run_macos_dmg_installer_with_privileges(installer, target_exe)?;
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
                }
                #[cfg(not(target_os = "windows"))]
                {
                    return Err("windows_msi installer can only run on Windows".to_string());
                }
            }
        }

        let args = UpdateManager::read_restart_args_file(args_file)?;
        crate::process::command_os(target_exe)
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

fn default_paths() -> (PathBuf, PathBuf) {
    let base = dirs::home_dir()
        .map(|h| h.join(".gwt"))
        .unwrap_or_else(|| std::env::temp_dir().join("gwt"));
    let cache_path = base.join("update-cache.json");
    let updates_dir = base.join("updates");
    (cache_path, updates_dir)
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
            // New release flow: prefer DMG for macOS.
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
            // New release flow: WiX MSI.
            if let Some(asset) = assets
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case("gwt-wix-windows-x86_64.msi"))
            {
                return Some(asset.browser_download_url.clone());
            }

            // Legacy naming fallback.
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

fn choose_apply_plan(
    platform: &Platform,
    current_exe: Option<&Path>,
    portable_url: Option<&str>,
    installer_url: Option<&str>,
) -> Option<ApplyPlan> {
    // macOS: prefer installer when available to preserve codesign/notarization integrity.
    if platform.os == "macos" {
        if let Some(url) = installer_url {
            let kind = installer_kind_for_url(platform, url)?;
            return Some(ApplyPlan::Installer {
                url: url.to_string(),
                kind,
            });
        }
    }

    let writable = current_exe
        .and_then(|p| p.parent())
        .and_then(|dir| is_dir_writable(dir).ok())
        .unwrap_or(true);

    // If we cannot replace in-place, prefer installer when available.
    if !writable {
        if let Some(url) = installer_url {
            let kind = installer_kind_for_url(platform, url)?;
            return Some(ApplyPlan::Installer {
                url: url.to_string(),
                kind,
            });
        }
        return None;
    }

    if let Some(url) = portable_url {
        return Some(ApplyPlan::Portable {
            url: url.to_string(),
        });
    }

    if let Some(url) = installer_url {
        let kind = installer_kind_for_url(platform, url)?;
        return Some(ApplyPlan::Installer {
            url: url.to_string(),
            kind,
        });
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
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                // Ignore unreadable directories and continue searching other paths.
                continue;
            }
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

fn ensure_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
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
        return crate::process::command("powershell")
            .args(["-NoProfile", "-Command", &script])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }

    #[cfg(not(target_os = "windows"))]
    {
        crate::process::command("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

#[cfg(target_os = "macos")]
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
    let status = crate::process::command("osascript")
        .arg("-e")
        .arg(applescript_cmd)
        .status()
        .map_err(|e| format!("Failed to run privileged command via osascript: {e}"))?;
    if !status.success() {
        return Err(format!("osascript command exited with {status}"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn app_bundle_from_executable(target_exe: &Path) -> Option<PathBuf> {
    target_exe
        .ancestors()
        .find(|p| p.extension() == Some(OsStr::new("app")))
        .map(Path::to_path_buf)
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
) -> Result<(), String> {
    let mount_dir = std::env::temp_dir().join(format!("gwt-update-dmg-{}", std::process::id()));
    let _ = fs::remove_dir_all(&mount_dir);
    fs::create_dir_all(&mount_dir).map_err(|e| format!("Failed to create mount dir: {e}"))?;

    let attach_status = crate::process::command("hdiutil")
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

    let install_result = (|| {
        let source_app = find_first_app_bundle(&mount_dir)?
            .ok_or_else(|| "Mounted dmg does not contain an .app bundle".to_string())?;
        let source_name = source_app
            .file_name()
            .ok_or_else(|| "Mounted app bundle has an invalid name".to_string())?;
        let target_app = app_bundle_from_executable(target_exe)
            .unwrap_or_else(|| PathBuf::from("/Applications").join(source_name));

        let shell_cmd = format!(
            "rm -rf {} && /usr/bin/ditto {} {}",
            sh_single_quote(&target_app.to_string_lossy()),
            sh_single_quote(&source_app.to_string_lossy()),
            sh_single_quote(&target_app.to_string_lossy())
        );
        run_shell_with_admin_privileges(&shell_cmd)
    })();

    let detach_status = crate::process::command("hdiutil")
        .arg("detach")
        .arg(&mount_dir)
        .arg("-force")
        .status()
        .map_err(|e| format!("Failed to unmount dmg: {e}"))?;
    let _ = fs::remove_dir_all(&mount_dir);

    install_result?;
    if !detach_status.success() {
        return Err(format!("hdiutil detach exited with {detach_status}"));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn run_windows_msi_with_uac(installer: &Path) -> Result<(), String> {
    // Trigger UAC for msiexec via PowerShell.
    let msi = installer.to_string_lossy().to_string();
    let args = format!(
        "Start-Process msiexec.exe -Verb RunAs -Wait -ArgumentList @('/i', '{}', '/passive')",
        msi.replace('\'', "''")
    );
    let status = crate::process::command("powershell")
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

fn replace_executable(target_exe: &Path, source_exe: &Path) -> Result<(), String> {
    let source_meta = fs::metadata(source_exe).map_err(|e| format!("Source missing: {e}"))?;
    if source_meta.len() == 0 {
        return Err("Source executable is empty".to_string());
    }

    let target_dir = target_exe
        .parent()
        .ok_or_else(|| "Target executable path has no parent dir".to_string())?;
    fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to ensure target dir exists: {e}"))?;

    let tmp_name = format!(".gwt-update-{}.tmp", std::process::id());
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

    let file_name = target_exe
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| "Target executable has invalid filename".to_string())?;
    let backup_path = target_dir.join(format!("{file_name}.old"));
    let _ = fs::remove_file(&backup_path);

    // Windows: file replacement can fail while the parent app is still shutting down.
    const MAX_RETRIES: usize = 200;
    const SLEEP_MS: u64 = 50;

    for attempt in 0..MAX_RETRIES {
        match replace_paths(target_exe, &backup_path, &tmp_path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt + 1 == MAX_RETRIES {
                    let _ = fs::remove_file(&tmp_path);
                    return Err(format!("Failed to replace executable: {e}"));
                }
                std::thread::sleep(Duration::from_millis(SLEEP_MS));
            }
        }
    }

    Err("Failed to replace executable".to_string())
}

fn replace_paths(target_exe: &Path, backup_path: &Path, tmp_path: &Path) -> io::Result<()> {
    if target_exe.exists() {
        let _ = fs::remove_file(backup_path);
        fs::rename(target_exe, backup_path)?;
    }
    fs::rename(tmp_path, target_exe)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _guard = crate::config::TestEnvGuard::new(temp.path());

        let mgr = UpdateManager::new().with_api_base_url("http://127.0.0.1:9");

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
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _guard = crate::config::TestEnvGuard::new(temp.path());

        let mgr = UpdateManager::new().with_api_base_url("http://127.0.0.1:9");

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
    fn find_installer_asset_url_prefers_windows_wix_msi() {
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
        ];

        let url = find_installer_asset_url(&platform, &assets);
        assert_eq!(url.as_deref(), Some("https://example.com/wix.msi"));
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
}
