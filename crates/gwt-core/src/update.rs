//! Self-update support via GitHub Releases.
//!
//! This module implements:
//! - Update discovery via GitHub Releases (latest)
//! - TTL-based local cache to avoid repeated API calls
//! - User-approved apply flow: download payload, then restart into the new version
//! - Internal helper mode to safely replace the running executable (especially on Windows)

use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
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
        /// Preferred payload URL for this platform, if present.
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
                    return self.state_from_cache(cache);
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

                let asset_url = expected_asset_name()
                    .and_then(|name| release.assets.iter().find(|a| a.name == name))
                    .map(|a| a.browser_download_url.clone());

                let cache_file = UpdateCacheFile {
                    checked_at: now,
                    latest_version: Some(latest_ver.to_string()),
                    release_url: Some(release.html_url.clone()),
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
                        return self.state_from_cache(cache);
                    }
                }
                UpdateState::Failed {
                    message: err,
                    failed_at: now,
                }
            }
        }
    }

    pub fn prepare_update(&self, latest: &str, asset_url: &str) -> Result<PathBuf, String> {
        let update_dir = self
            .updates_dir
            .join(format!("v{}", latest.trim().trim_start_matches('v')));
        fs::create_dir_all(&update_dir).map_err(|e| format!("Failed to create update dir: {e}"))?;

        let asset_name = asset_name_from_url(asset_url)
            .or_else(expected_asset_name)
            .unwrap_or_else(|| "gwt-update".to_string());
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dest)
                .ok()
                .map(|m| m.permissions().mode())
                .unwrap_or(0o755);
            let mut perms = fs::metadata(&dest)
                .map_err(|e| format!("Failed to read payload metadata: {e}"))?
                .permissions();
            perms.set_mode(mode | 0o111);
            let _ = fs::set_permissions(&dest, perms);
        }

        Ok(dest)
    }

    pub fn write_restart_args_file(&self, path: &Path, args: Vec<String>) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Invalid args file path".to_string())?;
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create args dir: {e}"))?;
        write_json_atomic(path, &RestartArgsFile { args })
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
        target_exe: &Path,
        new_exe: &Path,
        args_file: &Path,
    ) -> Result<(), String> {
        Command::new(helper_exe)
            .arg("__internal")
            .arg("apply-update")
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

    fn state_from_cache(&self, cache: &UpdateCacheFile) -> UpdateState {
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
            UpdateState::Available {
                current: self.current_version.to_string(),
                latest: latest_ver.to_string(),
                release_url,
                asset_url: cache.asset_url.clone(),
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
    target_exe: &Path,
    source_exe: &Path,
    args_file: &Path,
) -> Result<(), String> {
    let args = UpdateManager::read_restart_args_file(args_file)?;
    replace_executable(target_exe, source_exe)?;

    Command::new(target_exe)
        .args(args)
        .spawn()
        .map_err(|e| format!("Failed to restart: {e}"))?;
    Ok(())
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

fn expected_asset_name() -> Option<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("macos", "aarch64") => Some("gwt-macos-aarch64".to_string()),
        ("macos", "x86_64") => Some("gwt-macos-x86_64".to_string()),
        ("linux", "aarch64") => Some("gwt-linux-aarch64".to_string()),
        ("linux", "x86_64") => Some("gwt-linux-x86_64".to_string()),
        ("windows", "x86_64") => Some("gwt-windows-x86_64.exe".to_string()),
        _ => None,
    }
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
            asset_url: Some("https://example.com/asset".to_string()),
        };
        write_cache(mgr.cache_path(), &cache).unwrap();

        let state = mgr.check(true);
        assert!(matches!(state, UpdateState::Failed { .. }));
    }
}
