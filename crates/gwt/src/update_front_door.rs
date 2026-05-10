use super::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Initial delay after GUI launch before the first update check (FR-001 / FR-035).
pub const STARTUP_INITIAL_DELAY: Duration = Duration::from_millis(3000);

/// FR-002: gap between fast retries on the very first startup check.
const STARTUP_RETRY_GAP: Duration = Duration::from_millis(3000);

/// FR-002: number of fast retries on the very first startup check before the
/// 5min steady-state poll loop takes over.
const STARTUP_RETRY_ATTEMPTS: u32 = 3;

/// Steady-state polling intervals used by the exponential backoff in
/// [`PollState`] (FR-037). Index 0 is the success-path interval; subsequent
/// indices are reached after consecutive failures.
const POLL_INTERVAL_LADDER: &[Duration] = &[
    Duration::from_secs(5 * 60),  // 0 failures (or after success): 5 min
    Duration::from_secs(10 * 60), // 1 failure : 10 min
    Duration::from_secs(20 * 60), // 2 failures: 20 min
    Duration::from_secs(60 * 60), // 3+ failures: 60 min (cap)
];

/// State machine for the GUI auto-update polling loop (FR-035 / FR-036 / FR-037).
///
/// Tracks the last broadcasted `latest` version (so the same release is not
/// re-broadcasted) and the consecutive failure count (so retries back off).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PollState {
    failure_count: usize,
    last_seen: Option<String>,
}

impl PollState {
    /// Current sleep interval before the next poll attempt.
    pub fn current_interval(&self) -> Duration {
        let idx = self.failure_count.min(POLL_INTERVAL_LADDER.len() - 1);
        POLL_INTERVAL_LADDER[idx]
    }

    /// Latest version that was last broadcasted (test introspection).
    #[cfg(test)]
    pub fn last_seen(&self) -> Option<&str> {
        self.last_seen.as_deref()
    }

    /// Successful poll that returned `UpToDate` (or `Available` without an
    /// applicable asset). Resets the backoff to the base interval and returns
    /// the next sleep duration.
    pub fn on_success_uptodate(&mut self) -> Duration {
        self.failure_count = 0;
        self.current_interval()
    }

    /// Successful poll that returned `Available` with an applicable asset.
    /// Returns `(should_broadcast, next_interval)`. `should_broadcast` is
    /// `true` only when `latest` differs from the last broadcasted version,
    /// implementing FR-036's duplicate suppression.
    pub fn on_success_available(&mut self, latest: &str) -> (bool, Duration) {
        let should_broadcast = self.last_seen.as_deref() != Some(latest);
        if should_broadcast {
            self.last_seen = Some(latest.to_string());
        }
        self.failure_count = 0;
        (should_broadcast, self.current_interval())
    }

    /// Failed poll (network error, transient API failure). Advances the
    /// backoff one step (capped at 60 min) and returns the next sleep duration.
    pub fn on_failure(&mut self) -> Duration {
        self.failure_count = self.failure_count.saturating_add(1);
        self.current_interval()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupUpdateAction {
    Publish,
    Stop,
    Retry,
}

pub fn classify_startup_update_state(state: &gwt_core::update::UpdateState) -> StartupUpdateAction {
    match state {
        gwt_core::update::UpdateState::Available {
            asset_url: Some(_), ..
        } => StartupUpdateAction::Publish,
        gwt_core::update::UpdateState::Available {
            asset_url: None, ..
        }
        | gwt_core::update::UpdateState::UpToDate { .. } => StartupUpdateAction::Stop,
        gwt_core::update::UpdateState::Failed { .. } => StartupUpdateAction::Retry,
    }
}

pub fn spawn_startup_update_check(
    runtime: &Runtime,
    _clients: ClientHub,
    update_proxy: EventLoopProxy<UserEvent>,
) {
    if gwt_core::update::is_ci() {
        return;
    }
    runtime.spawn(async move {
        tokio::time::sleep(STARTUP_INITIAL_DELAY).await;
        let current_exe = std::env::current_exe().ok();
        let mut state = PollState::default();

        // FR-002: the first startup check uses up to 3 fast retries (3s gap)
        // so transient boot-time network failures still surface a toast in
        // under 10 seconds. Once any outcome lands (or all retries are
        // exhausted), the 5min steady-state poll loop (FR-035 / FR-037) takes
        // over and exponential backoff governs further failures.
        let mut initial_resolved = false;
        for attempt in 0..STARTUP_RETRY_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(STARTUP_RETRY_GAP).await;
            }
            let exe = current_exe.clone();
            let join = tokio::task::spawn_blocking(move || {
                gwt_core::update::UpdateManager::new().check_for_executable(true, exe.as_deref())
            })
            .await;
            let Ok(update_state) = join else {
                continue;
            };
            match classify_startup_update_state(&update_state) {
                StartupUpdateAction::Retry => continue,
                _ => {
                    handle_poll_outcome(&mut state, update_state, &update_proxy);
                    initial_resolved = true;
                    break;
                }
            }
        }
        if !initial_resolved {
            // All initial retries failed: enter the loop with a single
            // failure recorded so the next attempt is 10 minutes away.
            state.on_failure();
        }

        loop {
            tokio::time::sleep(state.current_interval()).await;
            let exe = current_exe.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                gwt_core::update::UpdateManager::new().check_for_executable(true, exe.as_deref())
            })
            .await;
            match outcome {
                Ok(update_state) => {
                    handle_poll_outcome(&mut state, update_state, &update_proxy);
                }
                Err(_) => {
                    state.on_failure();
                }
            }
        }
    });
}

