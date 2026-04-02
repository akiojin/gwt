//! App — Update and View functions for the Elm Architecture.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use gwt_agent::{AgentDetector, AgentId, DetectedAgent, VersionCache};
use gwt_ai::{suggest_branch_name, AIClient};
use gwt_config::{AISettings, Settings};
use gwt_core::paths::gwt_cache_dir;
use gwt_notification::{Notification, Severity};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Tabs},
    Frame,
};

use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    input::voice::VoiceInputMessage,
    message::Message,
    model::{ActiveLayer, ManagementTab, Model, SessionLayout, SessionTabType},
    screens,
};

static WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Process a message and update the model (Elm: update).
pub fn update(model: &mut Model, msg: Message) {
    match msg {
        Message::Quit => {
            model.quit = true;
        }
        Message::ToggleLayer => {
            model.active_layer = match model.active_layer {
                ActiveLayer::Initialization => ActiveLayer::Initialization, // blocked
                ActiveLayer::Main => ActiveLayer::Management,
                ActiveLayer::Management => ActiveLayer::Main,
            };
        }
        Message::SwitchManagementTab(tab) => {
            model.management_tab = tab;
            model.active_layer = ActiveLayer::Management;
        }
        Message::NextSession => {
            if !model.sessions.is_empty() {
                model.active_session = (model.active_session + 1) % model.sessions.len();
            }
        }
        Message::PrevSession => {
            if !model.sessions.is_empty() {
                model.active_session = if model.active_session == 0 {
                    model.sessions.len() - 1
                } else {
                    model.active_session - 1
                };
            }
        }
        Message::SwitchSession(idx) => {
            if idx < model.sessions.len() {
                model.active_session = idx;
            }
        }
        Message::ToggleSessionLayout => {
            model.session_layout = match model.session_layout {
                SessionLayout::Tab => SessionLayout::Grid,
                SessionLayout::Grid => SessionLayout::Tab,
            };
        }
        Message::NewShell => {
            let idx = model.sessions.len();
            let session = crate::model::SessionTab {
                id: format!("shell-{idx}"),
                name: format!("Shell {}", idx + 1),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
            };
            model.sessions.push(session);
            model.active_session = idx;
        }
        Message::CloseSession => {
            if model.sessions.len() > 1 {
                model.sessions.remove(model.active_session);
                if model.active_session >= model.sessions.len() {
                    model.active_session = model.sessions.len() - 1;
                }
            }
        }
        Message::Resize(w, h) => {
            model.terminal_size = (w, h);
        }
        Message::PtyOutput(_pane_id, _data) => {
            // Phase 2: feed data into vt100 parser for the matching pane
        }
        Message::PushError(err) => {
            model.error_queue.push_back(err);
        }
        Message::Notify(notification) => {
            apply_notification(model, notification);
        }
        Message::DismissError => {
            model.error_queue.pop_front();
        }
        Message::Tick => {
            drain_notification_bus(model);
            tick_notification(model);
            // Forward tick to wizard (AI suggest spinner) when active
            if let Some(ref mut wizard) = model.wizard {
                if wizard.ai_suggest.loading {
                    screens::wizard::update(wizard, screens::wizard::WizardMessage::Tick);
                }
            }
            // Forward tick to voice input when recording/transcribing
            if model.voice.is_active() {
                crate::input::voice::update(&mut model.voice, VoiceInputMessage::Tick);
            }
        }
        Message::KeyInput(key) => {
            if model.active_layer == ActiveLayer::Initialization {
                route_key_to_initialization(model, key);
            } else if model.active_layer == ActiveLayer::Management {
                route_key_to_management(model, key);
            } else {
                forward_key_to_active_session(model, key);
            }
        }
        Message::MouseInput(_) => {
            // Phase 2: mouse routing
        }
        Message::Branches(msg) => {
            screens::branches::update(&mut model.branches, msg);
        }
        Message::Profiles(msg) => {
            screens::profiles::update(&mut model.profiles, msg);
        }
        Message::Issues(msg) => {
            screens::issues::update(&mut model.issues, msg);
        }
        Message::GitView(msg) => {
            screens::git_view::update(&mut model.git_view, msg);
        }
        Message::PrDashboard(msg) => {
            screens::pr_dashboard::update(&mut model.pr_dashboard, msg);
        }
        Message::Specs(msg) => {
            if matches!(msg, screens::specs::SpecsMessage::LaunchAgent) {
                let spec_context = model
                    .specs
                    .selected_spec()
                    .map(|spec| (spec.id.clone(), spec.title.clone()));
                if let Some((spec_id, title)) = spec_context {
                    update(model, Message::OpenWizardWithSpec(spec_id, title));
                }
            }
            screens::specs::update(&mut model.specs, msg);
        }
        Message::Settings(msg) => {
            screens::settings::update(&mut model.settings, msg);
        }
        Message::Logs(msg) => {
            screens::logs::update(&mut model.logs, msg);
        }
        Message::Versions(msg) => {
            screens::versions::update(&mut model.versions, msg);
        }
        Message::Wizard(msg) => {
            if let Some(ref mut wizard) = model.wizard {
                screens::wizard::update(wizard, msg);
                maybe_start_wizard_branch_suggestions(wizard);
                if wizard.completed || wizard.cancelled {
                    model.wizard = None;
                }
            }
        }
        Message::DockerProgress(msg) => {
            if let Some(ref mut state) = model.docker_progress {
                screens::docker_progress::update(state, msg);
            }
        }
        Message::ServiceSelect(msg) => {
            if let Some(ref mut state) = model.service_select {
                screens::service_select::update(state, msg);
                if !state.visible {
                    model.service_select = None;
                }
            }
        }
        Message::PortSelect(msg) => {
            if let Some(ref mut state) = model.port_select {
                screens::port_select::update(state, msg);
                if !state.visible {
                    model.port_select = None;
                }
            }
        }
        Message::Confirm(msg) => {
            screens::confirm::update(&mut model.confirm, msg);
        }
        Message::Voice(msg) => {
            let voice_enabled = Settings::load()
                .map(|settings| settings.voice.enabled)
                .unwrap_or(false);
            handle_voice_message(model, msg, voice_enabled);
        }
        Message::Initialization(msg) => {
            use screens::initialization::InitializationMessage;
            if let Some(ref mut state) = model.initialization {
                match msg {
                    InitializationMessage::Exit => {
                        model.quit = true;
                    }
                    InitializationMessage::StartClone => {
                        let url = state.url_input.clone();
                        let target = model.repo_path.clone();
                        state.clone_status = screens::initialization::CloneStatus::Cloning;
                        match gwt_git::clone_repo(&url, &target) {
                            Ok(path) => {
                                let _ = gwt_git::install_develop_protection(&path);
                                let _ = gwt_git::initialize_workspace(&path);
                                model.reset(path);
                                load_initial_data(model);
                            }
                            Err(e) => {
                                state.clone_status =
                                    screens::initialization::CloneStatus::Error(e.to_string());
                            }
                        }
                    }
                    other => {
                        screens::initialization::update(state, other);
                    }
                }
            }
        }
        Message::PasteFiles => {
            if let Some(bytes) = read_clipboard_input_bytes() {
                push_input_to_active_session(model, bytes);
            }
        }
        Message::OpenWizard => {
            open_wizard(model, None);
        }
        Message::OpenWizardWithSpec(spec_id, title) => {
            open_wizard(
                model,
                Some(crate::screens::wizard::SpecContext { spec_id, title }),
            );
        }
        Message::CloseWizard => {
            model.wizard = None;
        }
    }
}

