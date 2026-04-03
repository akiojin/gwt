//! App — Update and View functions for the Elm Architecture.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gwt_agent::{
    AgentDetector, AgentId, AgentLaunchBuilder, DetectedAgent, LaunchConfig,
    Session as AgentSession, VersionCache,
};
use gwt_ai::{suggest_branch_name, AIClient};
use gwt_config::{AISettings, Settings};
use gwt_core::paths::{gwt_cache_dir, gwt_sessions_dir};
use gwt_notification::{Notification, Severity};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::{
    input::voice::VoiceInputMessage,
    message::Message,
    model::{
        ActiveLayer, DockerProgressQueue, DockerProgressResult, FocusPane, ManagementTab, Model,
        PendingSessionConversion, SessionLayout, SessionTabType,
    },
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
            model.active_focus = FocusPane::TabContent;
            if tab == ManagementTab::Settings && model.settings.fields.is_empty() {
                model.settings.load_category_fields();
            }
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
        Message::PtyOutput(pane_id, data) => {
            if let Some(session) = model.session_tab_mut(&pane_id) {
                session.vt.process(&data);
            }
        }
        Message::PushError(err) => {
            model
                .error_queue
                .push_back(Notification::new(Severity::Error, "app", err));
        }
        Message::PushErrorNotification(notification) => {
            model.error_queue.push_back(notification);
        }
        Message::Notify(notification) => {
            apply_notification(model, notification);
        }
        Message::ToggleHelp => {
            model.help_visible = !model.help_visible;
        }
        Message::ShowNotification(notification) => match notification.severity {
            Severity::Info => {
                model.current_notification = Some(notification);
                model.current_notification_ttl = Some(Duration::from_secs(5));
            }
            Severity::Warn => {
                model.current_notification = Some(notification);
                model.current_notification_ttl = None;
            }
            Severity::Debug | Severity::Error => {}
        },
        Message::DismissNotification => {
            model.current_notification = None;
            model.current_notification_ttl = None;
        }
        Message::DismissError => {
            model.error_queue.pop_front();
        }
        Message::Tick => {
            drain_notification_bus(model);
            drain_docker_progress_events(model);
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
            if route_overlay_key(model, key) {
                return;
            }

            if model.active_layer == ActiveLayer::Initialization {
                route_key_to_initialization(model, key);
            } else if model.active_layer == ActiveLayer::Management {
                // Focus cycling with Tab/BackTab (before pane-specific dispatch)
                if !is_in_text_input_mode(model) {
                    match key.code {
                        KeyCode::Tab if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                            model.active_focus = model.active_focus.next();
                            return;
                        }
                        KeyCode::BackTab => {
                            model.active_focus = model.active_focus.prev();
                            return;
                        }
                        _ => {}
                    }
                }

                // Dispatch based on focused pane
                match model.active_focus {
                    FocusPane::TabContent => route_key_to_management(model, key),
                    FocusPane::BranchDetail => route_key_to_branch_detail(model, key),
                    FocusPane::Terminal => forward_key_to_active_session(model, key),
                }

                // Check pending actions after key dispatch
                check_branch_pending_actions(model);
            } else {
                forward_key_to_active_session(model, key);
            }
        }
        Message::MouseInput(mouse) => {
            handle_mouse_input(model, mouse);
        }
        Message::Branches(msg) => {
            screens::branches::update(&mut model.branches, msg);
            check_branch_pending_actions(model);
            if let Some(action) = model.branches.pending_docker_action.take() {
                handle_pending_branch_docker_action(model, action);
            }
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
        Message::Settings(msg) => {
            screens::settings::update(&mut model.settings, msg);
            if model.settings.category == screens::settings::SettingsCategory::Skills {
                for field in &model.settings.fields {
                    let enabled = field.value == "true";
                    let _ = model.embedded_skills.set_enabled(&field.label, enabled);
                }
            }
        }
        Message::Logs(msg) => {
            screens::logs::update(&mut model.logs, msg);
        }
        Message::Versions(msg) => {
            screens::versions::update(&mut model.versions, msg);
        }
        Message::Wizard(msg) => {
            let launch_config = if let Some(ref mut wizard) = model.wizard {
                screens::wizard::update(wizard, msg);
                maybe_start_wizard_branch_suggestions(wizard);
                let completed = wizard.completed;
                let launch_config = if completed {
                    Some(build_launch_config_from_wizard(wizard))
                } else {
                    None
                };
                if wizard.completed || wizard.cancelled {
                    model.wizard = None;
                }
                launch_config
            } else {
                None
            };
            if let Some(config) = launch_config {
                model.pending_launch_config = Some(config);
                materialize_pending_launch(model);
                model.active_focus = FocusPane::Terminal;
            }
        }
        Message::DockerProgress(msg) => {
            let should_create = matches!(
                msg,
                screens::docker_progress::DockerProgressMessage::SetStage { .. }
                    | screens::docker_progress::DockerProgressMessage::Advance
                    | screens::docker_progress::DockerProgressMessage::SetError(_)
            );
            if model.docker_progress.is_none() && should_create {
                let mut state = screens::docker_progress::DockerProgressState::default();
                state.show();
                model.docker_progress = Some(state);
            }
            if let Some(ref mut state) = model.docker_progress {
                let hide_after = matches!(
                    msg,
                    screens::docker_progress::DockerProgressMessage::Hide
                        | screens::docker_progress::DockerProgressMessage::Reset
                );
                screens::docker_progress::update(state, msg);
                if hide_after || !state.visible {
                    model.docker_progress = None;
                }
            }
        }
        Message::ServiceSelect(msg) => {
            let selected_conversion =
                if matches!(msg, screens::service_select::ServiceSelectMessage::Select) {
                    model.service_select.as_ref().and_then(|state| {
                        state
                            .current_selection()
                            .map(|(service, value)| PendingSessionConversion {
                                session_index: model.active_session,
                                target_agent_id: value.to_string(),
                                target_display_name: service.to_string(),
                            })
                    })
                } else {
                    None
                };
            let cancelled = matches!(msg, screens::service_select::ServiceSelectMessage::Cancel);
            if let Some(ref mut state) = model.service_select {
                screens::service_select::update(state, msg);
                if !state.visible {
                    model.service_select = None;
                }
            }
            if cancelled {
                model.pending_session_conversion = None;
            }
            if let Some(pending) = selected_conversion {
                model.confirm = screens::confirm::ConfirmState::with_message(format!(
                    "Convert session to {}?",
                    pending.target_display_name
                ));
                model.pending_session_conversion = Some(pending);
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
            handle_confirm_message(model, msg);
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
        Message::OpenSessionConversion => {
            open_session_conversion(model);
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
/// Populates branches, version tags, and worktree mappings.  Each section is
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
                worktree_path: None,
            })
            .collect();
        screens::branches::update(
            &mut model.branches,
            screens::branches::BranchesMessage::SetBranches(items),
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

    // -- Worktree → branch mapping --
    if let Ok(worktrees) = gwt_git::WorktreeManager::new(&model.repo_path).list() {
        for wt in &worktrees {
            if let Some(ref branch_name) = wt.branch {
                // Match worktree branch to existing BranchItem
                if let Some(item) = model
                    .branches
                    .branches
                    .iter_mut()
                    .find(|b| &b.name == branch_name)
                {
                    item.worktree_path = Some(wt.path.clone());
                }
            }
        }
    }

    // Load detail for initially selected branch
    let repo_path = model.repo_path.clone();
    screens::branches::load_branch_detail(&mut model.branches, &repo_path);

    // -- Git View --
    if let Ok(entries) = gwt_git::diff::get_status(&model.repo_path) {
        let files = entries
            .into_iter()
            .map(|entry| {
                let diff_preview = entry.diff_content(&model.repo_path).unwrap_or_default();
                screens::git_view::GitFileItem {
                    path: entry.path.display().to_string(),
                    status: match entry.status {
                        gwt_git::diff::FileStatus::Staged => screens::git_view::FileStatus::Staged,
                        gwt_git::diff::FileStatus::Unstaged => {
                            screens::git_view::FileStatus::Unstaged
                        }
                        gwt_git::diff::FileStatus::Untracked => {
                            screens::git_view::FileStatus::Untracked
                        }
                    },
                    diff_preview,
                }
            })
            .collect();
        screens::git_view::update(
            &mut model.git_view,
            screens::git_view::GitViewMessage::SetFiles(files),
        );
    }

    if let Ok(entries) = gwt_git::commit::recent_commits(&model.repo_path, 10) {
        let commits = entries
            .into_iter()
            .map(|entry| screens::git_view::GitCommitItem {
                hash: entry.hash,
                subject: entry.subject,
                author: entry.author,
                date: entry.timestamp.chars().take(10).collect(),
            })
            .collect();
        screens::git_view::update(
            &mut model.git_view,
            screens::git_view::GitViewMessage::SetCommits(commits),
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

fn route_overlay_key(model: &mut Model, key: crossterm::event::KeyEvent) -> bool {
    if model.help_visible {
        if key.code == KeyCode::Esc {
            update(model, Message::ToggleHelp);
        }
        return true;
    }

    // Wizard overlay takes priority (fullscreen modal)
    if model.wizard.is_some() {
        let msg = match key.code {
            KeyCode::Down => Some(screens::wizard::WizardMessage::MoveDown),
            KeyCode::Up => Some(screens::wizard::WizardMessage::MoveUp),
            KeyCode::Enter => Some(screens::wizard::WizardMessage::Select),
            KeyCode::Esc => Some(screens::wizard::WizardMessage::Back),
            KeyCode::Backspace => Some(screens::wizard::WizardMessage::Backspace),
            KeyCode::Char(ch) => Some(screens::wizard::WizardMessage::InputChar(ch)),
            _ => None,
        };
        if let Some(msg) = msg {
            update(model, Message::Wizard(msg));
        }
        return true; // Always consume keys when wizard is open
    }

    // Error overlay
    if !model.error_queue.is_empty() {
        if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
            update(model, Message::DismissError);
        }
        return true;
    }

    if model.help_visible {
        if key.code == KeyCode::Esc {
            update(model, Message::ToggleHelp);
        }
        return true;
    }

    if model.service_select.is_some() {
        let msg = match key.code {
            KeyCode::Down => Some(screens::service_select::ServiceSelectMessage::MoveDown),
            KeyCode::Up => Some(screens::service_select::ServiceSelectMessage::MoveUp),
            KeyCode::Enter => Some(screens::service_select::ServiceSelectMessage::Select),
            KeyCode::Esc => Some(screens::service_select::ServiceSelectMessage::Cancel),
            _ => None,
        };
        if let Some(msg) = msg {
            update(model, Message::ServiceSelect(msg));
            return true;
        }
    }
    if model
        .docker_progress
        .as_ref()
        .is_some_and(|progress| progress.visible)
        && key.code == KeyCode::Esc
    {
        update(
            model,
            Message::DockerProgress(screens::docker_progress::DockerProgressMessage::Hide),
        );
        return true;
    }
    if model.confirm.visible {
        let msg = match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                Some(screens::confirm::ConfirmMessage::Toggle)
            }
            KeyCode::Enter => Some(screens::confirm::ConfirmMessage::Accept),
            KeyCode::Esc => Some(screens::confirm::ConfirmMessage::Cancel),
            _ => None,
        };
        if let Some(msg) = msg {
            update(model, Message::Confirm(msg));
            return true;
        }
    }

    false
}

/// Route a key event to the branch detail pane (sections, Enter launches agent).
fn route_key_to_branch_detail(model: &mut Model, key: crossterm::event::KeyEvent) {
    use screens::branches::BranchesMessage;

    let msg = match key.code {
        KeyCode::Left => Some(BranchesMessage::PrevDetailSection),
        KeyCode::Right => Some(BranchesMessage::NextDetailSection),
        KeyCode::Enter => Some(BranchesMessage::LaunchAgent),
        KeyCode::Up if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerContainerUp)
        }
        KeyCode::Down if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerContainerDown)
        }
        KeyCode::Char('s') if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerContainerStart)
        }
        KeyCode::Char('t') if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerContainerStop)
        }
        KeyCode::Char('r') if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerContainerRestart)
        }
        _ => None,
    };
    if let Some(m) = msg {
        update(model, Message::Branches(m));
    } else if key.code == KeyCode::Esc {
        dismiss_warn_notification(model);
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
    use screens::versions::VersionsMessage;

    // Left/Right switches tabs when not in text input mode
    // (Ctrl+Left/Right is reserved for sub-tab switching within individual tabs)
    if !is_in_text_input_mode(model) && !key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Right => {
                model.management_tab = model.management_tab.next();
                return;
            }
            KeyCode::Left => {
                model.management_tab = model.management_tab.prev();
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
                    KeyCode::Esc => Some(BranchesMessage::SearchClear),
                    KeyCode::Backspace => Some(BranchesMessage::SearchBackspace),
                    _ => search_input_char(&key).map(BranchesMessage::SearchInput),
                };
                if let Some(m) = msg {
                    screens::branches::update(&mut model.branches, m);
                    return;
                }
            }

            let is_cursor_move = matches!(key.code, KeyCode::Down | KeyCode::Up);
            let msg = match key.code {
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Some(BranchesMessage::OpenShell)
                }
                KeyCode::Down => Some(BranchesMessage::MoveDown),
                KeyCode::Up => Some(BranchesMessage::MoveUp),
                KeyCode::Char(' ') => {
                    model.active_focus = FocusPane::BranchDetail;
                    return;
                }
                KeyCode::Char('c')
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && model
                            .branches
                            .selected_branch()
                            .is_some_and(|branch| branch.worktree_path.is_some()) =>
                {
                    Some(BranchesMessage::DeleteWorktree)
                }
                KeyCode::Enter => Some(BranchesMessage::Select),
                KeyCode::Char('s') => Some(BranchesMessage::ToggleSort),
                KeyCode::Char('v') => Some(BranchesMessage::ToggleView),
                KeyCode::Char('/') => Some(BranchesMessage::SearchStart),
                KeyCode::Char('r') => {
                    load_initial_data(model);
                    return;
                }
                _ => None,
            };
            if let Some(m) = msg {
                screens::branches::update(&mut model.branches, m);
                if is_cursor_move {
                    let repo_path = model.repo_path.clone();
                    screens::branches::load_branch_detail(&mut model.branches, &repo_path);
                }
            } else if key.code == KeyCode::Esc {
                dismiss_warn_notification(model);
            }
        }
        ManagementTab::Issues => {
            if model.issues.search_active {
                let msg = match key.code {
                    KeyCode::Esc => Some(IssuesMessage::SearchClear),
                    KeyCode::Backspace => Some(IssuesMessage::SearchBackspace),
                    _ => search_input_char(&key).map(IssuesMessage::SearchInput),
                };
                if let Some(m) = msg {
                    screens::issues::update(&mut model.issues, m);
                    return;
                }
            }

            let msg = match key.code {
                KeyCode::Down => Some(IssuesMessage::MoveDown),
                KeyCode::Up => Some(IssuesMessage::MoveUp),
                KeyCode::Enter => Some(IssuesMessage::ToggleDetail),
                KeyCode::Char('/') => Some(IssuesMessage::SearchStart),
                KeyCode::Char('r') => Some(IssuesMessage::Refresh),
                _ => None,
            };
            if let Some(m) = msg {
                screens::issues::update(&mut model.issues, m);
            } else if key.code == KeyCode::Esc {
                dismiss_warn_notification(model);
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
                } else if key.code == KeyCode::Esc {
                    dismiss_warn_notification(model);
                }
            } else {
                let msg = match key.code {
                    KeyCode::Down => Some(SettingsMessage::MoveDown),
                    KeyCode::Up => Some(SettingsMessage::MoveUp),
                    KeyCode::Enter => Some(SettingsMessage::StartEdit),
                    KeyCode::Char(' ') => Some(SettingsMessage::ToggleBool),
                    KeyCode::Char('S') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        Some(SettingsMessage::Save)
                    }
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(SettingsMessage::PrevCategory)
                    }
                    KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(SettingsMessage::NextCategory)
                    }
                    _ => None,
                };
                if let Some(m) = msg {
                    screens::settings::update(&mut model.settings, m);
                } else if key.code == KeyCode::Esc {
                    dismiss_warn_notification(model);
                }
            }
        }
        ManagementTab::Logs => {
            let msg = match key.code {
                KeyCode::Down => Some(LogsMessage::MoveDown),
                KeyCode::Up => Some(LogsMessage::MoveUp),
                KeyCode::Enter => Some(LogsMessage::ToggleDetail),
                KeyCode::Char('f') => Some(LogsMessage::SetFilter(next_logs_filter_level(
                    model.logs.filter_level,
                ))),
                KeyCode::Char('d') => Some(LogsMessage::SetFilter(toggle_logs_debug_filter(
                    model.logs.filter_level,
                ))),
                KeyCode::Char('r') => Some(LogsMessage::Refresh),
                KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => Some(
                    LogsMessage::SetFilter(next_logs_filter_level(model.logs.filter_level)),
                ),
                KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => Some(
                    LogsMessage::SetFilter(prev_logs_filter_level(model.logs.filter_level)),
                ),
                _ => None,
            };
            if let Some(m) = msg {
                screens::logs::update(&mut model.logs, m);
            } else if key.code == KeyCode::Esc {
                dismiss_warn_notification(model);
            }
        }
        ManagementTab::Versions => {
            let msg = match key.code {
                KeyCode::Down => Some(VersionsMessage::MoveDown),
                KeyCode::Up => Some(VersionsMessage::MoveUp),
                KeyCode::Char('r') => {
                    load_initial_data(model);
                    return;
                }
                _ => None,
            };
            if let Some(m) = msg {
                screens::versions::update(&mut model.versions, m);
            } else if key.code == KeyCode::Esc {
                dismiss_warn_notification(model);
            }
        }
        ManagementTab::GitView => {
            let msg = match key.code {
                KeyCode::Down => Some(GitViewMessage::MoveDown),
                KeyCode::Up => Some(GitViewMessage::MoveUp),
                KeyCode::Enter => Some(GitViewMessage::ToggleExpand),
                KeyCode::Char('r') => {
                    load_initial_data(model);
                    return;
                }
                _ => None,
            };
            if let Some(m) = msg {
                screens::git_view::update(&mut model.git_view, m);
            } else if key.code == KeyCode::Esc {
                dismiss_warn_notification(model);
            }
        }
        ManagementTab::PrDashboard => {
            let msg = match key.code {
                KeyCode::Down => Some(PrDashboardMessage::MoveDown),
                KeyCode::Up => Some(PrDashboardMessage::MoveUp),
                KeyCode::Enter => Some(PrDashboardMessage::ToggleDetail),
                KeyCode::Char('r') => Some(PrDashboardMessage::Refresh),
                _ => None,
            };
            if let Some(m) = msg {
                screens::pr_dashboard::update(&mut model.pr_dashboard, m);
            } else if key.code == KeyCode::Esc {
                dismiss_warn_notification(model);
            }
        }
        ManagementTab::Profiles => {
            let msg = match key.code {
                KeyCode::Down => Some(ProfilesMessage::MoveDown),
                KeyCode::Up => Some(ProfilesMessage::MoveUp),
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

/// Check and consume pending branch actions (Wizard launch, shell open).
fn check_branch_pending_actions(model: &mut Model) {
    if model.branches.pending_launch_agent {
        model.branches.pending_launch_agent = false;
        if let Some(branch) = model.branches.selected_branch() {
            let branch_name = branch.name.clone();
            open_wizard(model, None);
            if let Some(ref mut wizard) = model.wizard {
                wizard.branch_name = branch_name;
            }
        }
    }
    if model.branches.pending_open_shell {
        model.branches.pending_open_shell = false;
        if let Some(branch) = model.branches.selected_branch() {
            if branch.worktree_path.is_some() {
                let idx = model.sessions.len();
                let session = crate::model::SessionTab {
                    id: format!("shell-{idx}"),
                    name: format!("Shell: {}", branch.name),
                    tab_type: crate::model::SessionTabType::Shell,
                    vt: crate::model::VtState::new(24, 80),
                };
                model.sessions.push(session);
                model.active_session = idx;
                model.active_focus = FocusPane::Terminal;
            }
        }
    }
    if model.branches.pending_delete_worktree && !model.confirm.visible {
        if let Some(branch) = model.branches.selected_branch() {
            model.confirm = screens::confirm::ConfirmState::with_message(format!(
                "Delete worktree for {}?",
                branch.name
            ));
        } else {
            model.branches.pending_delete_worktree = false;
        }
    }
}

/// Count active sessions whose name matches the selected branch.
fn count_sessions_for_branch(model: &Model) -> usize {
    let Some(branch) = model.branches.selected_branch() else {
        return 0;
    };
    let branch_name = &branch.name;
    model
        .sessions
        .iter()
        .filter(|s| s.name.contains(branch_name))
        .count()
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

fn dismiss_warn_notification(model: &mut Model) {
    if matches!(
        model
            .current_notification
            .as_ref()
            .map(|notification| notification.severity),
        Some(Severity::Warn)
    ) {
        update(model, Message::DismissNotification);
    }
}

fn handle_pending_branch_docker_action(
    model: &mut Model,
    action: screens::branches::PendingDockerAction,
) {
    if model.docker_progress_events.is_some() {
        update(
            model,
            Message::ShowNotification(Notification::new(
                Severity::Warn,
                "docker",
                "Docker action already running",
            )),
        );
        return;
    }

    let container_label = model
        .branches
        .docker_containers
        .iter()
        .find(|container| container.id == action.container_id)
        .map(|container| container.name.clone())
        .unwrap_or_else(|| action.container_id.clone());

    emit_branch_docker_progress(
        model,
        screens::docker_progress::DockerStage::StartingContainer,
        format!(
            "{} container {container_label}",
            start_message_for_action(action.action)
        ),
    );

    let events = Arc::new(Mutex::new(VecDeque::new()));
    model.docker_progress_events = Some(events.clone());
    spawn_docker_progress_worker(events, action, container_label);
}

fn emit_branch_docker_progress(
    model: &mut Model,
    stage: screens::docker_progress::DockerStage,
    message: String,
) {
    update(
        model,
        Message::DockerProgress(screens::docker_progress::DockerProgressMessage::SetStage {
            stage,
            message,
        }),
    );
}

fn spawn_docker_progress_worker(
    events: DockerProgressQueue,
    action: screens::branches::PendingDockerAction,
    container_label: String,
) {
    use screens::branches::DockerLifecycleAction;

    thread::spawn(move || {
        let outcome = match action.action {
            DockerLifecycleAction::Start => {
                gwt_docker::start(&action.container_id).map(|()| DockerProgressResult::Completed {
                    message: format!("Started container {container_label}"),
                })
            }
            DockerLifecycleAction::Stop => {
                gwt_docker::stop(&action.container_id).map(|()| DockerProgressResult::Completed {
                    message: format!("Stopped container {container_label}"),
                })
            }
            DockerLifecycleAction::Restart => gwt_docker::restart(&action.container_id).map(|()| {
                DockerProgressResult::Completed {
                    message: format!("Restarted container {container_label}"),
                }
            }),
        };

        let event = match outcome {
            Ok(result) => result,
            Err(err) => DockerProgressResult::Failed {
                message: format!(
                    "Failed to {} container {container_label}",
                    verb_for_action(action.action)
                ),
                detail: err.to_string(),
            },
        };

        if let Ok(mut queue) = events.lock() {
            queue.push_back(event);
        }
    });
}

fn drain_docker_progress_events(model: &mut Model) {
    let Some(events) = model.docker_progress_events.as_ref().cloned() else {
        return;
    };

    let event = events.lock().ok().and_then(|mut queue| queue.pop_front());
    let Some(event) = event else {
        return;
    };

    model.docker_progress_events = None;
    match event {
        DockerProgressResult::Completed { message } => {
            emit_branch_docker_progress(
                model,
                screens::docker_progress::DockerStage::Ready,
                message.clone(),
            );
            let repo_path = model.repo_path.clone();
            screens::branches::load_branch_detail(&mut model.branches, &repo_path);
            update(
                model,
                Message::Notify(Notification::new(Severity::Info, "docker", message)),
            );
        }
        DockerProgressResult::Failed { message, detail } => {
            update(
                model,
                Message::DockerProgress(screens::docker_progress::DockerProgressMessage::SetError(
                    format!("{message}: {detail}"),
                )),
            );
            update(
                model,
                Message::Notify(
                    Notification::new(Severity::Error, "docker", message).with_detail(detail),
                ),
            );
        }
    }
}

fn start_message_for_action(action: screens::branches::DockerLifecycleAction) -> &'static str {
    use screens::branches::DockerLifecycleAction;

    match action {
        DockerLifecycleAction::Start => "Starting",
        DockerLifecycleAction::Stop => "Stopping",
        DockerLifecycleAction::Restart => "Restarting",
    }
}

fn verb_for_action(action: screens::branches::DockerLifecycleAction) -> &'static str {
    use screens::branches::DockerLifecycleAction;

    match action {
        DockerLifecycleAction::Start => "start",
        DockerLifecycleAction::Stop => "stop",
        DockerLifecycleAction::Restart => "restart",
    }
}

fn next_logs_filter_level(level: screens::logs::FilterLevel) -> screens::logs::FilterLevel {
    level.next()
}

fn prev_logs_filter_level(level: screens::logs::FilterLevel) -> screens::logs::FilterLevel {
    level.prev()
}

fn toggle_logs_debug_filter(level: screens::logs::FilterLevel) -> screens::logs::FilterLevel {
    use screens::logs::FilterLevel;
    if level == FilterLevel::DebugUp {
        FilterLevel::All
    } else {
        FilterLevel::DebugUp
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

/// Build a LaunchConfig from the wizard's accumulated selections.
fn build_launch_config_from_wizard(wizard: &screens::wizard::WizardState) -> LaunchConfig {
    let agent_id = match wizard.agent_id.as_str() {
        "claude" => AgentId::ClaudeCode,
        "codex" => AgentId::Codex,
        "gemini" => AgentId::Gemini,
        "opencode" => AgentId::OpenCode,
        "gh" => AgentId::Copilot,
        other => AgentId::Custom(other.to_string()),
    };

    let mut builder = AgentLaunchBuilder::new(agent_id);

    if !wizard.branch_name.is_empty() {
        builder = builder.branch(&wizard.branch_name);
    }

    if is_explicit_model_selection(&wizard.model) {
        builder = builder.model(&wizard.model);
    }

    if !wizard.version.is_empty() {
        builder = builder.version(&wizard.version);
    }

    if !wizard.reasoning.is_empty() && wizard.reasoning != "medium" {
        builder = builder.reasoning_level(&wizard.reasoning);
    }

    if wizard.skip_perms {
        builder = builder.permission_mode(gwt_agent::launch::PermissionMode::Auto);
    }

    builder.build()
}

fn is_explicit_model_selection(model: &str) -> bool {
    !model.is_empty() && !model.starts_with("Default")
}

fn materialize_pending_launch(model: &mut Model) {
    if let Err(err) = materialize_pending_launch_with(model, &gwt_sessions_dir()) {
        apply_notification(
            model,
            Notification::new(Severity::Warn, "session", "Launch metadata was not persisted")
                .with_detail(err),
        );
    }
}

fn materialize_pending_launch_with(model: &mut Model, sessions_dir: &std::path::Path) -> Result<(), String> {
    let Some(config) = model.pending_launch_config.take() else {
        return Ok(());
    };

    let mut session = AgentSession::new(
        model.repo_path.clone(),
        config.branch.clone().unwrap_or_default(),
        config.agent_id.clone(),
    );
    session.model = config.model.clone().filter(|model| is_explicit_model_selection(model));
    session.tool_version = config.tool_version.clone();
    session.display_name = config.display_name.clone();
    session.save(sessions_dir).map_err(|err| err.to_string())?;

    let tab = crate::model::SessionTab {
        id: session.id.clone(),
        name: config.display_name.clone(),
        tab_type: SessionTabType::Agent {
            agent_id: config.agent_id.command().to_string(),
            color: tui_agent_color(config.color),
        },
        vt: crate::model::VtState::new(24, 80),
    };
    model.sessions.push(tab);
    model.active_session = model.sessions.len().saturating_sub(1);
    model.active_layer = ActiveLayer::Main;

    apply_notification(
        model,
        Notification::new(
            Severity::Info,
            "session",
            format!("Created session for {}", config.display_name),
        ),
    );

    Ok(())
}

fn open_wizard(model: &mut Model, spec_context: Option<screens::wizard::SpecContext>) {
    let cache_path = wizard_version_cache_path();
    let cache = VersionCache::load(&cache_path);
    let detected_agents = AgentDetector::detect_all();
    let (wizard, refresh_targets) = prepare_wizard_startup(spec_context, detected_agents, &cache);

    model.wizard = Some(wizard);
    schedule_wizard_version_cache_refresh(cache_path, refresh_targets);
}

fn open_session_conversion(model: &mut Model) {
    open_session_conversion_with(model, AgentDetector::detect_all());
}

fn open_session_conversion_with(model: &mut Model, detected_agents: Vec<DetectedAgent>) {
    let Some(session) = model.active_session_tab() else {
        return;
    };
    let SessionTabType::Agent { agent_id, .. } = &session.tab_type else {
        return;
    };

    let (services, values): (Vec<_>, Vec<_>) = detected_agents
        .into_iter()
        .filter(|detected| detected.agent_id.command() != agent_id)
        .map(|detected| {
            (
                detected.agent_id.display_name().to_string(),
                detected.agent_id.command().to_string(),
            )
        })
        .unzip();

    if services.is_empty() {
        return;
    }

    model.pending_session_conversion = None;
    model.service_select = Some(screens::service_select::ServiceSelectState::with_options(
        "Select Agent",
        services,
        values,
    ));
}

fn handle_confirm_message(model: &mut Model, msg: screens::confirm::ConfirmMessage) {
    handle_confirm_message_with(model, msg, AgentDetector::detect_all());
}

fn handle_confirm_message_with(
    model: &mut Model,
    msg: screens::confirm::ConfirmMessage,
    detected_agents: Vec<DetectedAgent>,
) {
    let should_apply_session_conversion = matches!(msg, screens::confirm::ConfirmMessage::Accept)
        && model.confirm.accepted()
        && model.pending_session_conversion.is_some();
    let should_delete_worktree = matches!(msg, screens::confirm::ConfirmMessage::Accept)
        && model.confirm.accepted()
        && model.branches.pending_delete_worktree;
    let dismisses_session_conversion = matches!(msg, screens::confirm::ConfirmMessage::Cancel)
        || (matches!(msg, screens::confirm::ConfirmMessage::Accept) && !model.confirm.accepted());
    let dismisses_worktree_delete = matches!(msg, screens::confirm::ConfirmMessage::Cancel)
        || (matches!(msg, screens::confirm::ConfirmMessage::Accept) && !model.confirm.accepted());
    screens::confirm::update(&mut model.confirm, msg);
    if should_apply_session_conversion {
        if let Some(pending) = model.pending_session_conversion.take() {
            let target_display_name = pending.target_display_name.clone();
            match apply_pending_session_conversion_with(model, pending, detected_agents) {
                Ok(()) => apply_notification(
                    model,
                    Notification::new(
                        Severity::Info,
                        "session",
                        format!("Converted session to {target_display_name}"),
                    ),
                ),
                Err(err) => {
                    apply_notification(model, Notification::new(Severity::Error, "session", err))
                }
            }
        }
    } else if should_delete_worktree {
        let worktree_target = model.branches.selected_branch().and_then(|branch| {
            branch
                .worktree_path
                .as_ref()
                .map(|path| (branch.name.clone(), path.clone()))
        });
        model.branches.pending_delete_worktree = false;
        if let Some((branch_name, path)) = worktree_target {
            let manager = gwt_git::worktree::WorktreeManager::new(&model.repo_path);
            match manager.remove(&path) {
                Ok(()) => {
                    load_initial_data(model);
                    apply_notification(
                        model,
                        Notification::new(
                            Severity::Info,
                            "worktree",
                            format!("Removed worktree for {branch_name}"),
                        ),
                    );
                }
                Err(err) => apply_notification(
                    model,
                    Notification::new(
                        Severity::Error,
                        "worktree",
                        format!("Failed to remove worktree for {branch_name}"),
                    )
                    .with_detail(err.to_string()),
                ),
            }
        }
    } else if dismisses_session_conversion {
        model.pending_session_conversion = None;
    }
    if dismisses_worktree_delete {
        model.branches.pending_delete_worktree = false;
    }
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
    let starts_new_branch = spec_context.is_some();

    let mut wizard = screens::wizard::WizardState {
        step: if starts_new_branch {
            screens::wizard::WizardStep::BranchTypeSelect
        } else {
            screens::wizard::WizardStep::QuickStart
        },
        is_new_branch: starts_new_branch,
        gh_cli_available: gwt_core::process::command_exists("gh"),
        ai_enabled: true,
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
/// All builtin agent IDs in display order.
const BUILTIN_AGENTS: [AgentId; 4] = [
    AgentId::ClaudeCode,
    AgentId::Codex,
    AgentId::Gemini,
    AgentId::Copilot,
];

fn build_wizard_agent_options(
    detected_agents: Vec<DetectedAgent>,
    cache: &VersionCache,
) -> (Vec<screens::wizard::AgentOption>, Vec<AgentId>) {
    let mut refresh_targets = Vec::new();
    let mut options = Vec::new();

    // Always list all builtin agents (installed or not), like old TUI
    for builtin_id in &BUILTIN_AGENTS {
        let detected = detected_agents
            .iter()
            .find(|d| &d.agent_id == builtin_id);
        let available = detected.is_some();
        let installed_version = detected.and_then(|d| d.version.clone());

        let cached_versions = cached_agent_versions(cache, builtin_id);
        let cache_refreshable = builtin_id.package_name().is_some();
        let cache_outdated = cache_refreshable && cache.needs_refresh(builtin_id);
        if cache_outdated {
            refresh_targets.push(builtin_id.clone());
        }

        options.push(screens::wizard::AgentOption {
            id: builtin_id.command().to_string(),
            name: builtin_id.display_name().to_string(),
            available,
            installed_version,
            versions: cached_versions,
            cache_outdated,
        });
    }

    // TODO: Add custom agents from Settings here

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

fn apply_pending_session_conversion_with(
    model: &mut Model,
    pending: PendingSessionConversion,
    detected_agents: Vec<DetectedAgent>,
) -> Result<(), String> {
    let original_tab_type = model
        .sessions
        .get(pending.session_index)
        .map(|session| session.tab_type.clone())
        .ok_or_else(|| format!("Session index {} is out of bounds", pending.session_index))?;

    if !matches!(original_tab_type, SessionTabType::Agent { .. }) {
        return Err("Active session is not an agent session".to_string());
    }

    let detected = detected_agents
        .into_iter()
        .find(|candidate| candidate.agent_id.command() == pending.target_agent_id)
        .ok_or_else(|| {
            format!(
                "Target agent `{}` is not available",
                pending.target_agent_id
            )
        })?;

    let session = model
        .sessions
        .get_mut(pending.session_index)
        .ok_or_else(|| format!("Session index {} is out of bounds", pending.session_index))?;
    session.name = pending.target_display_name;
    session.tab_type = SessionTabType::Agent {
        agent_id: detected.agent_id.command().to_string(),
        color: tui_agent_color(detected.agent_id.default_color()),
    };

    Ok(())
}

fn tui_agent_color(color: gwt_agent::AgentColor) -> crate::model::AgentColor {
    match color {
        gwt_agent::AgentColor::Green => crate::model::AgentColor::Green,
        gwt_agent::AgentColor::Blue => crate::model::AgentColor::Blue,
        gwt_agent::AgentColor::Cyan => crate::model::AgentColor::Cyan,
        gwt_agent::AgentColor::Yellow => crate::model::AgentColor::Yellow,
        gwt_agent::AgentColor::Magenta => crate::model::AgentColor::Magenta,
        gwt_agent::AgentColor::Gray => crate::model::AgentColor::Gray,
    }
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
        ManagementTab::Settings => model.settings.editing,
        _ => false,
    }
}

fn handle_mouse_input(model: &mut Model, mouse: MouseEvent) {
    if let Err(err) = handle_mouse_input_with(model, mouse, open_url) {
        model.error_queue.push_back(
            Notification::new(Severity::Error, "terminal", "Failed to open URL").with_detail(err),
        );
    }
}

fn handle_mouse_input_with<F>(
    model: &mut Model,
    mouse: MouseEvent,
    mut opener: F,
) -> Result<bool, String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        return Ok(false);
    }
    if !mouse.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(false);
    }

    let Some(url) = url_at_mouse_position(model, mouse) else {
        return Ok(false);
    };

    opener(&url)?;
    Ok(true)
}

fn url_at_mouse_position(model: &Model, mouse: MouseEvent) -> Option<String> {
    let area = active_session_content_area(model)?;
    if mouse.column < area.x
        || mouse.column >= area.right()
        || mouse.row < area.y
        || mouse.row >= area.bottom()
    {
        return None;
    }

    let session = model.active_session_tab()?;
    crate::renderer::collect_url_regions(
        session.vt.screen(),
        Rect::new(0, 0, area.width, area.height),
    )
    .into_iter()
    .find(|region| {
        let row = area.y + region.row;
        let start_col = area.x + region.start_col;
        let end_col = area.x + region.end_col;
        mouse.row == row && mouse.column >= start_col && mouse.column <= end_col
    })
    .map(|region| region.url)
}

fn active_session_content_area(model: &Model) -> Option<Rect> {
    if model.active_layer == ActiveLayer::Initialization {
        return None;
    }

    let (width, height) = model.terminal_size;
    if width == 0 || height == 0 {
        return None;
    }

    let size = Rect::new(0, 0, width, height);
    let main_area = Rect {
        height: size.height.saturating_sub(1),
        ..size
    };
    let session_area = if model.active_layer == ActiveLayer::Management {
        let lr = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_area);
        lr[1]
    } else {
        main_area
    };

    session_content_area(model, session_area)
}

fn session_content_area(model: &Model, session_area: Rect) -> Option<Rect> {
    match model.session_layout {
        SessionLayout::Tab => {
            model.active_session_tab()?;
            Some(
                pane_block(
                    build_session_title(model),
                    model.active_focus == FocusPane::Terminal,
                )
                .inner(session_area),
            )
        }
        SessionLayout::Grid => active_grid_session_content_area(model, session_area),
    }
}

fn active_grid_session_content_area(model: &Model, area: Rect) -> Option<Rect> {
    let count = model.sessions.len();
    if count == 0 || model.active_session >= count {
        return None;
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

    let target_row = model.active_session / cols;
    let start = target_row * cols;
    let end = (start + cols).min(count);
    let n = end - start;
    let col_constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();
    let col_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(col_constraints)
        .split(row_chunks[target_row]);

    let target_col = model.active_session - start;
    let session = model.sessions.get(model.active_session)?;
    Some(
        Block::default()
            .borders(Borders::ALL)
            .title(session.name.as_str())
            .inner(col_chunks[target_col]),
    )
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };

    command
        .status()
        .map_err(|err| format!("failed to spawn URL opener: {err}"))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(format!("URL opener exited with status {status}"))
            }
        })
}

