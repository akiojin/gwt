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
    Some(path.canonicalize().unwrap_or(path))
}

#[tauri::command]
pub fn get_skill_registration_status_cmd(
    window: Window,
    state: State<AppState>,
) -> Result<SkillRegistrationStatus, StructuredError> {
    let project_root = resolve_window_project_root(&state, &window);
    let settings = Settings::load_global().unwrap_or_else(|_| Settings::default());
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
    let settings = Settings::load_global().unwrap_or_else(|_| Settings::default());
    let status =
        repair_skill_registration_with_settings_at_project_root(&settings, project_root.as_deref());
    state.set_skill_registration_status(status);
    Ok(state.get_skill_registration_status())
}