fn handle_poll_outcome(
    state: &mut PollState,
    outcome: gwt_core::update::UpdateState,
    update_proxy: &EventLoopProxy<UserEvent>,
) -> Duration {
    match classify_startup_update_state(&outcome) {
        StartupUpdateAction::Publish => {
            let latest = match &outcome {
                gwt_core::update::UpdateState::Available { latest, .. } => latest.clone(),
                _ => return state.on_success_uptodate(),
            };
            let (should_publish, next) = state.on_success_available(&latest);
            if should_publish {
                let _ = update_proxy.send_event(UserEvent::UpdateAvailable(outcome));
            }
            next
        }
        StartupUpdateAction::Stop => state.on_success_uptodate(),
        StartupUpdateAction::Retry => state.on_failure(),
    }
}

trait UpdateApplyOps {
    fn prepare_update(
        &mut self,
        latest: &str,
        asset_url: &str,
    ) -> Result<gwt_core::update::PreparedPayload, String>;
    fn write_restart_args_file(&mut self, path: &Path, args: Vec<String>) -> Result<(), String>;
    fn make_helper_copy(&mut self, current_exe: &Path, latest: &str) -> Result<PathBuf, String>;
    fn spawn_internal_apply_update(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        payload: &Path,
        args_file: &Path,
    ) -> Result<(), String>;
    fn spawn_internal_run_installer(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        installer: &Path,
        kind: gwt_core::update::InstallerKind,
        args_file: &Path,
    ) -> Result<(), String>;
}

struct RealUpdateApplyOps {
    mgr: gwt_core::update::UpdateManager,
}

impl Default for RealUpdateApplyOps {
    fn default() -> Self {
        Self {
            mgr: gwt_core::update::UpdateManager::new(),
        }
    }
}

impl UpdateApplyOps for RealUpdateApplyOps {
    fn prepare_update(
        &mut self,
        latest: &str,
        asset_url: &str,
    ) -> Result<gwt_core::update::PreparedPayload, String> {
        self.mgr.prepare_update(latest, asset_url)
    }

    fn write_restart_args_file(&mut self, path: &Path, args: Vec<String>) -> Result<(), String> {
        self.mgr.write_restart_args_file(path, args)
    }

    fn make_helper_copy(&mut self, current_exe: &Path, latest: &str) -> Result<PathBuf, String> {
        self.mgr.make_helper_copy(current_exe, latest)
    }

    fn spawn_internal_apply_update(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        payload: &Path,
        args_file: &Path,
    ) -> Result<(), String> {
        self.mgr
            .spawn_internal_apply_update(helper_exe, old_pid, current_exe, payload, args_file)
    }

    fn spawn_internal_run_installer(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        installer: &Path,
        kind: gwt_core::update::InstallerKind,
        args_file: &Path,
    ) -> Result<(), String> {
        self.mgr.spawn_internal_run_installer(
            helper_exe,
            old_pid,
            current_exe,
            installer,
            kind,
            args_file,
        )
    }
}

/// SPEC-2041 Phase 19 (T-130): a download that has been completed but not yet
/// committed via the helper subprocess. Constructed by
/// [`prepare_update_payload`] from a verified `UpdateState::Available`, then
/// reused by `apply_update_state_and_exit` (which performs the helper spawn +
/// `exit(0)`). The struct intentionally owns the on-disk path so the apply
/// path does not have to re-derive it from the asset URL.
#[derive(Debug, Clone)]
pub struct PreparedUpdate {
    pub latest: String,
    /// Source URL the asset was downloaded from. Retained for diagnostics
    /// (logs, retry telemetry) so future Phase 19 work can correlate ready
    /// payloads back to the upstream release without re-walking the cache.
    #[allow(dead_code)]
    pub asset_url: String,
    pub payload: gwt_core::update::PreparedPayload,
}

