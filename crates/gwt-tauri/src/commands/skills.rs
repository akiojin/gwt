use crate::state::AppState;
use gwt_core::config::{
    get_skill_registration_status_with_settings_at_project_root,
    repair_skill_registration_with_settings_at_project_root, Settings, SkillRegistrationStatus,
};
use gwt_core::StructuredError;
use std::path::PathBuf;
use tauri::{State, Window};

fn resolve_window_project_root(state: &AppState, window: &Window) -> Option<PathBuf> {
    let project_path = state
        .window_projects
        .lock()
        .ok()
        .and_then(|projects| projects.get(window.label()).cloned())?;
    let path = PathBuf::from(project_path);
    Some(dunce::canonicalize(&path).unwrap_or(path))
}

fn load_skill_registration_settings(command: &str) -> Result<Settings, StructuredError> {
    Settings::load_global().map_err(|e| StructuredError::from_gwt_error(&e, command))
}

#[tauri::command]
pub fn get_skill_registration_status_cmd(
    window: Window,
    state: State<AppState>,
) -> Result<SkillRegistrationStatus, StructuredError> {
    let project_root = resolve_window_project_root(&state, &window);
    let settings = load_skill_registration_settings("get_skill_registration_status")?;
    let status = get_skill_registration_status_with_settings_at_project_root(
        &settings,
        project_root.as_deref(),
    );
    state.set_skill_registration_status(status);
    Ok(state.get_skill_registration_status())
}

#[tauri::command]
pub fn repair_skill_registration_cmd(
    window: Window,
    state: State<AppState>,
) -> Result<SkillRegistrationStatus, StructuredError> {
    let project_root = resolve_window_project_root(&state, &window);
    let settings = load_skill_registration_settings("repair_skill_registration")?;
    let status =
        repair_skill_registration_with_settings_at_project_root(&settings, project_root.as_deref());
    state.set_skill_registration_status(status);
    Ok(state.get_skill_registration_status())
}
