//! App self-update commands (GitHub Releases).

use crate::state::AppState;
use chrono::Utc;
use gwt_core::update::UpdateState;
use std::path::Path;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn check_app_update(
    state: State<'_, AppState>,
    force: bool,
) -> Result<UpdateState, String> {
    let mgr = state.update_manager.clone();
    let state = tauri::async_runtime::spawn_blocking(move || mgr.check(force))
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
    ensure_dir_writable(
        current_exe
            .parent()
            .ok_or_else(|| "Current executable path has no parent dir".to_string())?,
    )?;

    let update_state = tauri::async_runtime::spawn_blocking({
        let mgr = mgr.clone();
        move || mgr.check(true)
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

    let payload_path = tauri::async_runtime::spawn_blocking({
        let mgr = mgr.clone();
        let latest = latest.clone();
        let asset_url = asset_url.clone();
        move || mgr.prepare_update(&latest, &asset_url)
    })
    .await
    .map_err(|e| format!("Update download failed: {e}"))??;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args_file = payload_path
        .parent()
        .map(|p| p.join("restart-args.json"))
        .ok_or_else(|| "Invalid update payload path".to_string())?;
    mgr.write_restart_args_file(&args_file, args)?;

    let helper_exe = if cfg!(windows) {
        mgr.make_helper_copy(&current_exe, &latest)?
    } else {
        current_exe.clone()
    };

    mgr.spawn_internal_apply_update(&helper_exe, &current_exe, &payload_path, &args_file)?;

    // Ensure tray/close handlers allow actual exit.
    state.request_quit();
    app_handle.exit(0);
    Ok(())
}

fn ensure_dir_writable(dir: &Path) -> Result<(), String> {
    let test_path = dir.join(format!(".gwt-write-test-{}", std::process::id()));
    std::fs::write(&test_path, b"test")
        .map_err(|e| format!("Update requires write access to {}: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&test_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_dir_writable_fails_for_missing_dir() {
        let missing = std::path::PathBuf::from("/__gwt_missing_dir__/__does_not_exist__");
        ensure_dir_writable(&missing).expect_err("should fail");
    }
}
