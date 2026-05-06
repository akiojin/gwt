use super::*;
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

/// Apply an already-detected update state from the GUI notification path.
///
/// The startup poll has already selected the platform-specific asset. Reusing
/// that state avoids a second network/cache check that can make a clicked toast
/// appear to do nothing when the re-check does not return `Available`.
pub fn apply_update_state_and_exit(state: gwt_core::update::UpdateState) {
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    apply_update_state_with_current_exe_and_exit(state, current_exe);
}

fn apply_update_state_with_current_exe_and_exit(
    state: gwt_core::update::UpdateState,
    current_exe: std::path::PathBuf,
) {
    let (latest, asset_url) = match state {
        gwt_core::update::UpdateState::Available {
            latest,
            asset_url: Some(asset_url),
            ..
        } => (latest, asset_url),
        _ => return,
    };
    apply_update_payload_and_exit(&latest, &asset_url, current_exe);
}

fn apply_update_payload_and_exit(latest: &str, asset_url: &str, current_exe: std::path::PathBuf) {
    let mgr = gwt_core::update::UpdateManager::new();
    let payload = match mgr.prepare_update(latest, asset_url) {
        Ok(p) => p,
        Err(_) => return,
    };
    let args_file = match &payload {
        gwt_core::update::PreparedPayload::PortableBinary { path }
        | gwt_core::update::PreparedPayload::Installer { path, .. } => {
            path.parent().map(|dir| dir.join("restart-args.json"))
        }
    };
    let Some(args_file) = args_file else {
        return;
    };
    let restart_args: Vec<String> = std::env::args().skip(1).collect();
    if mgr
        .write_restart_args_file(&args_file, restart_args)
        .is_err()
    {
        return;
    }
    let helper_exe = if cfg!(windows) {
        match mgr.make_helper_copy(&current_exe, latest) {
            Ok(path) => path,
            Err(_) => return,
        }
    } else {
        current_exe.clone()
    };
    let old_pid = std::process::id();
    let result = match payload {
        gwt_core::update::PreparedPayload::PortableBinary { path } => {
            mgr.spawn_internal_apply_update(&helper_exe, old_pid, &current_exe, &path, &args_file)
        }
        gwt_core::update::PreparedPayload::Installer { path, kind } => mgr
            .spawn_internal_run_installer(
                &helper_exe,
                old_pid,
                &current_exe,
                &path,
                kind,
                &args_file,
            ),
    };
    if result.is_ok() {
        std::process::exit(0);
    }
}

#[cfg(test)]
mod poll_state_tests {
    use super::PollState;
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
}