/// Load initial data from the repository into the model.
///
/// Populates branches, specs, and version tags.  Each section is
/// best-effort: failures are silently ignored so the TUI still starts.
pub fn load_initial_data(model: &mut Model) {
    schedule_startup_version_cache_refresh();

    // -- Branches --
    if let Ok(branches) = gwt_git::branch::list_branches(&model.repo_path) {
        let items: Vec<screens::branches::BranchItem> = branches
            .iter()
            .map(|b| screens::branches::BranchItem {
                name: b.name.clone(),
                is_head: b.is_head,
                is_local: b.is_local,
                category: screens::branches::categorize_branch(&b.name),
            })
            .collect();
        screens::branches::update(
            &mut model.branches,
            screens::branches::BranchesMessage::SetBranches(items),
        );
    }

    // -- Specs (from specs/ directory metadata.json files) --
    model.specs.spec_root = Some(model.repo_path.clone());
    let specs_dir = model.repo_path.join("specs");
    if specs_dir.is_dir() {
        let mut spec_items: Vec<screens::specs::SpecItem> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&specs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let meta_path = path.join("metadata.json");
                    if let Ok(content) = std::fs::read_to_string(&meta_path) {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                            spec_items.push(screens::specs::SpecItem {
                                id: v["id"].as_str().unwrap_or("").to_string(),
                                title: v["title"].as_str().unwrap_or("").to_string(),
                                phase: v["phase"].as_str().unwrap_or("").to_string(),
                                status: v["status"].as_str().unwrap_or("").to_string(),
                            });
                        }
                    }
                }
            }
        }
        spec_items.sort_by(|a, b| a.id.cmp(&b.id));
        screens::specs::update(
            &mut model.specs,
            screens::specs::SpecsMessage::SetSpecs(spec_items),
        );
    }

    // -- Version tags --
    let repo_str = model.repo_path.to_string_lossy().to_string();
    if let Ok(output) = gwt_core::process::run_command(
        "git",
        &[
            "-C",
            &repo_str,
            "tag",
            "-l",
            "--sort=-v:refname",
            "--format=%(refname:short)\t%(creatordate:short)\t%(subject)",
        ],
    ) {
        let tags: Vec<screens::versions::VersionTag> = output
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                screens::versions::VersionTag {
                    name: parts.first().unwrap_or(&"").to_string(),
                    date: parts.get(1).unwrap_or(&"").to_string(),
                    message: parts.get(2).unwrap_or(&"").to_string(),
                }
            })
            .collect();
        screens::versions::update(
            &mut model.versions,
            screens::versions::VersionsMessage::SetTags(tags),
        );
    }
}

/// Route a key event to the initialization screen.
fn route_key_to_initialization(model: &mut Model, key: crossterm::event::KeyEvent) {
    use screens::initialization::InitializationMessage;

    let msg = match key.code {
        KeyCode::Esc => Some(Message::Initialization(InitializationMessage::Exit)),
        KeyCode::Enter => Some(Message::Initialization(InitializationMessage::StartClone)),
        KeyCode::Backspace => Some(Message::Initialization(InitializationMessage::Backspace)),
        KeyCode::Char(ch) => Some(Message::Initialization(InitializationMessage::InputChar(
            ch,
        ))),
        _ => None,
    };

    if let Some(m) = msg {
        update(model, m);
    }
}