impl PreparedUpdate {
    /// Filesystem path of the prepared payload (binary or installer).
    pub fn payload_path(&self) -> PathBuf {
        match &self.payload {
            gwt_core::update::PreparedPayload::PortableBinary { path }
            | gwt_core::update::PreparedPayload::Installer { path, .. } => path.clone(),
        }
    }
}

/// SPEC-2041 Phase 19 (FR-052/056): download and stage the update payload
/// without spawning the helper subprocess. The caller (typically the
/// `UserEvent::ApplyUpdateStart` worker thread in `main.rs`) broadcasts
/// [`crate::BackendEvent::UpdateReady`] when this returns `Ok`.
///
/// This is the explicit no-side-effects half of the legacy
/// [`apply_update_state_and_exit`]: it never calls `exit(0)`, never spawns the
/// helper, and never mutates the running binary. T-130 will eventually replace
/// `apply_update_state_and_exit` entirely with this + a separate
/// `commit_update_restart_now` once Phase 19 stabilizes.
/// Convenience wrapper that drops download-progress events. Kept on the
/// public API for future non-streaming callers (CLI, automated tests) so the
/// progress-aware variant does not have to be re-discovered.
#[allow(dead_code)]
pub fn prepare_update_payload(
    state: gwt_core::update::UpdateState,
) -> Result<PreparedUpdate, String> {
    prepare_update_payload_with_progress(state, &mut |_, _| {})
}

/// SPEC-2041 Phase 19 (FR-054): like [`prepare_update_payload`] but routes
/// download chunk progress to `progress`. The closure must be cheap because
/// it fires once per 64 KiB of payload; callers that broadcast progress over
/// WebSocket should throttle inside the closure (e.g. by `Instant::elapsed`).
pub fn prepare_update_payload_with_progress(
    state: gwt_core::update::UpdateState,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<PreparedUpdate, String> {
    let (latest, asset_url) = match state {
        gwt_core::update::UpdateState::Available {
            latest,
            asset_url: Some(asset_url),
            ..
        } => (latest, asset_url),
        gwt_core::update::UpdateState::Available { .. } => {
            return Err("No applicable update asset is available for this platform.".to_string());
        }
        gwt_core::update::UpdateState::UpToDate { .. } => {
            return Err("No pending update is available.".to_string());
        }
        gwt_core::update::UpdateState::Failed { message, .. } => {
            return Err(format!("Update check failed: {message}"));
        }
    };
    let mgr = gwt_core::update::UpdateManager::new();
    let payload = mgr
        .prepare_update_with_progress(&latest, &asset_url, progress)
        .map_err(|err| format!("Failed to prepare update payload: {err}"))?;
    Ok(PreparedUpdate {
        latest,
        asset_url,
        payload,
    })
}

/// Apply an already-detected update state from the GUI notification path.
///
/// The startup poll has already selected the platform-specific asset. Reusing
/// that state avoids a second network/cache check that can make a clicked toast
/// appear to do nothing when the re-check does not return `Available`.
pub fn apply_update_state_and_exit(state: gwt_core::update::UpdateState) -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve current executable: {err}"))?;
    let restart_args: Vec<String> = std::env::args().skip(1).collect();
    let mut ops = RealUpdateApplyOps::default();
    start_update_apply_with_ops(
        &mut ops,
        state,
        &current_exe,
        restart_args,
        std::process::id(),
        cfg!(windows),
    )?;
    std::process::exit(0);
}