fn render_session_surface(session: &crate::model::SessionTab, frame: &mut Frame, area: Rect) {
    if session.vt.screen().contents().trim().is_empty() {
        let placeholder = Paragraph::new(format!(
            "Session: {} ({}x{})",
            session.name,
            session.vt.cols(),
            session.vt.rows()
        ))
        .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(placeholder, area);
    } else {
        frame.render_widget(
            crate::widgets::terminal_view::TerminalView::new(session.vt.screen()),
            area,
        );
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

    // Reserve 1 line at bottom for keybind hints
    let main_area = Rect {
        height: size.height.saturating_sub(1),
        ..size
    };
    let hint_area = Rect {
        y: size.height.saturating_sub(1),
        height: 1,
        ..size
    };

    if model.active_layer == ActiveLayer::Management {
        // 50/50 split: left = management panes, right = session pane
        let lr = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_area);

        render_management_panes(model, frame, lr[0]);
        render_session_pane(model, frame, lr[1]);
    } else {
        render_session_pane(model, frame, main_area);
    }

    render_keybind_hints(model, frame, hint_area);

    // Overlays on top
    render_overlays(model, frame, size);
}

/// Build a bordered block with focus-aware border color (Green when focused, White otherwise).
fn pane_block(title: Line<'static>, is_focused: bool) -> Block<'static> {
    let border_color = if is_focused {
        Color::Green
    } else {
        Color::White
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title)
}

/// Build the management tab title line for embedding in a pane border.
fn management_tab_title(model: &Model) -> Line<'static> {
    let labels: Vec<&str> = ManagementTab::ALL.iter().map(|t| t.label()).collect();
    let active_idx = ManagementTab::ALL
        .iter()
        .position(|t| *t == model.management_tab)
        .unwrap_or(0);
    screens::build_tab_title(&labels, active_idx)
}

