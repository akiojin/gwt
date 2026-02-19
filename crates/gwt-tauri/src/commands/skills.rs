use crate::state::AppState;
use gwt_core::config::{
    get_skill_registration_status, repair_skill_registration, SkillRegistrationStatus,
};
use tauri::State;

#[tauri::command]
pub fn get_skill_registration_status_cmd(
    state: State<AppState>,
) -> Result<SkillRegistrationStatus, String> {
    let status = get_skill_registration_status();
    state.set_skill_registration_status(status);
    Ok(state.get_skill_registration_status())
}

#[tauri::command]
pub fn repair_skill_registration_cmd(
    state: State<AppState>,
) -> Result<SkillRegistrationStatus, String> {
    let status = repair_skill_registration();
    state.set_skill_registration_status(status);
    Ok(state.get_skill_registration_status())
}