/// Route a key event to the active management tab's screen message.
fn route_key_to_management(model: &mut Model, key: crossterm::event::KeyEvent) {
    use screens::branches::BranchesMessage;
    use screens::git_view::GitViewMessage;
    use screens::issues::IssuesMessage;
    use screens::logs::LogsMessage;
    use screens::pr_dashboard::PrDashboardMessage;
    use screens::profiles::ProfilesMessage;
    use screens::settings::SettingsMessage;
    use screens::specs::SpecsMessage;
    use screens::versions::VersionsMessage;

    // Global management keys: tab switching with Left/Right arrows
    // (only when not in text input mode like search or edit)
    if !is_in_text_input_mode(model) {
        let tab_count = ManagementTab::ALL.len();
        let idx = ManagementTab::ALL
            .iter()
            .position(|t| *t == model.management_tab)
            .unwrap_or(0);

        match key.code {
            KeyCode::Right => {
                model.management_tab = ManagementTab::ALL[(idx + 1) % tab_count];
                return;
            }
            KeyCode::Left => {
                model.management_tab =
                    ManagementTab::ALL[if idx == 0 { tab_count - 1 } else { idx - 1 }];
                return;
            }
            _ => {}
        }
    }

    // Tab-specific key routing
    match model.management_tab {
        ManagementTab::Branches => {
            if model.branches.search_active {
                let msg = match key.code {
                    KeyCode::Backspace => Some(BranchesMessage::SearchBackspace),
                    _ => search_input_char(&key).map(BranchesMessage::SearchInput),
                };
                if let Some(m) = msg {
                    screens::branches::update(&mut model.branches, m);
                    return;
                }
            }

            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(BranchesMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(BranchesMessage::MoveUp),
                KeyCode::Enter => Some(BranchesMessage::Select),
                KeyCode::Char('s') => Some(BranchesMessage::ToggleSort),
                KeyCode::Char('v') => Some(BranchesMessage::ToggleView),
                KeyCode::Char('/') => Some(BranchesMessage::SearchStart),
                KeyCode::Char('r') => {
                    load_initial_data(model);
                    return;
                }
                KeyCode::Esc => Some(BranchesMessage::SearchClear),
                _ => None,
            };
            if let Some(m) = msg {
                screens::branches::update(&mut model.branches, m);
            }
        }
        ManagementTab::Issues => {
            if model.issues.search_active {
                let msg = match key.code {
                    KeyCode::Backspace => Some(IssuesMessage::SearchBackspace),
                    _ => search_input_char(&key).map(IssuesMessage::SearchInput),
                };
                if let Some(m) = msg {
                    screens::issues::update(&mut model.issues, m);
                    return;
                }
            }

            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(IssuesMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(IssuesMessage::MoveUp),
                KeyCode::Enter => Some(IssuesMessage::ToggleDetail),
                KeyCode::Char('/') => Some(IssuesMessage::SearchStart),
                KeyCode::Char('r') => Some(IssuesMessage::Refresh),
                KeyCode::Esc => Some(IssuesMessage::SearchClear),
                _ => None,
            };
            if let Some(m) = msg {
                screens::issues::update(&mut model.issues, m);
            }
        }
        ManagementTab::Specs => {
            if model.specs.detail_editing {
                let msg = match key.code {
                    KeyCode::Enter => Some(SpecsMessage::SaveSectionEdit),
                    KeyCode::Esc => Some(SpecsMessage::CancelSectionEdit),
                    KeyCode::Backspace => Some(SpecsMessage::SectionEditBackspace),
                    _ => search_input_char(&key).map(SpecsMessage::SectionEditInput),
                };
                if let Some(m) = msg {
                    update(model, Message::Specs(m));
                }
                return;
            }

            if model.specs.editing {
                let msg = match key.code {
                    KeyCode::Enter => Some(SpecsMessage::SaveEdit),
                    KeyCode::Esc => Some(SpecsMessage::CancelEdit),
                    KeyCode::Backspace => Some(SpecsMessage::EditBackspace),
                    _ => search_input_char(&key).map(SpecsMessage::EditInput),
                };
                if let Some(m) = msg {
                    update(model, Message::Specs(m));
                }
                return;
            }

            if model.specs.search_active {
                let msg = match key.code {
                    KeyCode::Backspace => Some(SpecsMessage::SearchBackspace),
                    _ => search_input_char(&key).map(SpecsMessage::SearchInput),
                };
                if let Some(m) = msg {
                    update(model, Message::Specs(m));
                    return;
                }
            }

            let msg = match key.code {
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Some(SpecsMessage::LaunchAgent)
                }
                KeyCode::Char('j') | KeyCode::Down => Some(SpecsMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(SpecsMessage::MoveUp),
                KeyCode::Enter => Some(SpecsMessage::ToggleDetail),
                KeyCode::Tab => Some(SpecsMessage::NextSection),
                KeyCode::BackTab => Some(SpecsMessage::PrevSection),
                KeyCode::Char('e') => Some(if model.specs.detail_view {
                    SpecsMessage::StartSectionEdit
                } else {
                    SpecsMessage::StartEdit
                }),
                KeyCode::Char('/') => Some(SpecsMessage::SearchStart),
                KeyCode::Char('r') => {
                    load_initial_data(model);
                    return;
                }
                KeyCode::Esc => Some(if model.specs.detail_view {
                    SpecsMessage::ToggleDetail
                } else {
                    SpecsMessage::SearchClear
                }),
                _ => None,
            };
            if let Some(m) = msg {
                update(model, Message::Specs(m));
            }
        }
        ManagementTab::Settings => {
            if model.settings.editing {
                let msg = match key.code {
                    KeyCode::Enter => Some(SettingsMessage::EndEdit),
                    KeyCode::Esc => Some(SettingsMessage::CancelEdit),
                    KeyCode::Backspace => Some(SettingsMessage::Backspace),
                    KeyCode::Char(ch) => Some(SettingsMessage::InputChar(ch)),
                    _ => None,
                };
                if let Some(m) = msg {
                    screens::settings::update(&mut model.settings, m);
                }
            } else {
                let msg = match key.code {
                    KeyCode::Char('j') | KeyCode::Down => Some(SettingsMessage::MoveDown),
                    KeyCode::Char('k') | KeyCode::Up => Some(SettingsMessage::MoveUp),
                    KeyCode::Enter => Some(SettingsMessage::StartEdit),
                    KeyCode::Char(' ') => Some(SettingsMessage::ToggleBool),
                    KeyCode::Tab => Some(SettingsMessage::NextCategory),
                    KeyCode::BackTab => Some(SettingsMessage::PrevCategory),
                    KeyCode::Char('S') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        Some(SettingsMessage::Save)
                    }
                    _ => None,
                };
                if let Some(m) = msg {
                    screens::settings::update(&mut model.settings, m);
                }
            }
        }
        ManagementTab::Logs => {
            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(LogsMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(LogsMessage::MoveUp),
                KeyCode::Enter => Some(LogsMessage::ToggleDetail),
                KeyCode::Char('r') => Some(LogsMessage::Refresh),
                _ => None,
            };
            if let Some(m) = msg {
                screens::logs::update(&mut model.logs, m);
            }
        }
        ManagementTab::Versions => {
            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(VersionsMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(VersionsMessage::MoveUp),
                KeyCode::Char('r') => {
                    load_initial_data(model);
                    return;
                }
                _ => None,
            };
            if let Some(m) = msg {
                screens::versions::update(&mut model.versions, m);
            }
        }
        ManagementTab::GitView => {
            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(GitViewMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(GitViewMessage::MoveUp),
                KeyCode::Enter => Some(GitViewMessage::ToggleExpand),
                KeyCode::Char('r') => Some(GitViewMessage::Refresh),
                _ => None,
            };
            if let Some(m) = msg {
                screens::git_view::update(&mut model.git_view, m);
            }
        }
        ManagementTab::PrDashboard => {
            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(PrDashboardMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(PrDashboardMessage::MoveUp),
                KeyCode::Enter => Some(PrDashboardMessage::ToggleDetail),
                KeyCode::Char('r') => Some(PrDashboardMessage::Refresh),
                _ => None,
            };
            if let Some(m) = msg {
                screens::pr_dashboard::update(&mut model.pr_dashboard, m);
            }
        }
        ManagementTab::Profiles => {
            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(ProfilesMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(ProfilesMessage::MoveUp),
                KeyCode::Enter => Some(ProfilesMessage::ToggleActive),
                KeyCode::Char('n') => Some(ProfilesMessage::StartCreate),
                KeyCode::Char('e') => Some(ProfilesMessage::StartEdit),
                KeyCode::Char('d') => Some(ProfilesMessage::StartDelete),
                KeyCode::Esc => Some(ProfilesMessage::Cancel),
                _ => None,
            };
            if let Some(m) = msg {
                screens::profiles::update(&mut model.profiles, m);
            }
        }
    }
}

fn search_input_char(key: &crossterm::event::KeyEvent) -> Option<char> {
    if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }

    match key.code {
        KeyCode::Char(ch) => Some(ch),
        _ => None,
    }
}

fn forward_key_to_active_session(model: &mut Model, key: crossterm::event::KeyEvent) {
    let Some(bytes) = key_event_to_bytes(key) else {
        return;
    };
    push_input_to_active_session(model, bytes);
}

fn apply_notification(model: &mut Model, notification: Notification) {
    model.notification_log.push(notification.clone());
    let entries = notification_log_snapshot(model);
    screens::logs::update(
        &mut model.logs,
        screens::logs::LogsMessage::SetEntries(entries),
    );

    if let Some(msg) = crate::notification_router::route(&notification) {
        update(model, msg);
    }

    match notification.severity {
        Severity::Info => {
            model.current_notification = Some(notification);
            model.current_notification_ttl = Some(Duration::from_secs(5));
        }
        Severity::Warn => {
            model.current_notification = Some(notification);
            model.current_notification_ttl = None;
        }
        Severity::Debug | Severity::Error => {}
    }
}

fn notification_log_snapshot(model: &Model) -> Vec<screens::logs::LogEntry> {
    model
        .notification_log
        .entries()
        .into_iter()
        .cloned()
        .collect()
}

fn tick_notification(model: &mut Model) {
    let Some(ttl) = model.current_notification_ttl else {
        return;
    };

    let step = Duration::from_millis(100);
    if ttl <= step {
        model.current_notification = None;
        model.current_notification_ttl = None;
    } else {
        model.current_notification_ttl = Some(ttl - step);
    }
}

fn drain_notification_bus(model: &mut Model) {
    for notification in model.drain_notifications() {
        update(model, Message::Notify(notification));
    }
}