/// Render a 1-line header at the top of the management panel.
fn render_management_header(model: &Model, frame: &mut Frame, area: Rect) {
    let version = env!("CARGO_PKG_VERSION");
    let repo = model.repo_path.display();
    let branch_count = model.branches.branches.len();
    let wt_count = model
        .branches
        .branches
        .iter()
        .filter(|b| b.worktree_path.is_some())
        .count();
    let text = format!(
        " GWT v{} | {} | {} branches ({} worktrees)",
        version, repo, branch_count, wt_count
    );
    let p = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, area);
}

/// Render the management panes (left side — 2 stacked for Branches, 1 for others).
fn render_management_panes(model: &Model, frame: &mut Frame, area: Rect) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    render_management_header(model, frame, outer[0]);
    let content_area = outer[1];

    if model.management_tab == ManagementTab::Branches {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(content_area);

        // Top pane: management tab names in title, branch list content
        let list_focused = model.active_focus == FocusPane::TabContent;
        let list_block = pane_block(management_tab_title(model), list_focused);
        let list_inner = list_block.inner(chunks[0]);
        frame.render_widget(list_block, chunks[0]);
        screens::branches::render_list(&model.branches, frame, list_inner);

        // Bottom pane: detail section names in title, detail content
        let detail_focused = model.active_focus == FocusPane::BranchDetail;
        let detail_labels: Vec<&str> = screens::branches::detail_section_labels().to_vec();
        let detail_title = screens::build_tab_title(&detail_labels, model.branches.detail_section);
        let detail_block = pane_block(detail_title, detail_focused);
        let detail_inner = detail_block.inner(chunks[1]);
        frame.render_widget(detail_block, chunks[1]);
        let branch_session_count = count_sessions_for_branch(model);
        screens::branches::render_detail_content(
            &model.branches,
            frame,
            detail_inner,
            branch_session_count,
        );
    } else {
        // Single pane for all other tabs
        let focused = model.active_focus == FocusPane::TabContent;
        let block = pane_block(management_tab_title(model), focused);
        let inner = block.inner(content_area);
        frame.render_widget(block, content_area);
        render_management_tab_content(model, frame, inner);
    }
}