fn start_update_apply_with_ops(
    ops: &mut impl UpdateApplyOps,
    state: gwt_core::update::UpdateState,
    current_exe: &Path,
    restart_args: Vec<String>,
    old_pid: u32,
    use_helper_copy: bool,
) -> Result<(), String> {
    let (latest, asset_url) = match state {
        gwt_core::update::UpdateState::Available {
            latest,
            asset_url: Some(asset_url),
            ..
        } => (latest, asset_url),
        gwt_core::update::UpdateState::Available { .. } => {
            return Err("No applicable update asset is available for this platform.".to_string());
        }
        gwt_core::update::UpdateState::UpToDate { .. } => {
            return Err("No pending update is available.".to_string());
        }
        gwt_core::update::UpdateState::Failed { message, .. } => {
            return Err(format!("Update check failed: {message}"));
        }
    };
    let payload = ops
        .prepare_update(&latest, &asset_url)
        .map_err(|err| format!("Failed to prepare update payload: {err}"))?;
    let args_file = (match &payload {
        gwt_core::update::PreparedPayload::PortableBinary { path }
        | gwt_core::update::PreparedPayload::Installer { path, .. } => {
            path.parent().map(|dir| dir.join("restart-args.json"))
        }
    })
    .ok_or_else(|| "Failed to determine update restart args path.".to_string())?;
    ops.write_restart_args_file(&args_file, restart_args)
        .map_err(|err| format!("Failed to write update restart args: {err}"))?;
    let helper_exe = if use_helper_copy {
        ops.make_helper_copy(current_exe, &latest)
            .map_err(|err| format!("Failed to prepare update helper: {err}"))?
    } else {
        current_exe.to_path_buf()
    };
    match payload {
        gwt_core::update::PreparedPayload::PortableBinary { path } => ops
            .spawn_internal_apply_update(&helper_exe, old_pid, current_exe, &path, &args_file)
            .map_err(|err| format!("Failed to start update helper: {err}")),
        gwt_core::update::PreparedPayload::Installer { path, kind } => ops
            .spawn_internal_run_installer(
                &helper_exe,
                old_pid,
                current_exe,
                &path,
                kind,
                &args_file,
            )
            .map_err(|err| format!("Failed to start update installer: {err}")),
    }
}

#[cfg(test)]
mod poll_state_tests {
    use super::{start_update_apply_with_ops, PollState, UpdateApplyOps};
    use gwt_core::update::{InstallerKind, PreparedPayload, UpdateState};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    const FIVE_MIN: Duration = Duration::from_secs(5 * 60);
    const TEN_MIN: Duration = Duration::from_secs(10 * 60);
    const TWENTY_MIN: Duration = Duration::from_secs(20 * 60);
    const SIXTY_MIN: Duration = Duration::from_secs(60 * 60);

    #[test]
    fn default_state_uses_5min_interval_and_no_last_seen() {
        let state = PollState::default();
        assert_eq!(state.current_interval(), FIVE_MIN);
        assert_eq!(state.last_seen(), None);
    }

    #[test]
    fn first_available_detection_broadcasts_and_records_latest() {
        let mut state = PollState::default();
        let (should, next) = state.on_success_available("9.19.0");
        assert!(should, "first detection must broadcast");
        assert_eq!(next, FIVE_MIN);
        assert_eq!(state.last_seen(), Some("9.19.0"));
    }

    #[test]
    fn duplicate_available_detection_skips_broadcast() {
        let mut state = PollState::default();
        let _ = state.on_success_available("9.19.0");
        let (should, next) = state.on_success_available("9.19.0");
        assert!(!should, "duplicate latest must not re-broadcast (FR-036)");
        assert_eq!(next, FIVE_MIN);
    }

    #[test]
    fn new_version_after_previous_detection_broadcasts_again() {
        let mut state = PollState::default();
        let _ = state.on_success_available("9.19.0");
        let (should, _) = state.on_success_available("9.20.0");
        assert!(
            should,
            "newer latest must broadcast even after a prior detection"
        );
        assert_eq!(state.last_seen(), Some("9.20.0"));
    }

    #[test]
    fn failure_ladder_advances_5_10_20_60_and_caps() {
        let mut state = PollState::default();
        assert_eq!(state.on_failure(), TEN_MIN); //  1 failure
        assert_eq!(state.on_failure(), TWENTY_MIN); // 2 failures
        assert_eq!(state.on_failure(), SIXTY_MIN); //  3 failures (cap)
        assert_eq!(state.on_failure(), SIXTY_MIN); //  4 failures (still cap)
        assert_eq!(state.on_failure(), SIXTY_MIN); //  5 failures (still cap)
    }

    #[test]
    fn success_uptodate_resets_backoff_after_failures() {
        let mut state = PollState::default();
        let _ = state.on_failure();
        let _ = state.on_failure();
        assert_eq!(state.current_interval(), TWENTY_MIN);
        assert_eq!(state.on_success_uptodate(), FIVE_MIN);
    }

    #[test]
    fn success_available_resets_backoff_after_failures() {
        let mut state = PollState::default();
        let _ = state.on_failure();
        let _ = state.on_failure();
        assert_eq!(state.current_interval(), TWENTY_MIN);
        let (should, next) = state.on_success_available("9.19.0");
        assert!(should);
        assert_eq!(next, FIVE_MIN);
    }