fn push_input_to_active_session(model: &mut Model, bytes: Vec<u8>) {
    let Some(session_id) = model.active_session_tab().map(|session| session.id.clone()) else {
        return;
    };

    model
        .pending_pty_inputs
        .push_back(crate::model::PendingPtyInput { session_id, bytes });
}

fn handle_voice_message(model: &mut Model, msg: VoiceInputMessage, voice_enabled: bool) {
    if matches!(msg, VoiceInputMessage::StartRecording) && !voice_enabled {
        return;
    }

    let transcription = match &msg {
        VoiceInputMessage::TranscriptionResult(text) => Some(text.clone()),
        _ => None,
    };
    crate::input::voice::update(&mut model.voice, msg);
    if let Some(text) = transcription.filter(|text| !text.trim().is_empty()) {
        push_input_to_active_session(model, text.into_bytes());
    }
}

fn maybe_start_wizard_branch_suggestions(wizard: &mut screens::wizard::WizardState) {
    maybe_start_wizard_branch_suggestions_with(wizard, request_branch_suggestions);
}

fn maybe_start_wizard_branch_suggestions_with<F>(
    wizard: &mut screens::wizard::WizardState,
    request: F,
) where
    F: FnOnce(&str) -> Result<Vec<String>, String>,
{
    if wizard.step != screens::wizard::WizardStep::AIBranchSuggest
        || !wizard.ai_suggest.loading
        || wizard.ai_suggest.tick_counter != 0
        || !wizard.ai_suggest.suggestions.is_empty()
        || wizard.ai_suggest.error.is_some()
    {
        return;
    }

    let context = wizard_branch_suggestion_context(wizard);
    let msg = match request(&context) {
        Ok(suggestions) => screens::wizard::WizardMessage::SetBranchSuggestions(suggestions),
        Err(err) => screens::wizard::WizardMessage::SetBranchSuggestError(err),
    };
    screens::wizard::update(wizard, msg);
}

fn wizard_branch_suggestion_context(wizard: &screens::wizard::WizardState) -> String {
    let mut parts = Vec::new();
    if let Some(summary) = wizard.spec_context_summary() {
        parts.push(format!("SPEC: {summary}"));
    }
    if !wizard.branch_name.trim().is_empty() {
        parts.push(format!(
            "Current branch seed: {}",
            wizard.branch_name.trim()
        ));
    }
    if !wizard.issue_id.trim().is_empty() {
        parts.push(format!("Issue: {}", wizard.issue_id.trim()));
    }
    if parts.is_empty() {
        "Create a concise git branch name for a new worktree task.".to_string()
    } else {
        parts.join("\n")
    }
}

fn request_branch_suggestions(context: &str) -> Result<Vec<String>, String> {
    let client = branch_suggestion_client()?;
    suggest_branch_name(&client, context).map_err(|err| err.to_string())
}

fn branch_suggestion_client() -> Result<AIClient, String> {
    if let Ok(settings) = Settings::load() {
        if let Some(ai_settings) = settings
            .profiles
            .active_profile()
            .and_then(|profile| profile.ai_settings.as_ref())
        {
            if ai_settings.is_enabled() {
                return ai_client_from_settings(ai_settings);
            }
        }
    }

    let endpoint = std::env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let model = std::env::var("OPENAI_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "AI branch suggestion requires active profile AI settings or OPENAI_MODEL".to_string()
        })?;
    let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();

    AIClient::new(&endpoint, &api_key, &model).map_err(|err| err.to_string())
}

fn ai_client_from_settings(settings: &AISettings) -> Result<AIClient, String> {
    AIClient::new(
        &settings.endpoint,
        settings.api_key.as_deref().unwrap_or(""),
        &settings.model,
    )
    .map_err(|err| err.to_string())
}

fn open_wizard(model: &mut Model, spec_context: Option<screens::wizard::SpecContext>) {
    let cache_path = wizard_version_cache_path();
    let cache = VersionCache::load(&cache_path);
    let detected_agents = AgentDetector::detect_all();
    let (wizard, refresh_targets) = prepare_wizard_startup(spec_context, detected_agents, &cache);

    model.wizard = Some(wizard);
    schedule_wizard_version_cache_refresh(cache_path, refresh_targets);
}

fn schedule_startup_version_cache_refresh() {
    schedule_startup_version_cache_refresh_with(
        wizard_version_cache_path(),
        AgentDetector::detect_all,
        schedule_wizard_version_cache_refresh,
    );
}

fn schedule_startup_version_cache_refresh_with<Detect, Schedule>(
    cache_path: PathBuf,
    detect_agents: Detect,
    schedule_refresh: Schedule,
) where
    Detect: FnOnce() -> Vec<DetectedAgent>,
    Schedule: FnOnce(PathBuf, Vec<AgentId>),
{
    let cache = VersionCache::load(&cache_path);
    let (_, refresh_targets) = build_wizard_agent_options(detect_agents(), &cache);
    schedule_refresh(cache_path, refresh_targets);
}

fn prepare_wizard_startup(
    spec_context: Option<screens::wizard::SpecContext>,
    detected_agents: Vec<DetectedAgent>,
    cache: &VersionCache,
) -> (screens::wizard::WizardState, Vec<AgentId>) {
    let branch_name = spec_context
        .as_ref()
        .map(|ctx| format!("feature/{}", ctx.spec_id.to_lowercase()))
        .unwrap_or_default();

    let mut wizard = screens::wizard::WizardState {
        branch_name,
        spec_context,
        ..Default::default()
    };

    let (agents, refresh_targets) = build_wizard_agent_options(detected_agents, cache);
    if !agents.is_empty() {
        screens::wizard::update(
            &mut wizard,
            screens::wizard::WizardMessage::SetAgents(agents),
        );
    }

    (wizard, refresh_targets)
}

fn build_wizard_agent_options(
    detected_agents: Vec<DetectedAgent>,
    cache: &VersionCache,
) -> (Vec<screens::wizard::AgentOption>, Vec<AgentId>) {
    let mut refresh_targets = Vec::new();
    let options = detected_agents
        .into_iter()
        .map(|detected| {
            let cached_versions = cached_agent_versions(cache, &detected.agent_id);
            let cache_refreshable = detected.agent_id.package_name().is_some();
            let cache_outdated = cache_refreshable && cache.needs_refresh(&detected.agent_id);
            if cache_outdated {
                refresh_targets.push(detected.agent_id.clone());
            }

            let versions = if cached_versions.is_empty() {
                detected.version.into_iter().collect()
            } else {
                cached_versions
            };

            screens::wizard::AgentOption {
                id: detected.agent_id.command().to_string(),
                name: detected.agent_id.display_name().to_string(),
                available: true,
                versions,
                cache_outdated,
            }
        })
        .collect();

    (options, refresh_targets)
}

fn cached_agent_versions(cache: &VersionCache, agent_id: &AgentId) -> Vec<String> {
    let key = version_cache_key(agent_id);
    cache
        .entries
        .get(&key)
        .map(|entry| entry.versions.clone())
        .unwrap_or_default()
}