/// Render the content of the active management tab (non-Branches).
fn render_management_tab_content(model: &Model, frame: &mut Frame, area: Rect) {
    match model.management_tab {
        ManagementTab::Branches => {
            // Handled by render_management_panes directly
        }
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

/// Render the session pane (right side, or full screen).
fn render_session_pane(model: &Model, frame: &mut Frame, area: Rect) {
    let terminal_focused = model.active_focus == FocusPane::Terminal;
    match model.session_layout {
        SessionLayout::Tab => {
            if let Some(session) = model.active_session_tab() {
                let title = build_session_title(model);
                let block = pane_block(title, terminal_focused);
                let inner = block.inner(area);
                frame.render_widget(block, area);
                render_session_surface(session, frame, inner);
            }
        }
        SessionLayout::Grid => {
            render_grid_sessions(model, frame, area);
        }
    }
}

/// Build session tab title line (same pattern as management tabs in Block title).
fn build_session_title(model: &Model) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, s) in model.sessions.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("│"));
        }
        let label = format!(" {} {} ", s.tab_type.icon(), s.name);
        if i == model.active_session {
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(label, Style::default().fg(Color::Gray)));
        }
    }
    Line::from(spans)
}

/// Render context-sensitive keybind hints at the bottom of the screen.
///
/// When a notification is active, it is shown alongside the hints.
fn render_keybind_hints(model: &Model, frame: &mut Frame, area: Rect) {
    // Show notification if active, otherwise show keybind hints
    if let Some(ref notification) = model.current_notification {
        let severity_style = match notification.severity {
            gwt_notification::Severity::Info => Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            gwt_notification::Severity::Warn => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            _ => Style::default().fg(Color::DarkGray),
        };
        let summary = format!(
            " {} {}: {} ",
            notification.severity, notification.source, notification.message
        );
        let line = Paragraph::new(Span::styled(summary, severity_style));
        frame.render_widget(line, area);
    } else {
        let hints = match model.active_focus {
            FocusPane::TabContent if model.management_tab == ManagementTab::Branches => {
                "\u{2191}\u{2193}:move  \u{2190}\u{2192}:tab  Enter:wizard  Shift+Enter:shell  Space:detail  Ctrl+C:delete  Tab:focus"
            }
            FocusPane::TabContent => {
                "\u{2191}\u{2193}:select  \u{2190}\u{2192}:tab  Ctrl+\u{2190}\u{2192}:sub-tab  Enter:action  Tab:focus  ?:help"
            }
            FocusPane::BranchDetail => {
                "\u{2190}\u{2192}:section  Enter:action  Tab:focus  Esc:back"
            }
            FocusPane::Terminal => "Ctrl+G,g:management  Tab:focus  Ctrl+C\u{00d7}2:quit",
        };
        let line = Paragraph::new(hints).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(line, area);
    }
}

