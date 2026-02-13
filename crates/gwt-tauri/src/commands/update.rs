//! App self-update commands (GitHub Releases).

use crate::state::AppState;
use chrono::Utc;
use gwt_core::update::PreparedPayload;
use gwt_core::update::UpdateState;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn check_app_update(
    state: State<'_, AppState>,
    force: bool,
) -> Result<UpdateState, String> {
    let mgr = state.update_manager.clone();
    let current_exe = std::env::current_exe().ok();
    let state = tauri::async_runtime::spawn_blocking(move || {
        mgr.check_for_executable(force, current_exe.as_deref())
    })
    .await
    .unwrap_or_else(|e| UpdateState::Failed {
        message: format!("Update check failed: {e}"),
        failed_at: Utc::now(),
    });
    Ok(state)
}

#[tauri::command]
pub async fn apply_app_update(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let mgr = state.update_manager.clone();

    let current_exe =
        std::env::current_exe().map_err(|e| format!("Failed to locate current executable: {e}"))?;

    let update_state = tauri::async_runtime::spawn_blocking({
        let mgr = mgr.clone();
        let current_exe = current_exe.clone();
        move || mgr.check_for_executable(true, Some(&current_exe))
    })
    .await
    .map_err(|e| format!("Update check failed: {e}"))?;

    let (latest, asset_url) = match update_state {
        UpdateState::Available {
            latest, asset_url, ..
        } => (latest, asset_url),
        UpdateState::UpToDate { .. } => return Err("Already up to date.".to_string()),
        UpdateState::Failed { message, .. } => return Err(message),
    };

    let asset_url =
        asset_url.ok_or_else(|| "No suitable update asset found for this platform.".to_string())?;

    let payload = tauri::async_runtime::spawn_blocking({
        let mgr = mgr.clone();
        let latest = latest.clone();
        let asset_url = asset_url.clone();
        move || mgr.prepare_update(&latest, &asset_url)
    })
    .await
    .map_err(|e| format!("Update download failed: {e}"))??;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let update_dir = match &payload {
        PreparedPayload::PortableBinary { path } => path
            .parent()
            .ok_or_else(|| "Invalid update payload path".to_string())?
            .to_path_buf(),
        PreparedPayload::Installer { path, .. } => path
            .parent()
            .ok_or_else(|| "Invalid update payload path".to_string())?
            .to_path_buf(),
    };
    let args_file = update_dir.join("restart-args.json");
    mgr.write_restart_args_file(&args_file, args)?;

    let helper_exe = if cfg!(windows) {
        mgr.make_helper_copy(&current_exe, &latest)?
    } else {
        current_exe.clone()
    };

    let old_pid = std::process::id();
    match payload {
        PreparedPayload::PortableBinary { path } => {
            mgr.spawn_internal_apply_update(&helper_exe, old_pid, &current_exe, &path, &args_file)?;
        }
        PreparedPayload::Installer { path, kind } => {
            mgr.spawn_internal_run_installer(
                &helper_exe,
                old_pid,
                &current_exe,
                &path,
                kind,
                &args_file,
            )?;
        }
    }

    // Ensure tray/close handlers allow actual exit.
    state.request_quit();
    app_handle.exit(0);
    Ok(())
}

// No unit tests here: update application requires platform integration.