fn version_cache_key(agent_id: &AgentId) -> String {
    match agent_id {
        AgentId::ClaudeCode => "claude-code".to_string(),
        AgentId::Codex => "codex".to_string(),
        AgentId::Gemini => "gemini".to_string(),
        AgentId::OpenCode => "opencode".to_string(),
        AgentId::Copilot => "copilot".to_string(),
        AgentId::Custom(name) => format!("custom-{name}"),
    }
}

fn wizard_version_cache_path() -> PathBuf {
    gwt_cache_dir().join("agent-versions.json")
}

fn schedule_wizard_version_cache_refresh(cache_path: PathBuf, refresh_targets: Vec<AgentId>) {
    schedule_wizard_version_cache_refresh_with(
        cache_path,
        refresh_targets,
        |task| {
            let _ = thread::spawn(task);
        },
        run_wizard_version_cache_refresh,
    );
}

fn schedule_wizard_version_cache_refresh_with<Spawn, Refresh>(
    cache_path: PathBuf,
    refresh_targets: Vec<AgentId>,
    spawn_task: Spawn,
    refresh_cache: Refresh,
) where
    Spawn: FnOnce(Box<dyn FnOnce() + Send>),
    Refresh: FnOnce(PathBuf, Vec<AgentId>) + Send + 'static,
{
    if refresh_targets.is_empty() {
        return;
    }

    if WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    spawn_task(Box::new(move || {
        refresh_cache(cache_path, refresh_targets);
        WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT.store(false, Ordering::Release);
    }));
}

fn run_wizard_version_cache_refresh(cache_path: PathBuf, refresh_targets: Vec<AgentId>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();

    if let Ok(runtime) = runtime {
        runtime.block_on(async move {
            let mut cache = VersionCache::load(&cache_path);
            let mut changed = false;

            for agent_id in refresh_targets {
                if !cache.needs_refresh(&agent_id) {
                    continue;
                }

                if let Ok(Some(_versions)) = cache.refresh(&agent_id).await {
                    changed = true;
                }
            }

            if changed {
                let _ = cache.save(&cache_path);
            }
        });
    }
}

fn read_clipboard_input_bytes() -> Option<Vec<u8>> {
    if let Ok(paths) = gwt_clipboard::ClipboardFilePaste::extract_file_paths() {
        if let Some(bytes) = clipboard_payload_to_bytes(&paths, "") {
            return Some(bytes);
        }
    }

    let text = gwt_clipboard::ClipboardText::get_text().ok()?;
    clipboard_payload_to_bytes(&[], &text)
}

fn clipboard_payload_to_bytes(paths: &[PathBuf], fallback_text: &str) -> Option<Vec<u8>> {
    if !paths.is_empty() {
        let payload = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        return Some(payload.into_bytes());
    }

    let text = fallback_text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.as_bytes().to_vec())
    }
}

fn key_event_to_bytes(key: crossterm::event::KeyEvent) -> Option<Vec<u8>> {
    match key.code {
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            control_char_bytes(ch)
        }
        KeyCode::Char(ch) => Some(ch.to_string().into_bytes()),
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        _ => None,
    }
}

fn control_char_bytes(ch: char) -> Option<Vec<u8>> {
    let ch = ch.to_ascii_lowercase();
    match ch {
        '@' | ' ' => Some(vec![0x00]),
        'a'..='z' => Some(vec![(ch as u8) & 0x1f]),
        '[' => Some(vec![0x1b]),
        '\\' => Some(vec![0x1c]),
        ']' => Some(vec![0x1d]),
        '^' => Some(vec![0x1e]),
        '_' => Some(vec![0x1f]),
        _ => None,
    }
}

/// Check if the active management screen is in a text input mode (search, edit).
fn is_in_text_input_mode(model: &Model) -> bool {
    match model.management_tab {
        ManagementTab::Branches => model.branches.search_active,
        ManagementTab::Issues => model.issues.search_active,
        ManagementTab::Specs => {
            model.specs.search_active || model.specs.editing || model.specs.detail_editing
        }
        ManagementTab::Settings => model.settings.editing,
        _ => false,
    }
}

/// Render the full UI (Elm: view).
pub fn view(model: &Model, frame: &mut Frame) {
    let size = frame.area();

    // Initialization layer is fullscreen — no management panel or sessions
    if model.active_layer == ActiveLayer::Initialization {
        if let Some(ref init_state) = model.initialization {
            screens::initialization::render(init_state, frame, size);
        }
        // Error overlay on top
        if !model.error_queue.is_empty() {
            screens::error::render(&model.error_queue, frame, size);
        }
        return;
    }

    if model.active_layer == ActiveLayer::Management {
        // Split: left = management panel, right = sessions
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(size);

        render_management_panel(model, frame, chunks[0]);
        render_sessions_area(model, frame, chunks[1]);
    } else {
        render_sessions_area(model, frame, size);
    }

    // Confirm dialog overlay
    screens::confirm::render(&model.confirm, frame, size);

    // Docker progress overlay
    if let Some(ref docker) = model.docker_progress {
        screens::docker_progress::render(docker, frame, size);
    }

    // Service selection overlay
    if let Some(ref svc) = model.service_select {
        screens::service_select::render(svc, frame, size);
    }

    // Port selection overlay
    if let Some(ref port) = model.port_select {
        screens::port_select::render(port, frame, size);
    }

    // Wizard overlay (on top of everything except errors)
    if let Some(ref wizard) = model.wizard {
        screens::wizard::render(wizard, frame, size);
    }

    // Error overlay on top of everything
    if !model.error_queue.is_empty() {
        screens::error::render(&model.error_queue, frame, size);
    }
}

/// Render the management panel (left side).
fn render_management_panel(model: &Model, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Tab bar
    let titles: Vec<Line> = ManagementTab::ALL
        .iter()
        .map(|t| Line::from(t.label()))
        .collect();

    let active_idx = ManagementTab::ALL
        .iter()
        .position(|t| *t == model.management_tab)
        .unwrap_or(0);

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Management"))
        .select(active_idx)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, chunks[0]);

    // Tab content
    render_management_tab_content(model, frame, chunks[1]);
}

/// Render the content of the active management tab.
fn render_management_tab_content(model: &Model, frame: &mut Frame, area: Rect) {
    match model.management_tab {
        ManagementTab::Branches => screens::branches::render(&model.branches, frame, area),
        ManagementTab::Specs => screens::specs::render(&model.specs, frame, area),
        ManagementTab::Issues => screens::issues::render(&model.issues, frame, area),
        ManagementTab::PrDashboard => {
            screens::pr_dashboard::render(&model.pr_dashboard, frame, area)
        }
        ManagementTab::Profiles => screens::profiles::render(&model.profiles, frame, area),
        ManagementTab::GitView => screens::git_view::render(&model.git_view, frame, area),
        ManagementTab::Versions => screens::versions::render(&model.versions, frame, area),
        ManagementTab::Settings => screens::settings::render(&model.settings, frame, area),
        ManagementTab::Logs => screens::logs::render(&model.logs, frame, area),
    }
}

