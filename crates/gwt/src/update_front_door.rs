use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupUpdateAction {
    Publish,
    Stop,
    Retry,
}

pub(crate) fn classify_startup_update_state(
    state: &gwt_core::update::UpdateState,
) -> StartupUpdateAction {
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

pub(crate) fn spawn_startup_update_check(
    runtime: &Runtime,
    clients: ClientHub,
    update_proxy: EventLoopProxy<UserEvent>,
) {
    runtime.spawn(async move {
        if gwt_core::update::is_ci() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
        let current_exe = std::env::current_exe().ok();
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
            }
            let exe = current_exe.clone();
            let state = match tokio::task::spawn_blocking(move || {
                gwt_core::update::UpdateManager::new().check_for_executable(false, exe.as_deref())
            })
            .await
            {
                Ok(s) => s,
                Err(_) => break,
            };
            match classify_startup_update_state(&state) {
                StartupUpdateAction::Publish => {
                    let _ = update_proxy.send_event(UserEvent::UpdateAvailable(state.clone()));
                    clients.dispatch(vec![OutboundEvent::broadcast(BackendEvent::UpdateState(
                        state,
                    ))]);
                    return;
                }
                StartupUpdateAction::Stop => return,
                StartupUpdateAction::Retry => {}
            }
        }
    });
}

/// Download and apply a pending update, then exit.
///
/// Called from a background thread so the GUI remains responsive during download.
/// On success, this function calls `std::process::exit(0)` and never returns.
/// On any failure, it returns silently.
pub(crate) fn apply_update_and_exit() {
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let mgr = gwt_core::update::UpdateManager::new();
    let state = mgr.check_for_executable(false, Some(&current_exe));
    let (latest, asset_url) = match state {
        gwt_core::update::UpdateState::Available {
            latest,
            asset_url: Some(asset_url),
            ..
        } => (latest, asset_url),
        _ => return,
    };
    let payload = match mgr.prepare_update(&latest, &asset_url) {
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
        match mgr.make_helper_copy(&current_exe, &latest) {
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
