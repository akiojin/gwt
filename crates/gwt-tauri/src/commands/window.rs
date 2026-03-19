use tauri::{AppHandle, Manager, State, WebviewWindowBuilder, Window, Wry};
use tracing::{info, instrument, warn};
use uuid::Uuid;

use crate::state::AppState;

fn normalize_window_label(label: Option<String>) -> String {
    let trimmed = label.unwrap_or_default().trim().to_string();
    if !trimmed.is_empty() {
        trimmed
    } else {
        format!("project-{}", Uuid::new_v4())
    }
}

fn make_window_label(app: &AppHandle<Wry>, label: String) -> String {
    let mut candidate = label.trim().to_string();
    let mut suffix = 0;

    while app.get_webview_window(&candidate).is_some() {
        suffix += 1;
        candidate = format!("{}-{}", label.trim(), suffix);
    }

    candidate
}

fn open_window_with_label(app: &AppHandle<Wry>, label: &str) -> Result<(), String> {
    let mut conf = match app.config().app.windows.first() {
        Some(c) => c.clone(),
        None => {
            return Err("No window config found; cannot create window.".to_string());
        }
    };

    conf.label = label.to_string();
    let builder = WebviewWindowBuilder::from_config(app, &conf);
    let window = builder
        .map_err(|err| format!("Failed to create window: {err}"))?
        .build()
        .map_err(|err| format!("Failed to create window: {err}"))?;

    let _ = window.show();
    let _ = window.set_focus();
    let _ = crate::menu::rebuild_menu(app);
    Ok(())
}

#[instrument(skip_all, fields(command = "get_current_window_label", window_label = window.label()))]
#[tauri::command]
pub fn get_current_window_label(window: Window) -> String {
    window.label().to_string()
}

#[instrument(skip_all, fields(command = "try_acquire_window_restore_leader", window_label = label))]
#[tauri::command]
pub fn try_acquire_window_restore_leader(state: State<AppState>, label: String) -> bool {
    state.try_acquire_window_session_restore_leader(&label)
}

#[instrument(skip_all, fields(command = "release_window_restore_leader", window_label = label))]
#[tauri::command]
pub fn release_window_restore_leader(state: State<AppState>, label: String) {
    state.release_window_session_restore_leader(&label);
}

#[instrument(skip_all, fields(command = "open_gwt_window"))]
#[tauri::command]
pub fn open_gwt_window(app: AppHandle<Wry>, label: Option<String>) -> String {
    let requested = normalize_window_label(label);
    let final_label = make_window_label(&app, requested);

    let app_for_thread = app.clone();
    let label_for_thread = final_label.clone();

    std::thread::spawn(
        move || match open_window_with_label(&app_for_thread, &label_for_thread) {
            Ok(()) => {
                info!(
                    category = "tauri",
                    event = "WindowCreated",
                    label = %label_for_thread,
                    "Window created for session restore"
                );
            }
            Err(err) => {
                warn!(
                    category = "tauri",
                    event = "WindowCreateFailed",
                    label = %label_for_thread,
                    error = %err
                );
            }
        },
    );

    final_label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_window_label_defaults_when_empty() {
        assert!(normalize_window_label(None).starts_with("project-"));
        let fallback = normalize_window_label(Some("  ".to_string()));
        assert!(fallback.starts_with("project-"));
    }
}