    #[derive(Debug)]
    struct FakeUpdateApplyOps {
        prepare_result: Result<PreparedPayload, String>,
        restart_args_result: Result<(), String>,
        helper_copy_result: Result<PathBuf, String>,
        portable_spawn_result: Result<(), String>,
        installer_spawn_result: Result<(), String>,
        wrote_restart_args: bool,
        spawned_portable: bool,
        spawned_installer: bool,
    }

    impl FakeUpdateApplyOps {
        fn portable(payload: PathBuf) -> Self {
            Self {
                prepare_result: Ok(PreparedPayload::PortableBinary { path: payload }),
                restart_args_result: Ok(()),
                helper_copy_result: Ok(PathBuf::from("/tmp/gwt-helper")),
                portable_spawn_result: Ok(()),
                installer_spawn_result: Ok(()),
                wrote_restart_args: false,
                spawned_portable: false,
                spawned_installer: false,
            }
        }
    }

    impl UpdateApplyOps for FakeUpdateApplyOps {
        fn prepare_update(
            &mut self,
            _latest: &str,
            _asset_url: &str,
        ) -> Result<PreparedPayload, String> {
            self.prepare_result.clone()
        }

        fn write_restart_args_file(
            &mut self,
            _path: &Path,
            _args: Vec<String>,
        ) -> Result<(), String> {
            self.wrote_restart_args = true;
            self.restart_args_result.clone()
        }

        fn make_helper_copy(
            &mut self,
            _current_exe: &Path,
            _latest: &str,
        ) -> Result<PathBuf, String> {
            self.helper_copy_result.clone()
        }

        fn spawn_internal_apply_update(
            &mut self,
            _helper_exe: &Path,
            _old_pid: u32,
            _current_exe: &Path,
            _payload: &Path,
            _args_file: &Path,
        ) -> Result<(), String> {
            self.spawned_portable = true;
            self.portable_spawn_result.clone()
        }

        fn spawn_internal_run_installer(
            &mut self,
            _helper_exe: &Path,
            _old_pid: u32,
            _current_exe: &Path,
            _installer: &Path,
            _kind: InstallerKind,
            _args_file: &Path,
        ) -> Result<(), String> {
            self.spawned_installer = true;
            self.installer_spawn_result.clone()
        }
    }

    fn available_update_state() -> UpdateState {
        UpdateState::Available {
            current: "9.20.3".to_string(),
            latest: "9.20.4".to_string(),
            release_url: "https://example.invalid/releases/v9.20.4".to_string(),
            asset_url: Some("https://example.invalid/gwt-macos-universal.dmg".to_string()),
            checked_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn update_apply_start_reports_prepare_update_failure() {
        let mut ops = FakeUpdateApplyOps::portable(PathBuf::from("/tmp/gwt-update"));
        ops.prepare_result = Err("download failed".to_string());

        let err = start_update_apply_with_ops(
            &mut ops,
            available_update_state(),
            Path::new("/Applications/GWT.app/Contents/MacOS/gwt"),
            Vec::new(),
            42,
            false,
        )
        .expect_err("prepare failure should be returned to the caller");

        assert!(err.contains("download failed"));
        assert!(!ops.wrote_restart_args);
        assert!(!ops.spawned_portable);
        assert!(!ops.spawned_installer);
    }

    #[test]
    fn update_apply_start_reports_restart_args_failure() {
        let mut ops = FakeUpdateApplyOps::portable(PathBuf::from("/tmp/gwt-update"));
        ops.restart_args_result = Err("restart args write failed".to_string());

        let err = start_update_apply_with_ops(
            &mut ops,
            available_update_state(),
            Path::new("/Applications/GWT.app/Contents/MacOS/gwt"),
            vec!["--project".to_string(), "/repo".to_string()],
            42,
            false,
        )
        .expect_err("restart args failure should be returned to the caller");

        assert!(err.contains("restart args write failed"));
        assert!(ops.wrote_restart_args);
        assert!(!ops.spawned_portable);
        assert!(!ops.spawned_installer);
    }

    #[test]
    fn update_apply_start_reports_spawn_failure() {
        let mut ops = FakeUpdateApplyOps::portable(PathBuf::from("/tmp/gwt-update"));
        ops.portable_spawn_result = Err("helper spawn failed".to_string());

        let err = start_update_apply_with_ops(
            &mut ops,
            available_update_state(),
            Path::new("/Applications/GWT.app/Contents/MacOS/gwt"),
            Vec::new(),
            42,
            false,
        )
        .expect_err("spawn failure should be returned to the caller");

        assert!(err.contains("helper spawn failed"));
        assert!(ops.wrote_restart_args);
        assert!(ops.spawned_portable);
        assert!(!ops.spawned_installer);
    }
}