/// Render the sessions area (right side, or full screen).
fn render_sessions_area(model: &Model, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Tab bar
            Constraint::Min(0),    // Terminal content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    // Session tab bar — delegate to widget
    crate::widgets::tab_bar::render(model, frame, chunks[0]);

    // Session content
    render_session_content(model, frame, chunks[1]);

    // Status bar — delegate to widget
    crate::widgets::status_bar::render(model, frame, chunks[2]);
}

/// Render session content (Tab mode = single, Grid mode = tiled).
fn render_session_content(model: &Model, frame: &mut Frame, area: Rect) {
    match model.session_layout {
        SessionLayout::Tab => {
            if let Some(session) = model.active_session_tab() {
                render_single_session(session, frame, area);
            }
        }
        SessionLayout::Grid => {
            render_grid_sessions(model, frame, area);
        }
    }
}

/// Render a single session pane.
fn render_single_session(session: &crate::model::SessionTab, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(session.name.as_str());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Phase 2: render vt100 screen buffer here
    let placeholder = Paragraph::new(format!(
        "Session: {} ({}x{})",
        session.name,
        session.vt.cols(),
        session.vt.rows()
    ))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(placeholder, inner);
}

/// Render sessions in a grid layout.
fn render_grid_sessions(model: &Model, frame: &mut Frame, area: Rect) {
    let count = model.sessions.len();
    if count == 0 {
        return;
    }

    let cols = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(cols);

    let row_constraints: Vec<Constraint> = (0..rows)
        .map(|_| Constraint::Ratio(1, rows as u32))
        .collect();

    let row_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(area);

    for (row_idx, row_area) in row_chunks.iter().enumerate() {
        let start = row_idx * cols;
        let end = (start + cols).min(count);
        let n = end - start;

        let col_constraints: Vec<Constraint> =
            (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();

        let col_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints)
            .split(*row_area);

        for (col_idx, col_area) in col_chunks.iter().enumerate() {
            let session_idx = start + col_idx;
            if let Some(session) = model.sessions.get(session_idx) {
                let is_active = session_idx == model.active_session;
                let border_style = if is_active {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(session.name.as_str());
                frame.render_widget(block, *col_area);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use crossterm::event::{KeyEvent, KeyEventKind, KeyEventState};
    use gwt_agent::{version_cache::VersionEntry, AgentId, DetectedAgent, VersionCache};
    use gwt_notification::{Notification, Severity};
    use std::path::PathBuf;

    fn test_model() -> Model {
        Model::new(PathBuf::from("/tmp/test"))
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn detected_agent(agent_id: AgentId, version: Option<&str>) -> DetectedAgent {
        DetectedAgent {
            agent_id,
            version: version.map(|value| value.to_string()),
            path: PathBuf::from("/tmp/fake-agent"),
        }
    }

    fn version_entry(versions: &[&str], age_seconds: i64) -> VersionEntry {
        VersionEntry {
            versions: versions.iter().map(|value| value.to_string()).collect(),
            updated_at: Utc::now() - Duration::seconds(age_seconds),
        }
    }

    #[test]
    fn update_quit_sets_flag() {
        let mut model = test_model();
        update(&mut model, Message::Quit);
        assert!(model.quit);
    }

    #[test]
    fn update_toggle_layer() {
        let mut model = test_model();
        assert_eq!(model.active_layer, ActiveLayer::Management);

        update(&mut model, Message::ToggleLayer);
        assert_eq!(model.active_layer, ActiveLayer::Main);

        update(&mut model, Message::ToggleLayer);
        assert_eq!(model.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn update_switch_management_tab() {
        let mut model = test_model();
        update(
            &mut model,
            Message::SwitchManagementTab(ManagementTab::Settings),
        );
        assert_eq!(model.management_tab, ManagementTab::Settings);
        assert_eq!(model.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn update_next_prev_session() {
        let mut model = test_model();
        // Add a second session
        update(&mut model, Message::NewShell);
        assert_eq!(model.active_session, 1);

        update(&mut model, Message::PrevSession);
        assert_eq!(model.active_session, 0);

        update(&mut model, Message::PrevSession);
        // Wraps to last
        assert_eq!(model.active_session, 1);

        update(&mut model, Message::NextSession);
        assert_eq!(model.active_session, 0);
    }

    #[test]
    fn update_switch_session_by_index() {
        let mut model = test_model();
        update(&mut model, Message::NewShell);
        update(&mut model, Message::NewShell);

        update(&mut model, Message::SwitchSession(0));
        assert_eq!(model.active_session, 0);

        update(&mut model, Message::SwitchSession(2));
        assert_eq!(model.active_session, 2);

        // Out of range — no change
        update(&mut model, Message::SwitchSession(99));
        assert_eq!(model.active_session, 2);
    }

    #[test]
    fn update_toggle_session_layout() {
        let mut model = test_model();
        assert_eq!(model.session_layout, SessionLayout::Tab);

        update(&mut model, Message::ToggleSessionLayout);
        assert_eq!(model.session_layout, SessionLayout::Grid);

        update(&mut model, Message::ToggleSessionLayout);
        assert_eq!(model.session_layout, SessionLayout::Tab);
    }

    #[test]
    fn update_new_shell_adds_session() {
        let mut model = test_model();
        assert_eq!(model.session_count(), 1);

        update(&mut model, Message::NewShell);
        assert_eq!(model.session_count(), 2);
        assert_eq!(model.active_session, 1);
        assert_eq!(model.sessions[1].name, "Shell 2");
    }

    #[test]
    fn update_close_session_removes_active() {
        let mut model = test_model();
        update(&mut model, Message::NewShell);
        update(&mut model, Message::NewShell);
        assert_eq!(model.session_count(), 3);
        assert_eq!(model.active_session, 2);

        update(&mut model, Message::CloseSession);
        assert_eq!(model.session_count(), 2);
        assert_eq!(model.active_session, 1);
    }

    #[test]
    fn update_close_session_wont_remove_last() {
        let mut model = test_model();
        assert_eq!(model.session_count(), 1);

        update(&mut model, Message::CloseSession);
        assert_eq!(model.session_count(), 1);
    }

    #[test]
    fn update_resize() {
        let mut model = test_model();
        update(&mut model, Message::Resize(120, 40));
        assert_eq!(model.terminal_size, (120, 40));
    }

    #[test]
    fn update_error_queue() {
        let mut model = test_model();
        update(&mut model, Message::PushError("e1".into()));
        update(&mut model, Message::PushError("e2".into()));
        assert_eq!(model.error_queue.len(), 2);

        update(&mut model, Message::DismissError);
        assert_eq!(model.error_queue.len(), 1);
        assert_eq!(model.error_queue.front().unwrap(), "e2");
    }

    #[test]
    fn update_dismiss_empty_error_queue_is_noop() {
        let mut model = test_model();
        update(&mut model, Message::DismissError);
        assert!(model.error_queue.is_empty());
    }

    #[test]
    fn prepare_wizard_startup_prefills_spec_context_and_versions() {
        let mut cache = VersionCache::new();
        cache.entries.insert(
            "claude-code".into(),
            version_entry(&["1.0.54", "1.0.53"], 60),
        );
        cache
            .entries
            .insert("codex".into(), version_entry(&["0.5.0"], 90_000));

        let detected = vec![
            detected_agent(AgentId::ClaudeCode, Some("1.0.55")),
            detected_agent(AgentId::Codex, Some("0.5.1")),
            detected_agent(AgentId::Gemini, Some("0.2.0")),
        ];

        let (wizard, refresh_targets) = prepare_wizard_startup(
            Some(screens::wizard::SpecContext {
                spec_id: "SPEC-42".into(),
                title: "My Feature".into(),
            }),
            detected,
            &cache,
        );

        assert_eq!(wizard.branch_name, "feature/spec-42");
        let ctx = wizard.spec_context.as_ref().unwrap();
        assert_eq!(ctx.spec_id, "SPEC-42");
        assert_eq!(ctx.title, "My Feature");
        assert_eq!(wizard.detected_agents.len(), 3);
        assert_eq!(wizard.detected_agents[0].versions, vec!["1.0.54", "1.0.53"]);
        assert_eq!(wizard.detected_agents[1].versions, vec!["0.5.0"]);
        assert!(wizard.detected_agents[1].cache_outdated);
        assert_eq!(wizard.detected_agents[2].versions, vec!["0.2.0"]);
        assert!(wizard.detected_agents[2].cache_outdated);
        assert_eq!(refresh_targets, vec![AgentId::Codex, AgentId::Gemini]);
    }

    #[test]
    fn prepare_wizard_startup_uses_detected_version_when_cache_is_missing() {
        let cache = VersionCache::new();
        let detected = vec![detected_agent(AgentId::ClaudeCode, Some("1.0.55"))];

        let (wizard, refresh_targets) = prepare_wizard_startup(None, detected, &cache);

        assert!(wizard.spec_context.is_none());
        assert!(wizard.branch_name.is_empty());
        assert_eq!(wizard.detected_agents.len(), 1);
        assert_eq!(wizard.detected_agents[0].versions, vec!["1.0.55"]);
        assert!(wizard.detected_agents[0].cache_outdated);
        assert_eq!(refresh_targets, vec![AgentId::ClaudeCode]);
    }

    #[test]
    fn schedule_startup_version_cache_refresh_with_schedules_stale_refreshable_agents() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("agent-versions.json");
        let mut cache = VersionCache::new();
        cache.entries.insert(
            "claude-code".into(),
            version_entry(&["1.0.54", "1.0.53"], 90_000),
        );
        cache
            .entries
            .insert("codex".into(), version_entry(&["0.5.0"], 60));
        cache.save(&cache_path).unwrap();

        let scheduled = std::cell::RefCell::new(None);
        schedule_startup_version_cache_refresh_with(
            cache_path.clone(),
            || {
                vec![
                    detected_agent(AgentId::ClaudeCode, Some("1.0.55")),
                    detected_agent(AgentId::Codex, Some("0.5.1")),
                    detected_agent(AgentId::OpenCode, Some("0.2.0")),
                ]
            },
            |path, targets| {
                *scheduled.borrow_mut() = Some((path, targets));
            },
        );

        let (scheduled_path, targets) = scheduled.into_inner().unwrap();
        assert_eq!(scheduled_path, cache_path);
        assert_eq!(targets, vec![AgentId::ClaudeCode]);
    }

    #[test]
    fn schedule_startup_version_cache_refresh_with_schedules_missing_cache_entries() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("agent-versions.json");
        let scheduled = std::cell::RefCell::new(None);

        schedule_startup_version_cache_refresh_with(
            cache_path.clone(),
            || {
                vec![
                    detected_agent(AgentId::Gemini, Some("0.2.0")),
                    detected_agent(AgentId::OpenCode, Some("0.4.0")),
                ]
            },
            |path, targets| {
                *scheduled.borrow_mut() = Some((path, targets));
            },
        );

        let (scheduled_path, targets) = scheduled.into_inner().unwrap();
        assert_eq!(scheduled_path, cache_path);
        assert_eq!(targets, vec![AgentId::Gemini]);
    }

    #[test]
    fn schedule_wizard_version_cache_refresh_with_defers_refresh_until_spawned_task_runs() {
        WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT.store(false, Ordering::Release);

        let spawned = std::cell::RefCell::new(None::<Box<dyn FnOnce() + Send>>);
        let refreshed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let refreshed_flag = refreshed.clone();

        schedule_wizard_version_cache_refresh_with(
            PathBuf::from("/tmp/agent-versions.json"),
            vec![AgentId::ClaudeCode],
            |task| {
                *spawned.borrow_mut() = Some(task);
            },
            move |_, _| {
                refreshed_flag.store(true, Ordering::Release);
            },
        );

        assert!(!refreshed.load(Ordering::Acquire));
        assert!(spawned.borrow().is_some());
        assert!(WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT.load(Ordering::Acquire));

        let task = spawned.borrow_mut().take().unwrap();
        task();

        assert!(refreshed.load(Ordering::Acquire));
        assert!(!WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT.load(Ordering::Acquire));
    }

    #[test]
    fn route_key_to_management_specs_launch_agent_opens_wizard() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-9".into(),
            title: "Docker wizard".into(),
            phase: "implementation".into(),
            status: "open".into(),
        }];

        route_key_to_management(&mut model, key(KeyCode::Enter, KeyModifiers::SHIFT));

        let wizard = model.wizard.as_ref().unwrap();
        assert_eq!(wizard.spec_context.as_ref().unwrap().spec_id, "SPEC-9");
        assert_eq!(wizard.spec_context.as_ref().unwrap().title, "Docker wizard");
    }

    #[test]
    fn route_key_to_management_specs_start_phase_edit() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-3".into(),
            title: "Voice commands".into(),
            phase: "draft".into(),
            status: "open".into(),
        }];

        route_key_to_management(&mut model, key(KeyCode::Char('e'), KeyModifiers::NONE));

        assert!(model.specs.editing);
        assert_eq!(model.specs.edit_field, "draft");
    }

    #[test]
    fn maybe_start_wizard_branch_suggestions_with_applies_result() {
        let mut wizard = screens::wizard::WizardState::default();
        wizard.step = screens::wizard::WizardStep::AIBranchSuggest;
        wizard.ai_suggest.loading = true;
        wizard.spec_context = Some(screens::wizard::SpecContext {
            spec_id: "SPEC-42".into(),
            title: "My Feature".into(),
        });

        maybe_start_wizard_branch_suggestions_with(&mut wizard, |_| {
            Ok(vec!["feature/spec-42-my-feature".into()])
        });

        assert!(!wizard.ai_suggest.loading);
        assert_eq!(
            wizard.ai_suggest.suggestions,
            vec!["feature/spec-42-my-feature".to_string()]
        );
    }

    #[test]
    fn maybe_start_wizard_branch_suggestions_with_applies_error() {
        let mut wizard = screens::wizard::WizardState::default();
        wizard.step = screens::wizard::WizardStep::AIBranchSuggest;
        wizard.ai_suggest.loading = true;

        maybe_start_wizard_branch_suggestions_with(&mut wizard, |_| {
            Err("missing AI configuration".to_string())
        });

        assert!(!wizard.ai_suggest.loading);
        assert_eq!(
            wizard.ai_suggest.error.as_deref(),
            Some("missing AI configuration")
        );
    }

    #[test]
    fn wizard_branch_suggestion_context_includes_spec_and_branch_seed() {
        let mut wizard = screens::wizard::WizardState::default();
        wizard.branch_name = "feature/spec-7-voice".into();
        wizard.issue_id = "1776".into();
        wizard.spec_context = Some(screens::wizard::SpecContext {
            spec_id: "SPEC-7".into(),
            title: "Voice settings".into(),
        });

        let context = wizard_branch_suggestion_context(&wizard);

        assert!(context.contains("SPEC: SPEC-7 - Voice settings"));
        assert!(context.contains("Current branch seed: feature/spec-7-voice"));
        assert!(context.contains("Issue: 1776"));
    }

    #[test]
    fn update_key_input_in_main_layer_queues_pty_bytes() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        );

        let forwarded = model.pending_pty_inputs().back().unwrap();
        assert_eq!(forwarded.session_id, "shell-0");
        assert_eq!(forwarded.bytes, vec![0x03]);
    }

    #[test]
    fn update_voice_transcription_result_queues_pty_bytes() {
        let mut model = test_model();
        handle_voice_message(
            &mut model,
            VoiceInputMessage::TranscriptionResult("git status".into()),
            true,
        );

        let forwarded = model.pending_pty_inputs().back().unwrap();
        assert_eq!(forwarded.session_id, "shell-0");
        assert_eq!(forwarded.bytes, b"git status".to_vec());
        assert_eq!(model.voice.buffer, "git status");
    }

    #[test]
    fn update_voice_transcription_result_ignores_empty_text() {
        let mut model = test_model();
        handle_voice_message(
            &mut model,
            VoiceInputMessage::TranscriptionResult("   ".into()),
            true,
        );

        assert!(model.pending_pty_inputs().is_empty());
        assert_eq!(model.voice.buffer, "   ");
    }

    #[test]
    fn handle_voice_start_recording_is_noop_when_disabled() {
        let mut model = test_model();

        handle_voice_message(&mut model, VoiceInputMessage::StartRecording, false);

        assert_eq!(model.voice.status, crate::input::voice::VoiceStatus::Idle);
        assert!(model.pending_pty_inputs().is_empty());
    }

    #[test]
    fn handle_voice_start_recording_transitions_when_enabled() {
        let mut model = test_model();

        handle_voice_message(&mut model, VoiceInputMessage::StartRecording, true);

        assert_eq!(
            model.voice.status,
            crate::input::voice::VoiceStatus::Recording
        );
    }

    #[test]
    fn clipboard_payload_to_bytes_prefers_paths() {
        let bytes = clipboard_payload_to_bytes(
            &[
                std::path::PathBuf::from("/tmp/one.txt"),
                std::path::PathBuf::from("/tmp/two.txt"),
            ],
            "",
        )
        .unwrap();

        assert_eq!(bytes, b"/tmp/one.txt\n/tmp/two.txt".to_vec());
    }

    #[test]
    fn clipboard_payload_to_bytes_falls_back_to_text() {
        let bytes = clipboard_payload_to_bytes(&[], "echo hello").unwrap();
        assert_eq!(bytes, b"echo hello".to_vec());
    }

    #[test]
    fn clipboard_payload_to_bytes_ignores_empty_payload() {
        assert!(clipboard_payload_to_bytes(&[], "   ").is_none());
    }

    #[test]
    fn update_notify_info_sets_status_notification_and_log() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Info, "core", "Started");

        update(&mut model, Message::Notify(notification));

        assert!(model.current_notification.is_some());
        assert_eq!(model.logs.entries.len(), 1);
        assert_eq!(model.logs.entries[0].message, "Started");
        assert!(model.error_queue.is_empty());
    }

    #[test]
    fn update_notify_warn_persists_across_ticks() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Warn, "git", "Detached HEAD");

        update(&mut model, Message::Notify(notification));
        for _ in 0..60 {
            update(&mut model, Message::Tick);
        }

        assert!(model.current_notification.is_some());
        assert_eq!(
            model.current_notification.as_ref().unwrap().message,
            "Detached HEAD"
        );
    }

    #[test]
    fn update_notify_info_auto_dismisses_after_timeout() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Info, "core", "Started");

        update(&mut model, Message::Notify(notification));
        for _ in 0..50 {
            update(&mut model, Message::Tick);
        }

        assert!(model.current_notification.is_none());
    }

    #[test]
    fn update_notify_error_routes_to_error_queue_and_log() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Error, "pty", "Crashed");

        update(&mut model, Message::Notify(notification));

        assert_eq!(model.error_queue.len(), 1);
        assert_eq!(model.error_queue.front().unwrap(), "Crashed");
        assert_eq!(model.logs.entries.len(), 1);
        assert_eq!(model.logs.entries[0].source, "pty");
        assert!(model.current_notification.is_none());
    }

    #[test]
    fn update_tick_drains_notification_bus_into_notify_flow() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Info, "bus", "Queued");

        assert!(model.notification_bus_handle().send(notification));

        update(&mut model, Message::Tick);

        assert_eq!(model.logs.entries.len(), 1);
        assert_eq!(model.logs.entries[0].message, "Queued");
        assert!(model.current_notification.is_some());
        assert!(model.drain_notifications().is_empty());
    }

    #[test]
    fn route_key_to_management_routes_search_input_for_issues() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Issues;
        model.issues.search_active = true;

        route_key_to_management(&mut model, key(KeyCode::Char('b'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Char('u'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Backspace, KeyModifiers::NONE));

        assert_eq!(model.issues.search_query, "b");
    }

    #[test]
    fn update_toggle_layer_blocked_in_initialization() {
        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        assert_eq!(model.active_layer, ActiveLayer::Initialization);

        update(&mut model, Message::ToggleLayer);
        assert_eq!(model.active_layer, ActiveLayer::Initialization); // stays
    }

    #[test]
    fn update_initialization_exit_quits() {
        use crate::screens::initialization::InitializationMessage;

        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        update(
            &mut model,
            Message::Initialization(InitializationMessage::Exit),
        );
        assert!(model.quit);
    }

    #[test]
    fn route_key_to_initialization_esc_exits() {
        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        route_key_to_initialization(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(model.quit);
    }

    #[test]
    fn route_key_to_initialization_char_input() {
        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        route_key_to_initialization(&mut model, key(KeyCode::Char('h'), KeyModifiers::NONE));
        let init = model.initialization.as_ref().unwrap();
        assert_eq!(init.url_input, "h");
    }

    #[test]
    fn route_key_to_initialization_backspace() {
        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        route_key_to_initialization(&mut model, key(KeyCode::Char('a'), KeyModifiers::NONE));
        route_key_to_initialization(&mut model, key(KeyCode::Char('b'), KeyModifiers::NONE));
        route_key_to_initialization(&mut model, key(KeyCode::Backspace, KeyModifiers::NONE));
        let init = model.initialization.as_ref().unwrap();
        assert_eq!(init.url_input, "a");
    }

    #[test]
    fn route_key_to_management_routes_search_input_for_specs() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.search_active = true;

        route_key_to_management(&mut model, key(KeyCode::Char('s'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Char('p'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Backspace, KeyModifiers::NONE));

        assert_eq!(model.specs.search_query, "s");
    }
}