/// Render all overlay widgets on top of the main layout.
fn render_overlays(model: &Model, frame: &mut Frame, size: Rect) {
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

    if model.help_visible {
        let bindings = crate::input::keybind::KeybindRegistry::new();
        screens::help::render(bindings.all_bindings(), frame, size);
    }

    // Error overlay on top of everything
    if !model.error_queue.is_empty() {
        screens::error::render(&model.error_queue, frame, size);
    }
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
    use crossterm::event::{
        KeyEvent, KeyEventKind, KeyEventState, MouseButton, MouseEvent, MouseEventKind,
    };
    use gwt_agent::{version_cache::VersionEntry, AgentId, DetectedAgent, VersionCache};
    use gwt_notification::{Notification, Severity};
    use ratatui::backend::TestBackend;
    use ratatui::{buffer::Buffer, Terminal};
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

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

    fn buffer_text(buf: &Buffer) -> String {
        let mut text = String::new();
        for y in 0..buf.area.height {
            let line = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>();
            text.push_str(line.trim_end());
            text.push('\n');
        }
        text
    }

    fn render_model_text(model: &Model, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| view(model, frame))
            .expect("draw model");
        buffer_text(terminal.backend().buffer())
    }

    fn detected_agent(agent_id: AgentId, version: Option<&str>) -> DetectedAgent {
        DetectedAgent {
            agent_id,
            version: version.map(|value| value.to_string()),
            path: PathBuf::from("/tmp/fake-agent"),
        }
    }

    fn agent_session_tab(
        name: &str,
        agent_id: &str,
        color: crate::model::AgentColor,
    ) -> crate::model::SessionTab {
        crate::model::SessionTab {
            id: "agent-0".to_string(),
            name: name.to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: agent_id.to_string(),
                color,
            },
            vt: crate::model::VtState::new(30, 100),
        }
    }

    fn version_entry(versions: &[&str], age_seconds: i64) -> VersionEntry {
        VersionEntry {
            versions: versions.iter().map(|value| value.to_string()).collect(),
            updated_at: Utc::now() - Duration::seconds(age_seconds),
        }
    }

    fn docker_container(
        id: &str,
        name: &str,
        status: gwt_docker::ContainerStatus,
    ) -> gwt_docker::ContainerInfo {
        gwt_docker::ContainerInfo {
            id: id.to_string(),
            name: name.to_string(),
            status,
            image: "nginx:latest".to_string(),
            ports: "0.0.0.0:8080->80/tcp".to_string(),
        }
    }

    fn write_fake_docker(script_body: &str) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let script_path = dir.path().join("docker");
        let mut file = fs::File::create(&script_path).expect("create fake docker");
        file.write_all(script_body.as_bytes())
            .expect("write fake docker");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata().expect("stat fake docker").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).expect("chmod fake docker");
        }

        (dir, script_path)
    }

    fn with_fake_docker<R>(script_body: &str, f: impl FnOnce() -> R) -> R {
        let _guard = crate::DOCKER_ENV_TEST_LOCK
            .lock()
            .expect("lock docker tests");
        let (_dir, script_path) = write_fake_docker(script_body);
        let previous = std::env::var_os("GWT_DOCKER_BIN");
        std::env::set_var("GWT_DOCKER_BIN", &script_path);

        let result = f();

        match previous {
            Some(value) => std::env::set_var("GWT_DOCKER_BIN", value),
            None => std::env::remove_var("GWT_DOCKER_BIN"),
        }

        result
    }

    fn drive_docker_worker_until(model: &mut Model, done: impl Fn(&Model) -> bool, context: &str) {
        for _ in 0..40 {
            update(model, Message::Tick);
            if done(model) {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }

        panic!("timed out waiting for docker worker: {context}");
    }

    #[test]
    fn pty_output_renders_into_session_surface() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;

        update(
            &mut model,
            Message::PtyOutput("shell-0".to_string(), b"https://example.com".to_vec()),
        );

        let rendered = render_model_text(&model, 80, 24);
        assert!(rendered.contains("https://example.com"));
    }

    #[test]
    fn ctrl_click_on_url_invokes_opener_with_full_url() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.terminal_size = (80, 24);
        let expected_url = "https://example.com/docs";
        update(
            &mut model,
            Message::PtyOutput("shell-0".to_string(), expected_url.as_bytes().to_vec()),
        );
        let area = active_session_content_area(&model).expect("active session area");
        let region = crate::renderer::collect_url_regions(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .screen(),
            Rect::new(0, 0, area.width, area.height),
        )
        .into_iter()
        .find(|region| region.url == expected_url)
        .expect("url region");

        let mut opened = None;
        let opened_result = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x + region.start_col,
                row: area.y + region.row,
                modifiers: KeyModifiers::CONTROL,
            },
            |url| {
                opened = Some(url.to_string());
                Ok(())
            },
        )
        .expect("mouse handler succeeds");

        assert!(opened_result);
        assert_eq!(opened.as_deref(), Some(expected_url));
    }

    #[test]
    fn click_without_ctrl_does_not_invoke_opener() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        let expected_url = "https://example.com";
        update(
            &mut model,
            Message::PtyOutput("shell-0".to_string(), expected_url.as_bytes().to_vec()),
        );
        let area = active_session_content_area(&model).expect("active session area");
        let region = crate::renderer::collect_url_regions(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .screen(),
            Rect::new(0, 0, area.width, area.height),
        )
        .into_iter()
        .find(|region| region.url == expected_url)
        .expect("url region");

        let mut opened = false;
        let opened_result = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x + region.start_col,
                row: area.y + region.row,
                modifiers: KeyModifiers::NONE,
            },
            |_| {
                opened = true;
                Ok(())
            },
        )
        .expect("mouse handler succeeds");

        assert!(!opened_result);
        assert!(!opened);
    }

    fn init_git_repo(path: &std::path::Path) {
        let path_str = path.to_string_lossy().to_string();
        let init = std::process::Command::new("git")
            .args(["init", &path_str])
            .output()
            .expect("init git repo");
        assert!(init.status.success(), "git init failed: {:?}", init);

        let email = std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .expect("set git email");
        assert!(email.status.success(), "git config user.email failed");

        let name = std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .expect("set git name");
        assert!(name.status.success(), "git config user.name failed");
    }

    fn git_commit_allow_empty(path: &std::path::Path, message: &str) {
        let output = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", message])
            .current_dir(path)
            .output()
            .expect("create git commit");
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
    fn load_initial_data_populates_git_view_from_repository_state() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");

        let tracked = dir.path().join("tracked.txt");
        fs::write(&tracked, "before\n").expect("write tracked file");
        let add = std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(dir.path())
            .output()
            .expect("git add tracked file");
        assert!(add.status.success(), "git add failed");
        git_commit_allow_empty(dir.path(), "add tracked file");

        fs::write(&tracked, "before\nafter\n").expect("modify tracked file");
        fs::write(dir.path().join("new.txt"), "new file\n").expect("write untracked file");

        let mut model = Model::new(dir.path().to_path_buf());
        load_initial_data(&mut model);

        assert!(
            model
                .git_view
                .files
                .iter()
                .any(|item| item.path == "tracked.txt"),
            "tracked modified file should appear in Git View"
        );
        assert!(
            model
                .git_view
                .files
                .iter()
                .any(|item| item.path == "new.txt"),
            "untracked file should appear in Git View"
        );
        assert!(
            model
                .git_view
                .commits
                .iter()
                .any(|commit| commit.subject == "add tracked file"),
            "recent commits should populate Git View"
        );
    }

    #[test]
    fn load_initial_data_handles_empty_repo_git_view_gracefully() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());

        let mut model = Model::new(dir.path().to_path_buf());
        load_initial_data(&mut model);

        assert!(
            model.git_view.files.is_empty(),
            "empty repo should not produce file entries"
        );
        assert!(
            model.git_view.commits.is_empty(),
            "empty repo should not produce commit entries"
        );
    }

    #[test]
    fn update_error_queue() {
        let mut model = test_model();
        update(&mut model, Message::PushError("e1".into()));
        update(
            &mut model,
            Message::PushErrorNotification(Notification::new(Severity::Error, "core", "e2")),
        );
        assert_eq!(model.error_queue.len(), 2);

        update(&mut model, Message::DismissError);
        assert_eq!(model.error_queue.len(), 1);
        assert_eq!(model.error_queue.front().unwrap().message, "e2");
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
        // All 4 builtins are always listed
        assert_eq!(wizard.detected_agents.len(), 4);
        // Claude Code: installed with cache
        assert_eq!(wizard.detected_agents[0].name, "Claude Code");
        assert!(wizard.detected_agents[0].available);
        assert_eq!(
            wizard.detected_agents[0].installed_version.as_deref(),
            Some("1.0.55")
        );
        assert_eq!(wizard.detected_agents[0].versions, vec!["1.0.54", "1.0.53"]);
        // Codex: installed, stale cache
        assert_eq!(wizard.detected_agents[1].name, "Codex");
        assert!(wizard.detected_agents[1].available);
        assert_eq!(
            wizard.detected_agents[1].installed_version.as_deref(),
            Some("0.5.1")
        );
        assert!(wizard.detected_agents[1].cache_outdated);
        // Gemini: installed, no cache
        assert_eq!(wizard.detected_agents[2].name, "Gemini CLI");
        assert!(wizard.detected_agents[2].available);
        assert_eq!(
            wizard.detected_agents[2].installed_version.as_deref(),
            Some("0.2.0")
        );
        assert!(wizard.detected_agents[2].cache_outdated);
        // Copilot: not installed (not in detected list)
        assert_eq!(wizard.detected_agents[3].name, "GitHub Copilot");
        assert!(!wizard.detected_agents[3].available);
        assert!(wizard.detected_agents[3].installed_version.is_none());
        assert_eq!(wizard.model, "Default (Opus 4.6)");
        assert_eq!(
            wizard
                .version_options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Installed (v1.0.55)", "latest", "1.0.54", "1.0.53"]
        );
        assert_eq!(refresh_targets, vec![AgentId::Codex, AgentId::Gemini]);
    }

    #[test]
    fn prepare_wizard_startup_starts_spec_prefill_at_branch_type_select() {
        let cache = VersionCache::new();

        let (wizard, _) = prepare_wizard_startup(
            Some(screens::wizard::SpecContext {
                spec_id: "SPEC-42".into(),
                title: "My Feature".into(),
            }),
            vec![],
            &cache,
        );

        assert_eq!(wizard.step, screens::wizard::WizardStep::BranchTypeSelect);
        assert_eq!(wizard.branch_name, "feature/spec-42");
    }

    #[test]
    fn prepare_wizard_startup_uses_detected_version_when_cache_is_missing() {
        let cache = VersionCache::new();
        let detected = vec![detected_agent(AgentId::ClaudeCode, Some("1.0.55"))];

        let (wizard, refresh_targets) = prepare_wizard_startup(None, detected, &cache);

        assert!(wizard.spec_context.is_none());
        assert!(wizard.branch_name.is_empty());
        // All 4 builtins listed; Claude installed, others not
        assert_eq!(wizard.detected_agents.len(), 4);
        assert!(wizard.detected_agents[0].available); // Claude installed
        assert_eq!(
            wizard.detected_agents[0].installed_version.as_deref(),
            Some("1.0.55")
        );
        assert!(wizard.detected_agents[0].versions.is_empty());
        assert_eq!(
            wizard
                .version_options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Installed (v1.0.55)", "latest"]
        );
        assert!(wizard.detected_agents[0].cache_outdated);
        assert!(!wizard.detected_agents[1].available); // Codex not installed
        assert!(!wizard.detected_agents[2].available); // Gemini not installed
        assert!(!wizard.detected_agents[3].available); // Copilot not installed
        // All npm agents need refresh (empty cache)
        assert!(refresh_targets.contains(&AgentId::ClaudeCode));
        assert!(refresh_targets.contains(&AgentId::Codex));
        assert!(refresh_targets.contains(&AgentId::Gemini));
    }

    #[test]
    fn build_launch_config_from_wizard_omits_default_model_selection() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "Default (Opus 4.6)".to_string(),
            branch_name: "feature/spec-42".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.agent_id, AgentId::ClaudeCode);
        assert_eq!(config.branch.as_deref(), Some("feature/spec-42"));
        assert!(config.model.is_none());
        assert!(!config.args.iter().any(|arg| arg.contains("--model")));
    }

    #[test]
    fn build_launch_config_from_wizard_keeps_selected_version() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "sonnet".to_string(),
            version: "latest".to_string(),
            branch_name: "feature/spec-42".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.agent_id, AgentId::ClaudeCode);
        assert_eq!(config.branch.as_deref(), Some("feature/spec-42"));
        assert_eq!(config.model.as_deref(), Some("sonnet"));
        assert_eq!(config.tool_version.as_deref(), Some("latest"));
    }

    #[test]
    fn materialize_pending_launch_with_creates_agent_session_and_persists_metadata() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut model = test_model();
        model.pending_launch_config = Some(
            AgentLaunchBuilder::new(AgentId::ClaudeCode)
                .branch("feature/spec-42")
                .model("sonnet")
                .version("latest")
                .build(),
        );

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        assert!(model.pending_launch_config.is_none());
        assert_eq!(model.sessions.len(), 2);
        assert_eq!(model.active_layer, ActiveLayer::Main);
        let session_tab = model.active_session_tab().expect("active launched session");
        assert_eq!(session_tab.name, "Claude Code");
        assert_eq!(
            session_tab.tab_type,
            SessionTabType::Agent {
                agent_id: "claude".to_string(),
                color: crate::model::AgentColor::Green,
            }
        );

        let mut entries = fs::read_dir(dir.path())
            .expect("read sessions dir")
            .map(|entry| entry.expect("dir entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        assert_eq!(entries.len(), 1);

        let persisted = AgentSession::load(&entries[0]).expect("load persisted session");
        assert_eq!(persisted.agent_id, AgentId::ClaudeCode);
        assert_eq!(persisted.branch, "feature/spec-42");
        assert_eq!(persisted.model.as_deref(), Some("sonnet"));
        assert_eq!(persisted.tool_version.as_deref(), Some("latest"));
        assert_eq!(persisted.display_name, "Claude Code");
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
        // Claude stale, Codex fresh, Gemini missing → Claude + Gemini need refresh
        assert!(targets.contains(&AgentId::ClaudeCode));
        assert!(!targets.contains(&AgentId::Codex));
        assert!(targets.contains(&AgentId::Gemini));
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
        // All npm agents (Claude, Codex, Gemini) need refresh from empty cache
        assert!(targets.contains(&AgentId::ClaudeCode));
        assert!(targets.contains(&AgentId::Codex));
        assert!(targets.contains(&AgentId::Gemini));
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
    fn open_session_conversion_with_opens_picker_for_alternative_agents() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);

        open_session_conversion_with(
            &mut model,
            vec![
                detected_agent(AgentId::ClaudeCode, Some("1.0.55")),
                detected_agent(AgentId::Codex, Some("0.5.1")),
                detected_agent(AgentId::Gemini, Some("0.2.0")),
            ],
        );

        let picker = model.service_select.as_ref().unwrap();
        assert!(picker.visible);
        assert_eq!(picker.title, "Select Agent");
        assert_eq!(
            picker.services,
            vec!["Codex".to_string(), "Gemini CLI".to_string()]
        );
        assert_eq!(
            picker.values,
            vec!["codex".to_string(), "gemini".to_string()]
        );
    }

    #[test]
    fn apply_pending_session_conversion_with_updates_active_session_and_preserves_repo_path() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);
        let original_repo_path = model.repo_path.clone();

        apply_pending_session_conversion_with(
            &mut model,
            PendingSessionConversion {
                session_index: 0,
                target_agent_id: "codex".to_string(),
                target_display_name: "Codex".to_string(),
            },
            vec![detected_agent(AgentId::Codex, Some("0.5.1"))],
        )
        .unwrap();

        let converted = &model.sessions[0];
        assert_eq!(converted.name, "Codex");
        assert_eq!(
            converted.tab_type,
            SessionTabType::Agent {
                agent_id: "codex".to_string(),
                color: crate::model::AgentColor::Blue,
            }
        );
        assert_eq!(converted.vt.rows(), 30);
        assert_eq!(converted.vt.cols(), 100);
        assert_eq!(model.repo_path, original_repo_path);
    }

    #[test]
    fn apply_pending_session_conversion_with_preserves_original_session_on_failure() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);
        let original_name = model.sessions[0].name.clone();
        let original_tab_type = model.sessions[0].tab_type.clone();

        let err = apply_pending_session_conversion_with(
            &mut model,
            PendingSessionConversion {
                session_index: 0,
                target_agent_id: "gemini".to_string(),
                target_display_name: "Gemini CLI".to_string(),
            },
            vec![detected_agent(AgentId::Codex, Some("0.5.1"))],
        )
        .unwrap_err();

        assert!(err.contains("gemini"));
        assert_eq!(model.sessions[0].name, original_name);
        assert_eq!(model.sessions[0].tab_type, original_tab_type);
    }

    #[test]
    fn update_open_session_conversion_for_agent_session_opens_picker() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);

        open_session_conversion_with(
            &mut model,
            vec![
                detected_agent(AgentId::ClaudeCode, Some("1.0.55")),
                detected_agent(AgentId::Codex, Some("0.5.1")),
            ],
        );

        assert!(model.service_select.is_some());
        assert!(model.pending_session_conversion.is_none());
    }

    #[test]
    fn update_service_select_select_sets_pending_conversion_and_opens_confirm() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);
        model.service_select = Some(screens::service_select::ServiceSelectState::with_options(
            "Select Agent",
            vec!["Codex".to_string()],
            vec!["codex".to_string()],
        ));

        update(
            &mut model,
            Message::ServiceSelect(screens::service_select::ServiceSelectMessage::Select),
        );

        assert!(model.service_select.is_none());
        assert_eq!(
            model.pending_session_conversion,
            Some(PendingSessionConversion {
                session_index: 0,
                target_agent_id: "codex".to_string(),
                target_display_name: "Codex".to_string(),
            })
        );
        assert!(model.confirm.visible);
        assert_eq!(model.confirm.message, "Convert session to Codex?");
    }

    #[test]
    fn handle_confirm_message_with_accept_applies_pending_session_conversion_and_logs_info() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);
        model.pending_session_conversion = Some(PendingSessionConversion {
            session_index: 0,
            target_agent_id: "codex".to_string(),
            target_display_name: "Codex".to_string(),
        });
        model.confirm = screens::confirm::ConfirmState::with_message("Convert?");
        model.confirm.selected = screens::confirm::ConfirmChoice::Yes;

        let target = detected_agent(AgentId::Codex, Some("0.5.1"));
        let target_name = target.agent_id.display_name().to_string();
        let target_command = target.agent_id.command().to_string();
        let target_color = tui_agent_color(target.agent_id.default_color());
        handle_confirm_message_with(
            &mut model,
            screens::confirm::ConfirmMessage::Accept,
            vec![target],
        );

        assert_eq!(model.sessions[0].name, target_name);
        assert_eq!(
            model.sessions[0].tab_type,
            SessionTabType::Agent {
                agent_id: target_command,
                color: target_color,
            }
        );
        assert_eq!(model.logs.entries.last().unwrap().source, "session");
        assert!(model.current_notification.is_some());
        assert!(model.pending_session_conversion.is_none());
    }

    #[test]
    fn handle_confirm_message_with_failure_routes_error_queue() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);
        let original_name = model.sessions[0].name.clone();
        let original_tab_type = model.sessions[0].tab_type.clone();
        model.pending_session_conversion = Some(PendingSessionConversion {
            session_index: 0,
            target_agent_id: "missing-agent".to_string(),
            target_display_name: "Missing Agent".to_string(),
        });
        model.confirm = screens::confirm::ConfirmState::with_message("Convert?");
        model.confirm.selected = screens::confirm::ConfirmChoice::Yes;

        handle_confirm_message_with(&mut model, screens::confirm::ConfirmMessage::Accept, vec![]);

        assert_eq!(model.sessions[0].name, original_name);
        assert_eq!(model.sessions[0].tab_type, original_tab_type);
        assert_eq!(model.error_queue.len(), 1);
        assert!(model
            .logs
            .entries
            .last()
            .unwrap()
            .message
            .contains("missing-agent"));
        assert!(model.pending_session_conversion.is_none());
    }

    #[test]
    fn handle_confirm_message_with_accept_removes_pending_branch_worktree() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");

        let worktree_path = dir.path().join("wt-feature-delete");
        let output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                worktree_path.to_str().expect("worktree path"),
                "-b",
                "feature/delete-me",
            ])
            .current_dir(dir.path())
            .output()
            .expect("git worktree add");
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let mut model = Model::new(dir.path().to_path_buf());
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/delete-me".into(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(worktree_path.clone()),
        }];

        update(
            &mut model,
            Message::Branches(screens::branches::BranchesMessage::DeleteWorktree),
        );
        assert!(model.confirm.visible);
        model.confirm.selected = screens::confirm::ConfirmChoice::Yes;

        handle_confirm_message_with(&mut model, screens::confirm::ConfirmMessage::Accept, vec![]);

        assert!(!worktree_path.exists(), "worktree should be removed");
        assert!(!model.branches.pending_delete_worktree);
        let notification = model
            .current_notification
            .as_ref()
            .expect("worktree notification");
        assert_eq!(notification.source, "worktree");
        assert_eq!(
            notification.message,
            "Removed worktree for feature/delete-me"
        );
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
    fn update_key_input_esc_dismisses_warn_notification_when_unclaimed() {
        let mut model = test_model();
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Esc, KeyModifiers::NONE)),
        );

        assert!(model.current_notification.is_none());
    }

    #[test]
    fn update_key_input_esc_preserves_warn_notification_during_branch_search() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.branches.search_active = true;
        model.branches.search_query = "detached".into();
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Esc, KeyModifiers::NONE)),
        );

        assert!(model.current_notification.is_some());
        assert!(!model.branches.search_active);
        assert!(model.branches.search_query.is_empty());
    }

    #[test]
    fn update_key_input_esc_preserves_warn_notification_during_settings_edit() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Settings;
        model.settings.editing = true;
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Esc, KeyModifiers::NONE)),
        );

        assert!(model.current_notification.is_some());
        assert!(!model.settings.editing);
    }

    #[test]
    fn update_key_input_esc_preserves_warn_notification_during_issue_search() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Issues;
        model.issues.search_active = true;
        model.issues.search_query = "warn".into();
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Esc, KeyModifiers::NONE)),
        );

        assert!(model.current_notification.is_some());
        assert!(!model.issues.search_active);
        assert!(model.issues.search_query.is_empty());
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
        let notification =
            Notification::new(Severity::Error, "pty", "Crashed").with_detail("stack trace");

        update(&mut model, Message::Notify(notification));

        assert_eq!(model.error_queue.len(), 1);
        let queued = model.error_queue.front().unwrap();
        assert_eq!(queued.severity, Severity::Error);
        assert_eq!(queued.source, "pty");
        assert_eq!(queued.message, "Crashed");
        assert_eq!(queued.detail.as_deref(), Some("stack trace"));
        assert_eq!(model.logs.entries.len(), 1);
        assert_eq!(model.logs.entries[0].source, "pty");
        assert!(model.current_notification.is_none());
    }

    #[test]
    fn update_notify_debug_logs_without_ui_surface() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Debug, "pty", "raw bytes");

        update(&mut model, Message::Notify(notification));

        assert_eq!(model.logs.entries.len(), 1);
        assert_eq!(model.logs.entries[0].severity, Severity::Debug);
        assert!(model.current_notification.is_none());
        assert!(model.error_queue.is_empty());
    }

    #[test]
    fn route_key_to_management_logs_f_cycles_filter_levels() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;

        route_key_to_management(&mut model, key(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(
            model.logs.filter_level,
            screens::logs::FilterLevel::ErrorOnly
        );

        route_key_to_management(&mut model, key(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(model.logs.filter_level, screens::logs::FilterLevel::WarnUp);
    }

    #[test]
    fn route_key_to_management_logs_d_toggles_debug_filter() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;

        route_key_to_management(&mut model, key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(model.logs.filter_level, screens::logs::FilterLevel::DebugUp);

        route_key_to_management(&mut model, key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(model.logs.filter_level, screens::logs::FilterLevel::All);
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
    fn route_key_to_management_git_view_refresh_reloads_repository_data() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");
        fs::write(dir.path().join("tracked.txt"), "one\n").expect("write tracked file");
        let add = std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(dir.path())
            .output()
            .expect("git add tracked file");
        assert!(add.status.success(), "git add failed");
        git_commit_allow_empty(dir.path(), "add tracked file");

        let mut model = Model::new(dir.path().to_path_buf());
        model.management_tab = ManagementTab::GitView;
        load_initial_data(&mut model);
        assert!(model.git_view.files.is_empty());

        fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").expect("modify tracked file");

        route_key_to_management(&mut model, key(KeyCode::Char('r'), KeyModifiers::NONE));

        assert_eq!(model.git_view.files.len(), 1);
        assert_eq!(model.git_view.files[0].path, "tracked.txt");
    }

    #[test]
    fn update_toggle_help_flips_overlay_visibility() {
        let mut model = test_model();
        assert!(!model.help_visible);

        update(&mut model, Message::ToggleHelp);
        assert!(model.help_visible);

        update(&mut model, Message::ToggleHelp);
        assert!(!model.help_visible);
    }

    #[test]
    fn route_overlay_key_escape_closes_help_overlay() {
        let mut model = test_model();
        model.help_visible = true;

        assert!(route_overlay_key(
            &mut model,
            key(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert!(!model.help_visible);
    }

    #[test]
    fn route_overlay_key_escape_hides_docker_progress_overlay() {
        let mut model = test_model();
        model.docker_progress = Some(screens::docker_progress::DockerProgressState {
            visible: true,
            stage: screens::docker_progress::DockerStage::StartingContainer,
            message: "Starting container web".into(),
            error: None,
        });

        assert!(route_overlay_key(
            &mut model,
            key(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert!(model.docker_progress.is_none());
    }

    #[test]
    fn render_help_overlay_lists_all_registered_keybindings_only() {
        let mut model = test_model();
        model.help_visible = true;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("create terminal");
        terminal
            .draw(|frame| view(&model, frame))
            .expect("render help overlay");

        let text = buffer_text(terminal.backend().buffer());
        let registry = crate::input::keybind::KeybindRegistry::new();

        for binding in registry.all_bindings() {
            assert!(
                text.contains(&binding.keys),
                "expected help overlay to contain binding {}",
                binding.keys
            );
            assert!(
                text.contains(&binding.description),
                "expected help overlay to contain description {}",
                binding.description
            );
        }

        assert!(!text.contains("Ctrl+G, y"));
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
    fn update_key_input_routes_to_service_select_overlay() {
        let mut model = test_model();
        model.service_select = Some(screens::service_select::ServiceSelectState {
            title: "Select Agent".into(),
            services: vec!["claude".into(), "codex".into()],
            values: vec!["claude".into(), "codex".into()],
            selected: 0,
            visible: true,
        });

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Down, KeyModifiers::NONE)),
        );

        assert_eq!(model.service_select.as_ref().unwrap().selected, 1);
    }

    #[test]
    fn route_key_to_branch_detail_overview_moves_docker_selection() {
        let mut model = test_model();
        model.branches.detail_section = 0;
        model.branches.docker_containers = vec![
            docker_container("abc123", "web", gwt_docker::ContainerStatus::Running),
            docker_container("def456", "db", gwt_docker::ContainerStatus::Stopped),
        ];

        route_key_to_branch_detail(&mut model, key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(model.branches.docker_selected, 1);

        route_key_to_branch_detail(&mut model, key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(model.branches.docker_selected, 0);
    }

    #[test]
    fn update_branches_docker_stop_executes_and_refreshes_detail() {
        let tmp = tempfile::tempdir().expect("temp worktree");
        fs::write(
            tmp.path().join("docker-compose.yml"),
            "services:\n  web:\n    image: nginx:latest\n",
        )
        .expect("compose");

        let script = "#!/bin/sh\nif [ \"$1\" = \"stop\" ]; then\n  sleep 0.1\n  exit 0\nfi\nif [ \"$1\" = \"ps\" ]; then\n  printf 'abc123\tweb\texited\tnginx:latest\t0.0.0.0:8080->80/tcp\\n'\n  exit 0\nfi\nexit 0\n";

        with_fake_docker(script, || {
            let mut model = test_model();
            model.branches.branches = vec![screens::branches::BranchItem {
                name: "feature/docker".into(),
                is_head: true,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: Some(tmp.path().to_path_buf()),
            }];
            model.branches.docker_containers = vec![docker_container(
                "abc123",
                "web",
                gwt_docker::ContainerStatus::Running,
            )];

            update(
                &mut model,
                Message::Branches(screens::branches::BranchesMessage::DockerContainerStop),
            );

            assert!(model.branches.pending_docker_action.is_none());
            assert!(model.docker_progress_events.is_some());
            let docker_progress = model.docker_progress.as_ref().expect("docker progress");
            assert!(docker_progress.visible);
            assert_eq!(
                docker_progress.stage,
                screens::docker_progress::DockerStage::StartingContainer
            );
            assert_eq!(docker_progress.message, "Stopping container web");
            assert_eq!(
                model.branches.docker_containers[0].status,
                gwt_docker::ContainerStatus::Running
            );

            drive_docker_worker_until(
                &mut model,
                |model| {
                    model.docker_progress_events.is_none() && model.current_notification.is_some()
                },
                "docker stop completion",
            );

            assert_eq!(model.branches.docker_containers.len(), 1);
            assert_eq!(
                model.branches.docker_containers[0].status,
                gwt_docker::ContainerStatus::Exited
            );
            let docker_progress = model.docker_progress.as_ref().expect("docker progress");
            assert_eq!(
                docker_progress.stage,
                screens::docker_progress::DockerStage::Ready
            );
            assert_eq!(docker_progress.message, "Stopped container web");
            assert!(docker_progress.error.is_none());
            let notification = model
                .current_notification
                .as_ref()
                .expect("status notification");
            assert_eq!(notification.source, "docker");
            assert_eq!(notification.message, "Stopped container web");
            assert!(model.error_queue.is_empty());
        });
    }

    #[test]
    fn update_branches_docker_restart_failure_routes_error_notification() {
        let script = "#!/bin/sh\nif [ \"$1\" = \"restart\" ]; then\n  sleep 0.1\n  printf 'permission denied' >&2\n  exit 1\nfi\nif [ \"$1\" = \"ps\" ]; then\n  printf 'abc123\tweb\trunning\tnginx:latest\t0.0.0.0:8080->80/tcp\\n'\n  exit 0\nfi\nexit 0\n";

        with_fake_docker(script, || {
            let mut model = test_model();
            model.branches.docker_containers = vec![docker_container(
                "abc123",
                "web",
                gwt_docker::ContainerStatus::Running,
            )];

            update(
                &mut model,
                Message::Branches(screens::branches::BranchesMessage::DockerContainerRestart),
            );

            assert!(model.docker_progress_events.is_some());
            let docker_progress = model.docker_progress.as_ref().expect("docker progress");
            assert!(docker_progress.visible);
            assert_eq!(
                docker_progress.stage,
                screens::docker_progress::DockerStage::StartingContainer
            );
            assert_eq!(docker_progress.message, "Restarting container web");

            drive_docker_worker_until(
                &mut model,
                |model| model.docker_progress_events.is_none() && !model.error_queue.is_empty(),
                "docker restart failure",
            );

            assert!(model.current_notification.is_none());
            assert_eq!(model.error_queue.len(), 1);
            let docker_progress = model.docker_progress.as_ref().expect("docker progress");
            assert!(docker_progress.visible);
            assert_eq!(
                docker_progress.stage,
                screens::docker_progress::DockerStage::Failed
            );
            assert!(docker_progress
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("Failed to restart container web"));
            let notification = model.error_queue.front().unwrap();
            assert_eq!(notification.source, "docker");
            assert_eq!(notification.message, "Failed to restart container web");
            assert!(notification
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("permission denied"));
        });
    }

    #[test]
    fn update_docker_progress_set_stage_creates_overlay_when_missing() {
        let mut model = test_model();
        assert!(model.docker_progress.is_none());

        update(
            &mut model,
            Message::DockerProgress(screens::docker_progress::DockerProgressMessage::SetStage {
                stage: screens::docker_progress::DockerStage::BuildingImage,
                message: "Building image".into(),
            }),
        );

        let state = model.docker_progress.as_ref().expect("docker progress");
        assert!(state.visible);
        assert_eq!(
            state.stage,
            screens::docker_progress::DockerStage::BuildingImage
        );
        assert_eq!(state.message, "Building image");
    }

    #[test]
    fn update_docker_progress_hide_drops_overlay() {
        let mut model = test_model();
        model.docker_progress = Some(screens::docker_progress::DockerProgressState {
            visible: true,
            stage: screens::docker_progress::DockerStage::WaitingForServices,
            message: "Waiting".into(),
            error: None,
        });

        update(
            &mut model,
            Message::DockerProgress(screens::docker_progress::DockerProgressMessage::Hide),
        );

        assert!(model.docker_progress.is_none());
    }

    #[test]
    fn update_key_input_routes_to_confirm_overlay() {
        let mut model = test_model();
        model.confirm = screens::confirm::ConfirmState::with_message("Convert?");

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Tab, KeyModifiers::NONE)),
        );

        assert!(model.confirm.accepted());
    }

    #[test]
    fn switch_management_tab_settings_loads_fields() {
        let mut model = test_model();
        assert!(model.settings.fields.is_empty());

        update(
            &mut model,
            Message::SwitchManagementTab(ManagementTab::Settings),
        );

        assert_eq!(model.management_tab, ManagementTab::Settings);
        assert!(!model.settings.fields.is_empty());
    }

    #[test]
    fn update_settings_toggle_bool_syncs_embedded_skill_registry() {
        let mut model = test_model();
        model.settings.category = screens::settings::SettingsCategory::Skills;
        model.settings.load_category_fields();
        model.settings.selected = 0;

        let skill_name = model.settings.fields[0].label.clone();
        assert!(
            model
                .embedded_skills()
                .list()
                .iter()
                .find(|skill| skill.name == skill_name)
                .expect("skill exists")
                .enabled
        );

        update(
            &mut model,
            Message::Settings(screens::settings::SettingsMessage::ToggleBool),
        );

        assert_eq!(model.settings.fields[0].value, "false");
        assert!(
            !model
                .embedded_skills()
                .list()
                .iter()
                .find(|skill| skill.name == skill_name)
                .expect("skill exists")
                .enabled
        );
    }
}
