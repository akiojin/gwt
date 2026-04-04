//! App — Update and View functions for the Elm Architecture.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gwt_agent::{
    AgentDetector, AgentId, AgentLaunchBuilder, DetectedAgent, LaunchConfig,
    Session as AgentSession, SessionMode, VersionCache,
};
use gwt_ai::{suggest_branch_name, AIClient};
use gwt_config::{AISettings, Settings, VoiceConfig};
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
            match model.active_layer {
                ActiveLayer::Initialization => {} // blocked
                ActiveLayer::Main => {
                    model.active_layer = ActiveLayer::Management;
                    model.active_focus = FocusPane::Terminal;
                }
                ActiveLayer::Management => {
                    model.active_layer = ActiveLayer::Main;
                    model.active_focus = FocusPane::Terminal;
                }
            }
        }
        Message::SwitchManagementTab(tab) => {
            switch_management_tab(model, tab);
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
                            model.active_focus = next_management_focus(model, false);
                            return;
                        }
                        KeyCode::BackTab => {
                            model.active_focus = next_management_focus(model, true);
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
            let voice_config = Settings::load()
                .map(|settings| settings.voice)
                .unwrap_or_default();
            let mut runtime = std::mem::take(&mut model.voice_runtime);
            handle_voice_message_with_config_and_runtime(model, msg, &voice_config, &mut runtime);
            model.voice_runtime = runtime;
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
        Message::OpenWizardWithSpec(spec_context) => {
            open_wizard(model, Some(spec_context));
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

    // -- Specs --
    load_specs(model);

    // -- Git View --
    load_git_view_with(
        model,
        gwt_git::diff::get_status,
        |repo_path| gwt_git::commit::recent_commits(repo_path, 10),
        gwt_git::branch::list_branches,
        fetch_current_pr_link,
    );

    load_pr_dashboard(model);
}

fn load_git_view_with<S, C, B, P>(
    model: &mut Model,
    load_status: S,
    load_commits: C,
    load_branches: B,
    load_pr_link: P,
) where
    S: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::diff::FileEntry>>,
    C: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::commit::CommitEntry>>,
    B: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::Branch>>,
    P: FnOnce(&std::path::Path) -> gwt_core::Result<Option<String>>,
{
    if let Ok(entries) = load_status(&model.repo_path) {
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

    if let Ok(entries) = load_commits(&model.repo_path) {
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

    let divergence_summary = load_branches(&model.repo_path)
        .ok()
        .and_then(|branches| git_view_divergence_summary(&branches));
    let pr_link = load_pr_link(&model.repo_path).ok().flatten();
    screens::git_view::update(
        &mut model.git_view,
        screens::git_view::GitViewMessage::SetMetadata {
            divergence_summary,
            pr_link,
        },
    );
}

fn git_view_divergence_summary(branches: &[gwt_git::Branch]) -> Option<String> {
    let current = branches
        .iter()
        .find(|branch| branch.is_head && branch.is_local)?;
    current.upstream.as_ref()?;

    match (current.ahead, current.behind) {
        (0, 0) => Some("Up to date".to_string()),
        (ahead, 0) => Some(format!("Ahead {ahead}")),
        (0, behind) => Some(format!("Behind {behind}")),
        (ahead, behind) => Some(format!("Ahead {ahead} Behind {behind}")),
    }
}

fn fetch_current_pr_link(repo_path: &std::path::Path) -> gwt_core::Result<Option<String>> {
    let output = Command::new("gh")
        .args(["pr", "view", "--json", "url"])
        .current_dir(repo_path)
        .output()
        .map_err(|err| gwt_core::GwtError::Git(format!("gh pr view: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        let lowered = trimmed.to_ascii_lowercase();
        if lowered.contains("no pull requests found")
            || lowered.contains("no pull request found")
            || lowered.contains("could not resolve to a pull request")
        {
            return Ok(None);
        }
        return Err(gwt_core::GwtError::Git(format!("gh pr view: {trimmed}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_current_pr_link_json(&stdout)
}

fn parse_current_pr_link_json(json: &str) -> gwt_core::Result<Option<String>> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| gwt_core::GwtError::Other(format!("gh pr view JSON: {err}")))?;
    Ok(value
        .get("url")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned))
}

fn load_specs(model: &mut Model) {
    model.specs.spec_root = Some(model.repo_path.clone());

    let mut items = Vec::new();
    let specs_dir = model.repo_path.join("specs");
    if let Ok(entries) = std::fs::read_dir(specs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(dir_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !dir_name.starts_with("SPEC-") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path.join("metadata.json")) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
                continue;
            };

            items.push(screens::specs::SpecItem {
                id: value
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(dir_name)
                    .to_string(),
                title: value
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                phase: value
                    .get("phase")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                status: value
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
    }

    items.sort_by(|a, b| spec_sort_key(&a.id).cmp(&spec_sort_key(&b.id)));
    screens::specs::update(
        &mut model.specs,
        screens::specs::SpecsMessage::SetSpecs(items),
    );
}

fn read_spec_body(repo_path: &Path, spec_id: &str) -> String {
    std::fs::read_to_string(repo_path.join("specs").join(spec_id).join("spec.md"))
        .unwrap_or_default()
}

fn spec_sort_key(spec_id: &str) -> (u32, &str) {
    let numeric = spec_id
        .strip_prefix("SPEC-")
        .unwrap_or(spec_id)
        .parse::<u32>()
        .unwrap_or(u32::MAX);
    (numeric, spec_id)
}

fn switch_management_tab(model: &mut Model, tab: ManagementTab) {
    switch_management_tab_with(
        model,
        tab,
        gwt_git::fetch_pr_list,
        fetch_pr_dashboard_detail_report,
    );
}

fn switch_management_tab_with<F, D>(
    model: &mut Model,
    tab: ManagementTab,
    fetch_prs: F,
    fetch_detail: D,
) where
    F: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::PrStatus>>,
    D: FnOnce(&std::path::Path, u32) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport>,
{
    let preserve_terminal_focus =
        model.active_layer == ActiveLayer::Main || model.active_focus == FocusPane::Terminal;
    model.management_tab = tab;
    model.active_layer = ActiveLayer::Management;
    model.active_focus = if preserve_terminal_focus {
        FocusPane::Terminal
    } else {
        FocusPane::TabContent
    };
    if tab == ManagementTab::Settings && model.settings.fields.is_empty() {
        model.settings.load_category_fields();
    }
    if tab == ManagementTab::PrDashboard {
        refresh_pr_dashboard_with(model, fetch_prs, fetch_detail);
    }
}

fn load_pr_dashboard(model: &mut Model) {
    load_pr_dashboard_with(model, gwt_git::fetch_pr_list);
}

fn refresh_pr_dashboard_with<F, D>(model: &mut Model, fetch_prs: F, fetch_detail: D)
where
    F: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::PrStatus>>,
    D: FnOnce(&std::path::Path, u32) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport>,
{
    load_pr_dashboard_with(model, fetch_prs);
    if model.pr_dashboard.detail_view {
        load_pr_dashboard_detail_with(model, fetch_detail);
    }
}

fn load_pr_dashboard_with<F>(model: &mut Model, fetch_prs: F)
where
    F: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::PrStatus>>,
{
    let Ok(prs) = fetch_prs(&model.repo_path) else {
        return;
    };
    let items = prs.into_iter().map(map_pr_item).collect();
    screens::pr_dashboard::update(
        &mut model.pr_dashboard,
        screens::pr_dashboard::PrDashboardMessage::SetPrs(items),
    );
}

fn load_pr_dashboard_detail_with<F>(model: &mut Model, fetch_detail: F)
where
    F: FnOnce(&std::path::Path, u32) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport>,
{
    let Some(pr) = model.pr_dashboard.selected_pr() else {
        return;
    };

    let Ok(detail) = fetch_detail(&model.repo_path, pr.number) else {
        return;
    };

    screens::pr_dashboard::update(
        &mut model.pr_dashboard,
        screens::pr_dashboard::PrDashboardMessage::SetDetailReport(Some(detail)),
    );
}

fn fetch_pr_dashboard_detail_report(
    repo_path: &std::path::Path,
    number: u32,
) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport> {
    let output = std::process::Command::new("gh")
        .args([
            "pr",
            "view",
            &number.to_string(),
            "--json",
            "title,state,mergeable,reviewDecision,statusCheckRollup",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|err| gwt_core::GwtError::Git(format!("gh pr view: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(gwt_core::GwtError::Git(format!(
            "gh pr view: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_pr_dashboard_detail_report_json(&stdout)
}

fn parse_pr_dashboard_detail_report_json(
    json: &str,
) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| gwt_core::GwtError::Other(format!("gh pr view JSON: {err}")))?;

    let ci_status = match value.get("statusCheckRollup") {
        Some(serde_json::Value::Array(checks)) if checks.is_empty() => "pending".to_string(),
        Some(serde_json::Value::Array(checks)) => {
            let any_fail = checks.iter().any(|check| {
                check
                    .get("conclusion")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| matches!(s, "FAILURE" | "CANCELLED" | "TIMED_OUT"))
            });
            let all_pass = checks.iter().all(|check| {
                check
                    .get("conclusion")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| matches!(s, "SUCCESS" | "NEUTRAL" | "SKIPPED"))
            });
            if any_fail {
                "failing".to_string()
            } else if all_pass {
                "passing".to_string()
            } else {
                "pending".to_string()
            }
        }
        _ => "unknown".to_string(),
    };

    let merge_status = match value.get("mergeable").and_then(|v| v.as_str()) {
        Some("MERGEABLE") => "ready".to_string(),
        Some("CONFLICTING") => "conflicts".to_string(),
        Some(_) => "blocked".to_string(),
        None => "unknown".to_string(),
    };

    let review_status = match value.get("reviewDecision").and_then(|v| v.as_str()) {
        Some("APPROVED") => "approved".to_string(),
        Some("CHANGES_REQUESTED") => "changes_requested".to_string(),
        Some("REVIEW_REQUIRED") => "pending".to_string(),
        _ => "unknown".to_string(),
    };

    let checks = value
        .get("statusCheckRollup")
        .and_then(|v| v.as_array())
        .map(|checks| {
            checks
                .iter()
                .map(|check| {
                    let name = check
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let conclusion = check
                        .get("conclusion")
                        .and_then(|v| v.as_str())
                        .or_else(|| check.get("status").and_then(|v| v.as_str()))
                        .unwrap_or("UNKNOWN")
                        .to_ascii_lowercase();
                    format!("{name}: {conclusion}")
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(screens::pr_dashboard::PrDetailReport {
        summary: format!("CI {ci_status}, merge {merge_status}, review {review_status}"),
        ci_status,
        merge_status,
        review_status,
        checks,
    })
}

fn map_pr_item(pr: gwt_git::PrStatus) -> screens::pr_dashboard::PrItem {
    let state = match pr.state {
        gwt_git::pr_status::PrState::Open => screens::pr_dashboard::PrState::Open,
        gwt_git::pr_status::PrState::Closed => screens::pr_dashboard::PrState::Closed,
        gwt_git::pr_status::PrState::Merged => screens::pr_dashboard::PrState::Merged,
    };
    let ci_status = pr.ci_status.to_ascii_lowercase();
    let review_status = pr.review_status.to_ascii_lowercase();
    let mergeable = matches!(pr.mergeable.as_str(), "MERGEABLE" | "mergeable");

    screens::pr_dashboard::PrItem {
        number: pr.number as u32,
        title: pr.title,
        state,
        ci_status,
        mergeable,
        review_status,
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

/// Route a key event to the branch detail pane (sections, session handoff, launch agent).
fn route_key_to_branch_detail(model: &mut Model, key: crossterm::event::KeyEvent) {
    use screens::branches::BranchesMessage;

    let msg = match key.code {
        KeyCode::Left => Some(BranchesMessage::PrevDetailSection),
        KeyCode::Right => Some(BranchesMessage::NextDetailSection),
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::SHIFT)
                && model.branches.detail_section != 3
                && selected_branch_has_worktree(model) =>
        {
            Some(BranchesMessage::OpenShell)
        }
        KeyCode::Up if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerContainerUp)
        }
        KeyCode::Down if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerContainerDown)
        }
        KeyCode::Up if model.branches.detail_section == 3 => {
            let len = branch_session_matches(model).len();
            if len > 0 {
                model.branches.clamp_detail_session_selected(len);
                model.branches.detail_session_selected =
                    if model.branches.detail_session_selected == 0 {
                        len - 1
                    } else {
                        model.branches.detail_session_selected - 1
                    };
            }
            None
        }
        KeyCode::Down if model.branches.detail_section == 3 => {
            let len = branch_session_matches(model).len();
            if len > 0 {
                model.branches.clamp_detail_session_selected(len);
                model.branches.detail_session_selected =
                    (model.branches.detail_session_selected + 1) % len;
            }
            None
        }
        KeyCode::Enter if model.branches.detail_section == 3 => {
            let sessions = branch_session_matches(model);
            model.branches.clamp_detail_session_selected(sessions.len());
            if let Some(selected) = sessions.get(model.branches.detail_session_selected) {
                model.active_session = selected.session_index;
                model.active_focus = FocusPane::Terminal;
            }
            None
        }
        KeyCode::Enter => Some(BranchesMessage::LaunchAgent),
        KeyCode::Char('s') if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerContainerStart)
        }
        KeyCode::Char('t') if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerContainerStop)
        }
        KeyCode::Char('r') if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerContainerRestart)
        }
        KeyCode::Char('m') => {
            screens::branches::update(&mut model.branches, BranchesMessage::ToggleView);
            let repo_path = model.repo_path.clone();
            screens::branches::load_branch_detail(&mut model.branches, &repo_path);
            None
        }
        KeyCode::Char('v') => {
            update(model, Message::SwitchManagementTab(ManagementTab::GitView));
            None
        }
        KeyCode::Char('f') | KeyCode::Char('/') => {
            screens::branches::update(&mut model.branches, BranchesMessage::SearchStart);
            model.active_focus = FocusPane::TabContent;
            None
        }
        KeyCode::Char('?') | KeyCode::Char('h') => {
            update(model, Message::ToggleHelp);
            None
        }
        KeyCode::Char('c')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && model.branches.detail_section != 3
                && selected_branch_has_worktree(model) =>
        {
            Some(BranchesMessage::DeleteWorktree)
        }
        KeyCode::Esc => {
            model.active_focus = FocusPane::TabContent;
            return;
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
    use screens::profiles::ProfilesMessage;
    use screens::settings::SettingsMessage;
    use screens::versions::VersionsMessage;

    let specs_detail_handles_horizontal_navigation =
        model.management_tab == ManagementTab::Specs && model.specs.detail_view;

    // Left/Right switches tabs when not in text input mode.
    // Ctrl+Left/Right is reserved for sub-tab switching within individual tabs,
    // and Specs detail owns plain Left/Right for artifact section navigation.
    if !is_in_text_input_mode(model)
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !specs_detail_handles_horizontal_navigation
    {
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
                KeyCode::Char('m') => Some(BranchesMessage::ToggleView),
                KeyCode::Char('v') => {
                    update(model, Message::SwitchManagementTab(ManagementTab::GitView));
                    return;
                }
                KeyCode::Char('f') | KeyCode::Char('/') => Some(BranchesMessage::SearchStart),
                KeyCode::Char('?') | KeyCode::Char('h') => {
                    update(model, Message::ToggleHelp);
                    return;
                }
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
                fallback_management_escape(model);
            }
        }
        ManagementTab::Specs => {
            use screens::specs::SpecsMessage;

            if model.specs.detail_editing {
                let msg = match key.code {
                    KeyCode::Enter => Some(SpecsMessage::SaveSectionEdit),
                    KeyCode::Esc => Some(SpecsMessage::CancelSectionEdit),
                    KeyCode::Backspace => Some(SpecsMessage::SectionEditBackspace),
                    KeyCode::Char(ch) => Some(SpecsMessage::SectionEditInput(ch)),
                    _ => None,
                };
                if let Some(m) = msg {
                    screens::specs::update(&mut model.specs, m);
                }
                return;
            }

            if model.specs.editing {
                let msg = match key.code {
                    KeyCode::Enter => Some(SpecsMessage::SaveEdit),
                    KeyCode::Esc => Some(SpecsMessage::CancelEdit),
                    KeyCode::Up => Some(SpecsMessage::MoveUp),
                    KeyCode::Down => Some(SpecsMessage::MoveDown),
                    _ => None,
                };
                if let Some(m) = msg {
                    screens::specs::update(&mut model.specs, m);
                }
                return;
            }

            if model.specs.search_active && !model.specs.detail_view {
                let msg = match key.code {
                    KeyCode::Esc => Some(SpecsMessage::SearchClear),
                    KeyCode::Backspace => Some(SpecsMessage::SearchBackspace),
                    _ => search_input_char(&key).map(SpecsMessage::SearchInput),
                };
                if let Some(m) = msg {
                    screens::specs::update(&mut model.specs, m);
                    return;
                }
            }

            let msg = match key.code {
                KeyCode::Down => Some(SpecsMessage::MoveDown),
                KeyCode::Up => Some(SpecsMessage::MoveUp),
                KeyCode::Enter
                    if !key.modifiers.contains(KeyModifiers::SHIFT) && !model.specs.detail_view =>
                {
                    Some(SpecsMessage::ToggleDetail)
                }
                KeyCode::Char('e')
                    if key.modifiers.contains(KeyModifiers::CONTROL) && model.specs.detail_view =>
                {
                    Some(SpecsMessage::StartSectionEdit)
                }
                KeyCode::Char('E') if model.specs.detail_view => Some(SpecsMessage::StartFileEdit),
                KeyCode::Char('e') if model.specs.detail_view => Some(SpecsMessage::StartEdit),
                KeyCode::Char('s') if model.specs.detail_view => {
                    Some(SpecsMessage::StartStatusEdit)
                }
                KeyCode::Esc if model.specs.detail_view => Some(SpecsMessage::ToggleDetail),
                KeyCode::Left if model.specs.detail_view => Some(SpecsMessage::PrevSection),
                KeyCode::Right if model.specs.detail_view => Some(SpecsMessage::NextSection),
                KeyCode::Char('/') => Some(SpecsMessage::SearchStart),
                KeyCode::Char('r') => {
                    load_specs(model);
                    return;
                }
                _ => None,
            };

            if let Some(m) = msg {
                screens::specs::update(&mut model.specs, m);
            } else if key.code == KeyCode::Enter
                && key.modifiers.contains(KeyModifiers::SHIFT)
                && model.specs.detail_view
            {
                if let Some(spec) = model.specs.selected_spec() {
                    let spec_context = screens::wizard::SpecContext::new(
                        spec.id.clone(),
                        spec.title.clone(),
                        read_spec_body(&model.repo_path, &spec.id),
                    );
                    update(model, Message::OpenWizardWithSpec(spec_context));
                }
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
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
            } else if key.code == KeyCode::Esc && model.issues.detail_view {
                screens::issues::update(&mut model.issues, IssuesMessage::ToggleDetail);
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
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
                    fallback_management_escape(model);
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
                    fallback_management_escape(model);
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
            } else if key.code == KeyCode::Esc && model.logs.detail_view {
                screens::logs::update(&mut model.logs, LogsMessage::ToggleDetail);
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
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
                fallback_management_escape(model);
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
                fallback_management_escape(model);
            }
        }
        ManagementTab::PrDashboard => {
            if matches!(
                key.code,
                KeyCode::Down | KeyCode::Up | KeyCode::Enter | KeyCode::Char('r')
            ) {
                route_key_to_management_pr_dashboard_with(
                    model,
                    key,
                    fetch_pr_dashboard_detail_report,
                );
            } else if key.code == KeyCode::Esc && model.pr_dashboard.detail_view {
                screens::pr_dashboard::update(
                    &mut model.pr_dashboard,
                    screens::pr_dashboard::PrDashboardMessage::ToggleDetail,
                );
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
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
                KeyCode::Esc if model.profiles.mode != screens::profiles::ProfileMode::List => {
                    Some(ProfilesMessage::Cancel)
                }
                _ => None,
            };
            if let Some(m) = msg {
                screens::profiles::update(&mut model.profiles, m);
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
            }
        }
    }
}

fn route_key_to_management_pr_dashboard_with<F>(
    model: &mut Model,
    key: crossterm::event::KeyEvent,
    fetch_detail: F,
) where
    F: FnOnce(&std::path::Path, u32) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport>,
{
    use screens::pr_dashboard::PrDashboardMessage;

    let msg = match key.code {
        KeyCode::Down => Some(PrDashboardMessage::MoveDown),
        KeyCode::Up => Some(PrDashboardMessage::MoveUp),
        KeyCode::Enter => Some(PrDashboardMessage::ToggleDetail),
        KeyCode::Char('r') => Some(PrDashboardMessage::Refresh),
        _ => None,
    };

    if let Some(m) = msg {
        let should_open_detail =
            matches!(m, PrDashboardMessage::ToggleDetail) && !model.pr_dashboard.detail_view;
        let should_refresh = matches!(m, PrDashboardMessage::Refresh);
        let should_reload_detail_selection = model.pr_dashboard.detail_view
            && matches!(m, PrDashboardMessage::MoveUp | PrDashboardMessage::MoveDown);
        screens::pr_dashboard::update(&mut model.pr_dashboard, m);
        if (should_open_detail && model.pr_dashboard.detail_view) || should_reload_detail_selection
        {
            load_pr_dashboard_detail_with(model, fetch_detail);
        } else if should_refresh {
            refresh_pr_dashboard_with(model, gwt_git::fetch_pr_list, fetch_detail);
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
                configure_existing_branch_wizard_with_sessions(
                    wizard,
                    &model.repo_path,
                    &gwt_sessions_dir(),
                    &branch_name,
                );
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
        if let Some(branch) = model
            .branches
            .selected_branch()
            .filter(|branch| branch.worktree_path.is_some())
        {
            model.confirm = screens::confirm::ConfirmState::with_message(format!(
                "Delete worktree for {}?",
                branch.name
            ));
        } else {
            model.branches.pending_delete_worktree = false;
        }
    }
}

fn selected_branch_has_worktree(model: &Model) -> bool {
    model
        .branches
        .selected_branch()
        .is_some_and(|branch| branch.worktree_path.is_some())
}

/// Build lightweight summaries of active sessions associated with the selected branch.
fn branch_session_summaries(model: &Model) -> Vec<screens::branches::DetailSessionSummary> {
    branch_session_matches(model)
        .into_iter()
        .map(|entry| entry.summary)
        .collect()
}

#[cfg_attr(not(test), allow(dead_code))]
fn branch_session_summaries_with(
    model: &Model,
    sessions_dir: &Path,
) -> Vec<screens::branches::DetailSessionSummary> {
    branch_session_matches_with(model, sessions_dir)
        .into_iter()
        .map(|entry| entry.summary)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BranchSessionMatch {
    session_index: usize,
    summary: screens::branches::DetailSessionSummary,
}

fn branch_session_matches(model: &Model) -> Vec<BranchSessionMatch> {
    branch_session_matches_with(model, &gwt_sessions_dir())
}

fn branch_session_matches_with(model: &Model, sessions_dir: &Path) -> Vec<BranchSessionMatch> {
    let Some(branch) = model.branches.selected_branch() else {
        return Vec::new();
    };

    let branch_name = branch.name.as_str();
    let branch_worktree = branch.worktree_path.as_deref().unwrap_or(model.repo_path());
    let branch_shell_name = format!("Shell: {branch_name}");

    model
        .sessions
        .iter()
        .enumerate()
        .filter_map(|(index, session)| match &session.tab_type {
            SessionTabType::Shell if session.name == branch_shell_name => {
                Some(BranchSessionMatch {
                    session_index: index,
                    summary: screens::branches::DetailSessionSummary {
                        kind: "Shell",
                        name: session.name.clone(),
                        detail: None,
                        active: index == model.active_session,
                    },
                })
            }
            SessionTabType::Agent { .. } => {
                let path = sessions_dir.join(format!("{}.toml", session.id));
                let persisted = AgentSession::load(&path).ok()?;
                if persisted.branch != branch_name || persisted.worktree_path != branch_worktree {
                    return None;
                }

                let detail = match (
                    persisted.model.as_deref(),
                    persisted.reasoning_level.as_deref(),
                ) {
                    (Some(model), Some(reasoning)) => Some(format!("{model} · {reasoning}")),
                    (Some(model), None) => Some(model.to_string()),
                    (None, Some(reasoning)) => Some(reasoning.to_string()),
                    (None, None) => None,
                };

                Some(BranchSessionMatch {
                    session_index: index,
                    summary: screens::branches::DetailSessionSummary {
                        kind: "Agent",
                        name: session.name.clone(),
                        detail,
                        active: index == model.active_session,
                    },
                })
            }
            _ => None,
        })
        .collect()
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

fn fallback_management_escape(model: &mut Model) {
    if matches!(
        model
            .current_notification
            .as_ref()
            .map(|notification| notification.severity),
        Some(Severity::Warn)
    ) {
        update(model, Message::DismissNotification);
    } else {
        model.active_focus = FocusPane::Terminal;
    }
}

fn next_management_focus(model: &Model, reverse: bool) -> FocusPane {
    if model.management_tab == ManagementTab::Branches {
        return if reverse {
            model.active_focus.prev()
        } else {
            model.active_focus.next()
        };
    }

    match (model.active_focus, reverse) {
        (FocusPane::Terminal, false) => FocusPane::TabContent,
        (FocusPane::TabContent, false) => FocusPane::Terminal,
        (FocusPane::BranchDetail, false) => FocusPane::Terminal,
        (FocusPane::Terminal, true) => FocusPane::TabContent,
        (FocusPane::TabContent, true) => FocusPane::Terminal,
        (FocusPane::BranchDetail, true) => FocusPane::TabContent,
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

trait VoiceRuntime {
    fn configure(&mut self, config: &VoiceConfig);
    fn start_recording(&mut self) -> Result<(), String>;
    fn stop_and_transcribe(&mut self) -> Result<String, String>;
    fn reset(&mut self);
}

impl VoiceRuntime for crate::model::VoiceRuntimeState {
    fn configure(&mut self, config: &VoiceConfig) {
        crate::model::VoiceRuntimeState::configure(self, config);
    }

    fn start_recording(&mut self) -> Result<(), String> {
        crate::model::VoiceRuntimeState::start_recording(self)
    }

    fn stop_and_transcribe(&mut self) -> Result<String, String> {
        crate::model::VoiceRuntimeState::stop_and_transcribe(self)
    }

    fn reset(&mut self) {
        crate::model::VoiceRuntimeState::reset(self);
    }
}

#[cfg(test)]
fn handle_voice_message_with_runtime<R>(
    model: &mut Model,
    msg: VoiceInputMessage,
    voice_enabled: bool,
    runtime: &mut R,
) where
    R: VoiceRuntime,
{
    let voice_config = VoiceConfig {
        enabled: voice_enabled,
        ..VoiceConfig::default()
    };
    handle_voice_message_with_config_and_runtime(model, msg, &voice_config, runtime);
}

fn handle_voice_message_with_config_and_runtime<R>(
    model: &mut Model,
    msg: VoiceInputMessage,
    voice_config: &VoiceConfig,
    runtime: &mut R,
) where
    R: VoiceRuntime,
{
    runtime.configure(voice_config);

    match msg {
        VoiceInputMessage::StartRecording if !voice_config.enabled => {}
        VoiceInputMessage::StartRecording
            if model.voice.status == crate::input::voice::VoiceStatus::Recording =>
        {
            complete_voice_transcription(model, runtime);
        }
        VoiceInputMessage::StartRecording => match runtime.start_recording() {
            Ok(()) => {
                crate::input::voice::update(&mut model.voice, VoiceInputMessage::StartRecording)
            }
            Err(err) => {
                runtime.reset();
                crate::input::voice::update(
                    &mut model.voice,
                    VoiceInputMessage::TranscriptionError(err),
                );
            }
        },
        VoiceInputMessage::StopRecording => {
            if model.voice.status == crate::input::voice::VoiceStatus::Recording {
                complete_voice_transcription(model, runtime);
            } else {
                runtime.reset();
                crate::input::voice::update(
                    &mut model.voice,
                    VoiceInputMessage::TranscriptionError("Not currently recording".into()),
                );
            }
        }
        other => handle_voice_message(model, other, voice_config.enabled),
    }
}

fn complete_voice_transcription<R>(model: &mut Model, runtime: &mut R)
where
    R: VoiceRuntime,
{
    crate::input::voice::update(&mut model.voice, VoiceInputMessage::StopRecording);
    match runtime.stop_and_transcribe() {
        Ok(text) => {
            crate::input::voice::update(
                &mut model.voice,
                VoiceInputMessage::TranscriptionResult(text.clone()),
            );
            if !text.trim().is_empty() {
                push_input_to_active_session(model, text.into_bytes());
            }
        }
        Err(err) => {
            runtime.reset();
            crate::input::voice::update(
                &mut model.voice,
                VoiceInputMessage::TranscriptionError(err),
            );
        }
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
    if let Some(spec_context) = wizard.spec_context.as_ref() {
        let spec_body = spec_context.spec_body.trim();
        if !spec_body.is_empty() {
            parts.push(format!("SPEC body:\n{spec_body}"));
        }
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
    let session_mode = match wizard.mode.as_str() {
        "continue" => SessionMode::Continue,
        "resume" if wizard.resume_session_id.is_some() => SessionMode::Resume,
        "resume" => SessionMode::Continue,
        _ => SessionMode::Normal,
    };
    builder = builder.session_mode(session_mode);
    if let Some(resume_session_id) = wizard.resume_session_id.as_deref() {
        builder = builder.resume_session_id(resume_session_id);
    }

    let mut config = builder.build();
    if wizard.agent_id == "codex" && !wizard.reasoning.is_empty() {
        config.reasoning_level = Some(wizard.reasoning.clone());
    }
    config
}

fn is_explicit_model_selection(model: &str) -> bool {
    !model.is_empty() && !model.starts_with("Default")
}

fn materialize_pending_launch(model: &mut Model) {
    if let Err(err) = materialize_pending_launch_with(model, &gwt_sessions_dir()) {
        apply_notification(
            model,
            Notification::new(
                Severity::Warn,
                "session",
                "Launch metadata was not persisted",
            )
            .with_detail(err),
        );
    }
}

fn materialize_pending_launch_with(
    model: &mut Model,
    sessions_dir: &std::path::Path,
) -> Result<(), String> {
    let Some(config) = model.pending_launch_config.take() else {
        return Ok(());
    };

    let mut session = AgentSession::new(
        model.repo_path.clone(),
        config.branch.clone().unwrap_or_default(),
        config.agent_id.clone(),
    );
    session.model = config
        .model
        .clone()
        .filter(|model| is_explicit_model_selection(model));
    session.reasoning_level = config.reasoning_level.clone();
    session.tool_version = config.tool_version.clone();
    session.agent_session_id = config.resume_session_id.clone();
    session.skip_permissions = config.skip_permissions;
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

fn configure_existing_branch_wizard_with_sessions(
    wizard: &mut screens::wizard::WizardState,
    repo_path: &std::path::Path,
    sessions_dir: &std::path::Path,
    branch_name: &str,
) {
    wizard.is_new_branch = false;
    wizard.branch_name = branch_name.to_string();
    wizard.quick_start_entries = load_quick_start_entries(repo_path, sessions_dir, branch_name);
    wizard.has_quick_start = !wizard.quick_start_entries.is_empty();
    wizard.step = if wizard.has_quick_start {
        screens::wizard::WizardStep::QuickStart
    } else {
        screens::wizard::WizardStep::BranchAction
    };
    wizard.selected = 0;
}

fn load_quick_start_entries(
    repo_path: &std::path::Path,
    sessions_dir: &std::path::Path,
    branch_name: &str,
) -> Vec<screens::wizard::QuickStartEntry> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };

    let mut latest_by_agent: HashMap<String, AgentSession> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        let Ok(session) = AgentSession::load(&path) else {
            continue;
        };
        if session.branch != branch_name || session.worktree_path != repo_path {
            continue;
        }

        let agent_key = session.agent_id.command().to_string();
        let should_replace = latest_by_agent
            .get(&agent_key)
            .map(|current| {
                session.updated_at > current.updated_at
                    || (session.updated_at == current.updated_at
                        && session.created_at > current.created_at)
            })
            .unwrap_or(true);
        if should_replace {
            latest_by_agent.insert(agent_key, session);
        }
    }

    let mut sessions = latest_by_agent.into_values().collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
    });

    sessions
        .into_iter()
        .map(|session| screens::wizard::QuickStartEntry {
            agent_id: session.agent_id.command().to_string(),
            tool_label: session.display_name.clone(),
            model: session.model.clone(),
            reasoning: session.reasoning_level.clone(),
            version: session.tool_version.clone(),
            resume_session_id: session.agent_session_id.clone(),
            skip_permissions: session.skip_permissions,
        })
        .collect()
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
        .and_then(|ctx| ctx.branch_seed())
        .unwrap_or_default();
    let starts_new_branch = spec_context.is_some();

    let mut wizard = screens::wizard::WizardState {
        step: if starts_new_branch {
            screens::wizard::WizardStep::BranchTypeSelect
        } else {
            screens::wizard::WizardStep::BranchAction
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
        let detected = detected_agents.iter().find(|d| &d.agent_id == builtin_id);
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
        ManagementTab::Specs => {
            model.specs.search_active || model.specs.editing || model.specs.detail_editing
        }
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
        management_split(main_area)[1]
    } else {
        main_area
    };

    session_content_area(model, session_area)
}

fn management_split(area: Rect) -> [Rect; 2] {
    let management_percentage = if area.width >= 120 { 40 } else { 50 };
    let lr = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(management_percentage),
            Constraint::Percentage(100 - management_percentage),
        ])
        .split(area);
    [lr[0], lr[1]]
}

fn session_content_area(model: &Model, session_area: Rect) -> Option<Rect> {
    match model.session_layout {
        SessionLayout::Tab => {
            model.active_session_tab()?;
            Some(
                pane_block(
                    build_session_title(model, session_area.width),
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
        let lr = management_split(main_area);

        render_management_panes(model, frame, lr[0]);
        render_session_pane(model, frame, lr[1]);
    } else {
        render_session_pane(model, frame, main_area);
    }

    render_keybind_hints(model, frame, hint_area);

    // Overlays on top
    render_overlays(model, frame, size);
}

/// Build a bordered block with focus-aware border color (Cyan when focused, Gray otherwise).
fn pane_block(title: Line<'static>, is_focused: bool) -> Block<'static> {
    let border_color = if is_focused { Color::Cyan } else { Color::Gray };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title)
}

/// Build the management tab title line for embedding in a pane border.
fn management_tab_title(model: &Model, width: u16) -> Line<'static> {
    if should_compact_management_tab_title(width) {
        return Line::from(vec![Span::styled(
            format!(" {} ", model.management_tab.label()),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]);
    }

    let labels: Vec<&str> = ManagementTab::ALL.iter().map(|t| t.label()).collect();
    let active_idx = ManagementTab::ALL
        .iter()
        .position(|t| *t == model.management_tab)
        .unwrap_or(0);
    screens::build_tab_title(&labels, active_idx)
}

fn should_compact_management_tab_title(width: u16) -> bool {
    let available_title_width = width.saturating_sub(2) as usize;
    let full_strip_width: usize = ManagementTab::ALL
        .iter()
        .map(|tab| tab.label().chars().count() + 2)
        .sum::<usize>()
        + ManagementTab::ALL.len().saturating_sub(1);
    full_strip_width > available_title_width
}

/// Render the management panes (left side — 2 stacked for Branches, 1 for others).
fn render_management_panes(model: &Model, frame: &mut Frame, area: Rect) {
    if model.management_tab == ManagementTab::Branches {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Top pane: management tab names in title, branch list content
        let list_focused = model.active_focus == FocusPane::TabContent;
        let list_block = pane_block(management_tab_title(model, chunks[0].width), list_focused);
        let list_inner = list_block.inner(chunks[0]);
        frame.render_widget(list_block, chunks[0]);
        screens::branches::render_list(&model.branches, frame, list_inner);

        // Bottom pane: detail section names in title, detail content
        let detail_focused = model.active_focus == FocusPane::BranchDetail;
        let detail_title = branch_detail_title(model);
        let detail_block = pane_block(detail_title, detail_focused);
        let detail_inner = detail_block.inner(chunks[1]);
        frame.render_widget(detail_block, chunks[1]);
        let branch_sessions = branch_session_summaries(model);
        screens::branches::render_detail_content(
            &model.branches,
            frame,
            detail_inner,
            &branch_sessions,
        );
    } else {
        // Single pane for all other tabs
        let focused = model.active_focus == FocusPane::TabContent;
        let block = pane_block(management_tab_title(model, area.width), focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        render_management_tab_content(model, frame, inner);
    }
}

fn branch_detail_title(model: &Model) -> Line<'static> {
    let detail_labels: Vec<&str> = screens::branches::detail_section_labels().to_vec();
    let mut title = screens::build_tab_title(&detail_labels, model.branches.detail_section);
    title
        .spans
        .push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
    let branch_label = model
        .branches
        .selected_branch()
        .map(|branch| branch.name.clone())
        .unwrap_or_else(|| "No branch selected".to_string());
    title.spans.push(Span::styled(
        branch_label,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));
    title
}

/// Render the content of the active management tab (non-Branches).
fn render_management_tab_content(model: &Model, frame: &mut Frame, area: Rect) {
    match model.management_tab {
        ManagementTab::Branches => {
            // Handled by render_management_panes directly
        }
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

/// Render the session pane (right side, or full screen).
fn render_session_pane(model: &Model, frame: &mut Frame, area: Rect) {
    let terminal_focused = model.active_focus == FocusPane::Terminal;
    match model.session_layout {
        SessionLayout::Tab => {
            if let Some(session) = model.active_session_tab() {
                let title = build_session_title(model, area.width);
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
fn build_session_title(model: &Model, width: u16) -> Line<'static> {
    if should_compact_session_title(model, width) {
        if let Some(active) = model.active_session_tab() {
            let label = format!(" {} {} ", active.tab_type.icon(), active.name);
            return Line::from(vec![Span::styled(
                label,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]);
        }
    }

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

fn should_compact_session_title(model: &Model, width: u16) -> bool {
    let available_title_width = width.saturating_sub(2) as usize;
    if available_title_width == 0 {
        return false;
    }

    let full_strip_width: usize = model
        .sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let label_width = format!(" {} {} ", session.tab_type.icon(), session.name).len();
            if i == 0 {
                label_width
            } else {
                label_width + "│".len()
            }
        })
        .sum();

    full_strip_width > available_title_width
}

/// Render context-sensitive keybind hints at the bottom of the screen.
///
/// The status bar keeps session context visible and appends the relevant hints.
fn render_keybind_hints(model: &Model, frame: &mut Frame, area: Rect) {
    let compact = area.width <= 80;
    let hints = match model.active_focus {
        FocusPane::TabContent if model.management_tab == ManagementTab::Branches => {
            branches_list_hint_text(compact)
        }
        FocusPane::TabContent => management_hint_text(model, compact),
        FocusPane::BranchDetail => branch_detail_hint_text(model, compact),
        FocusPane::Terminal => terminal_hint_text(),
    };

    crate::widgets::status_bar::render_with_notification_and_hints(
        model,
        model.current_notification.as_ref(),
        Some(&hints),
        frame,
        area,
    );
}

fn terminal_hint_text() -> String {
    "Ctrl+G:b/i/s g c []/1-9 z ?  Tab:focus  ^C×2".to_string()
}

fn branches_list_hint_text(compact: bool) -> String {
    if compact {
        "↑↓ mv  ←→ tab  ↵ wiz  S↵ sh  Sp dtl  ^C del  mvf?  Esc→T".to_string()
    } else {
        "↑↓:move  ←→:tab  Enter:wizard  Shift+Enter:shell  Space:detail  Ctrl+C:delete  m:view  v:git  f:search  Esc:term  ?:help".to_string()
    }
}

fn management_hint_text(model: &Model, compact: bool) -> String {
    match model.management_tab {
        ManagementTab::Branches => branches_list_hint_text(compact),
        ManagementTab::Issues => {
            if model.issues.detail_view {
                generic_management_hint_text(compact, false, "back")
            } else {
                generic_management_hint_text(compact, false, "term")
            }
        }
        ManagementTab::Settings => {
            if model.settings.editing {
                settings_edit_hint_text(compact)
            } else {
                generic_management_hint_text(compact, true, "term")
            }
        }
        ManagementTab::Logs => {
            if model.logs.detail_view {
                generic_management_hint_text(compact, true, "back")
            } else {
                generic_management_hint_text(compact, true, "term")
            }
        }
        ManagementTab::PrDashboard => {
            if model.pr_dashboard.detail_view {
                generic_management_hint_text(compact, false, "back")
            } else {
                generic_management_hint_text(compact, false, "term")
            }
        }
        ManagementTab::Profiles => {
            if model.profiles.mode != screens::profiles::ProfileMode::List {
                generic_management_hint_text(compact, false, "cancel")
            } else {
                generic_management_hint_text(compact, false, "term")
            }
        }
        ManagementTab::Specs => {
            if model.specs.detail_editing {
                generic_management_hint_text(compact, false, "cancel")
            } else if model.specs.detail_view {
                generic_management_hint_text(compact, false, "back")
            } else {
                generic_management_hint_text(compact, false, "term")
            }
        }
        ManagementTab::GitView | ManagementTab::Versions => {
            generic_management_hint_text(compact, false, "term")
        }
    }
}

fn generic_management_hint_text(
    compact: bool,
    include_sub_tab: bool,
    escape_action: &str,
) -> String {
    let compact_sub_tab = if include_sub_tab {
        "  C-←→ sub"
    } else {
        ""
    };
    let full_sub_tab = if include_sub_tab {
        "  Ctrl+←→:sub-tab"
    } else {
        ""
    };

    if compact {
        format!("↑↓ sel  ←→ tab{compact_sub_tab}  ↵ act  Tab pane  Esc {escape_action}  ?")
    } else {
        format!(
            "↑↓:select  ←→:tab{full_sub_tab}  Enter:action  Tab:focus  Esc:{escape_action}  ?:help"
        )
    }
}

fn settings_edit_hint_text(compact: bool) -> String {
    if compact {
        "↵ save  ⌫ del  Esc cancel  ?".to_string()
    } else {
        "Enter:save  Backspace:delete  Esc:cancel  ?:help".to_string()
    }
}

fn branch_detail_hint_text(model: &Model, compact: bool) -> String {
    let direct_action_hints = if selected_branch_has_worktree(model) {
        "  Shift+Enter:shell  Ctrl+C:delete"
    } else {
        ""
    };
    let local_mnemonics = "  m:view  v:git  f:search  ?:help";
    if compact {
        let direct_action_hints = if selected_branch_has_worktree(model) {
            "  S↵ sh  ^C del"
        } else {
            ""
        };
        let docker_hints = model
            .branches
            .docker_containers
            .get(model.branches.docker_selected)
            .map(|container| match container.status {
                gwt_docker::ContainerStatus::Running => "  T/R",
                gwt_docker::ContainerStatus::Paused => "  S/T/R",
                gwt_docker::ContainerStatus::Created
                | gwt_docker::ContainerStatus::Stopped
                | gwt_docker::ContainerStatus::Exited => "  S/R",
            })
            .unwrap_or("");
        return match model.branches.detail_section {
            0 => format!("←→ sec  ↵ act{direct_action_hints}{docker_hints}  mvf?  Tab↔P  Esc←"),
            3 => "↑↓ ses  ←→ sec  ↵ focus  mvf?  Tab↔P  Esc←".to_string(),
            _ => format!("←→ sec  ↵ act{direct_action_hints}  mvf?  Tab↔P  Esc←"),
        };
    }
    match model.branches.detail_section {
        0 => {
            let docker_hints = model
                .branches
                .docker_containers
                .get(model.branches.docker_selected)
                .map(|container| match container.status {
                    gwt_docker::ContainerStatus::Running => "  T:stop  R:restart",
                    gwt_docker::ContainerStatus::Paused => "  S:start  T:stop  R:restart",
                    gwt_docker::ContainerStatus::Created
                    | gwt_docker::ContainerStatus::Stopped
                    | gwt_docker::ContainerStatus::Exited => "  S:start  R:restart",
                })
                .unwrap_or("");
            format!(
                "←→:section  Enter:launch{direct_action_hints}{docker_hints}{local_mnemonics}  Tab:focus  Esc:back"
            )
        }
        3 => format!("↑↓:session  ←→:section  Enter:focus{local_mnemonics}  Tab:focus  Esc:back"),
        _ => format!(
            "←→:section  Enter:launch{direct_action_hints}{local_mnemonics}  Tab:focus  Esc:back"
        ),
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
    use gwt_git::pr_status::PrState as GitPrState;
    use gwt_notification::{Notification, Severity};
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui::text::Line;
    use ratatui::widgets::Widget;
    use ratatui::{buffer::Buffer, Terminal};
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn test_model() -> Model {
        Model::new(PathBuf::from("/tmp/test"))
    }

    #[derive(Debug)]
    struct FakeVoiceRuntime {
        start_result: Result<(), String>,
        stop_result: Result<String, String>,
    }

    impl FakeVoiceRuntime {
        fn success(transcript: &str) -> Self {
            Self {
                start_result: Ok(()),
                stop_result: Ok(transcript.to_string()),
            }
        }

        fn start_error(message: &str) -> Self {
            Self {
                start_result: Err(message.to_string()),
                stop_result: Ok(String::new()),
            }
        }

        fn stop_error(message: &str) -> Self {
            Self {
                start_result: Ok(()),
                stop_result: Err(message.to_string()),
            }
        }
    }

    impl VoiceRuntime for FakeVoiceRuntime {
        fn configure(&mut self, _config: &VoiceConfig) {}

        fn start_recording(&mut self) -> Result<(), String> {
            self.start_result.clone()
        }

        fn stop_and_transcribe(&mut self) -> Result<String, String> {
            self.stop_result.clone()
        }

        fn reset(&mut self) {}
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

    #[allow(clippy::too_many_arguments)]
    fn persist_agent_session(
        dir: &std::path::Path,
        repo_path: &str,
        branch: &str,
        agent_id: AgentId,
        updated_at: chrono::DateTime<Utc>,
        model: Option<&str>,
        reasoning_level: Option<&str>,
        tool_version: Option<&str>,
        resume_session_id: Option<&str>,
        skip_permissions: bool,
    ) {
        let mut session = AgentSession::new(repo_path, branch, agent_id);
        session.model = model.map(str::to_string);
        session.reasoning_level = reasoning_level.map(str::to_string);
        session.tool_version = tool_version.map(str::to_string);
        session.agent_session_id = resume_session_id.map(str::to_string);
        session.skip_permissions = skip_permissions;
        session.updated_at = updated_at;
        session.created_at = updated_at;
        session.last_activity_at = updated_at;
        session.save(dir).expect("persist session");
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
    fn pane_block_uses_cyan_border_when_focused() {
        let area = Rect::new(0, 0, 12, 3);
        let mut buffer = Buffer::empty(area);

        pane_block(Line::from("Focused"), true).render(area, &mut buffer);

        assert_eq!(buffer[(0, 0)].fg, Color::Cyan);
    }

    #[test]
    fn pane_block_uses_gray_border_when_unfocused() {
        let area = Rect::new(0, 0, 12, 3);
        let mut buffer = Buffer::empty(area);

        pane_block(Line::from("Unfocused"), false).render(area, &mut buffer);

        assert_eq!(buffer[(0, 0)].fg, Color::Gray);
    }

    #[test]
    fn management_split_uses_50_50_at_standard_width_and_40_60_at_wide_width() {
        let standard = Rect::new(0, 0, 100, 20);
        let [standard_management, standard_session] = management_split(standard);
        assert_eq!(standard_management, Rect::new(0, 0, 50, 20));
        assert_eq!(standard_session, Rect::new(50, 0, 50, 20));

        let wide = Rect::new(0, 0, 120, 20);
        let [wide_management, wide_session] = management_split(wide);
        assert_eq!(wide_management, Rect::new(0, 0, 48, 20));
        assert_eq!(wide_session, Rect::new(48, 0, 72, 20));
    }

    #[test]
    fn active_session_content_area_matches_responsive_management_split() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        model.terminal_size = (100, 24);

        let standard = active_session_content_area(&model).expect("active session content area");
        assert_eq!(standard, Rect::new(51, 1, 48, 21));

        model.terminal_size = (120, 24);
        let wide = active_session_content_area(&model).expect("active session content area");
        assert_eq!(wide, Rect::new(49, 1, 70, 21));
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
    fn render_model_text_status_bar_keeps_branch_context_and_branch_hints() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.sessions[0] = crate::model::SessionTab {
            id: "shell-0".to_string(),
            name: "Shell: feature/status-bar".to_string(),
            tab_type: SessionTabType::Shell,
            vt: crate::model::VtState::new(24, 80),
        };

        let rendered = render_model_text(&model, 220, 24);
        assert!(rendered.contains("branch: feature/status-bar"));
        assert!(rendered.contains("type: Shell"));
        assert!(rendered.contains("Enter:wizard"));
        assert!(rendered.contains("Esc:term"));
    }

    #[test]
    fn render_model_text_generic_management_hints_include_escape_to_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::GitView;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);
        assert!(rendered.contains("Enter:action"));
        assert!(rendered.contains("Esc:term"));
    }

    #[test]
    fn render_model_text_issues_detail_hints_show_escape_back_not_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;
        model.issues.detail_view = true;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Esc:back"));
        assert!(!rendered.contains("Esc:term"));
    }

    #[test]
    fn render_model_text_profiles_create_hints_show_escape_cancel_not_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;
        model.profiles.mode = screens::profiles::ProfileMode::Create;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Esc:cancel"));
        assert!(!rendered.contains("Esc:term"));
    }

    #[test]
    fn render_model_text_settings_list_hints_include_sub_tab_controls() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Settings;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Ctrl+←→:sub-tab"));
    }

    #[test]
    fn render_model_text_git_view_hints_omit_sub_tab_controls() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::GitView;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);

        assert!(!rendered.contains("Ctrl+←→:sub-tab"));
    }

    #[test]
    fn render_model_text_management_omits_standalone_header_banner() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.repo_path = PathBuf::from("/tmp/demo/project-repo");
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/banner".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/demo/project-repo-feature-banner")),
        }];

        let rendered = render_model_text(&model, 120, 16);

        assert!(
            !rendered.contains(" gwt | "),
            "management should rely on pane titles instead of a standalone header banner"
        );
    }

    #[test]
    fn render_model_text_management_top_row_uses_pane_title_chrome() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/top-row".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/demo/project-repo-feature-top-row")),
        }];

        let rendered = render_model_text(&model, 120, 16);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(
            first_line.contains("Branches"),
            "top row should start with pane title chrome once the standalone header is removed"
        );
    }

    #[test]
    fn render_model_text_non_branches_management_top_row_uses_pane_title_chrome() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Settings;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 120, 16);
        let mut lines = rendered.lines();
        let first_line = lines.next().unwrap_or_default();
        let second_line = lines.next().unwrap_or_default();

        assert!(
            !rendered.contains(" gwt | "),
            "non-Branches tabs should also omit the standalone management banner"
        );
        assert!(
            first_line.contains("Settings"),
            "non-Branches top row should keep the active pane title chrome visible"
        );
        assert!(
            second_line.contains("General"),
            "non-Branches content should start immediately below the pane title chrome"
        );
    }

    #[test]
    fn render_model_text_standard_width_branches_title_collapses_to_active_tab() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 80, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("Branches"));
        assert!(
            !first_line.contains("Specs"),
            "standard-width management title should collapse to the active tab instead of a truncated tab strip"
        );
    }

    #[test]
    fn render_model_text_standard_width_non_branches_title_collapses_to_active_tab() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 80, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("Issues"));
        assert!(
            !first_line.contains("Branches"),
            "standard-width non-Branches title should also collapse to the active tab"
        );
    }

    #[test]
    fn render_model_text_medium_width_management_title_still_collapses_to_active_tab() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 120, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("Issues"));
        assert!(
            !first_line.contains("Branches"),
            "when the full tab strip does not fit, medium-width panes should still collapse to the active tab"
        );
    }

    #[test]
    fn render_model_text_extra_wide_management_title_keeps_tab_strip() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("Branches"));
        assert!(first_line.contains("Specs"));
        assert!(first_line.contains("Issues"));
    }

    fn shell_tab(id: &str, name: &str) -> crate::model::SessionTab {
        crate::model::SessionTab {
            id: id.to_string(),
            name: name.to_string(),
            tab_type: SessionTabType::Shell,
            vt: crate::model::VtState::new(24, 80),
        }
    }

    #[test]
    fn render_model_text_standard_width_session_title_collapses_to_active_session() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/session-one"),
            shell_tab("shell-1", "Shell: feature/session-two"),
            shell_tab("shell-2", "Shell: feature/session-three"),
            shell_tab("shell-3", "Shell: feature/session-four"),
        ];
        model.active_session = 2;

        let rendered = render_model_text(&model, 80, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("session-three"));
        assert!(
            !first_line.contains("session-one"),
            "standard-width session title should collapse to the active session instead of truncating the strip"
        );
    }

    #[test]
    fn render_model_text_medium_width_session_title_still_collapses_when_strip_does_not_fit() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/session-one"),
            shell_tab("shell-1", "Shell: feature/session-two"),
            shell_tab("shell-2", "Shell: feature/session-three"),
            shell_tab("shell-3", "Shell: feature/session-four"),
        ];
        model.active_session = 1;

        let rendered = render_model_text(&model, 120, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("session-two"));
        assert!(
            !first_line.contains("session-one"),
            "medium-width session pane should still collapse when the full strip would truncate"
        );
    }

    #[test]
    fn render_model_text_extra_wide_session_title_keeps_full_strip() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/session-one"),
            shell_tab("shell-1", "Shell: feature/session-two"),
            shell_tab("shell-2", "Shell: feature/session-three"),
            shell_tab("shell-3", "Shell: feature/session-four"),
        ];
        model.active_session = 1;

        let rendered = render_model_text(&model, 220, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("session-one"));
        assert!(first_line.contains("session-two"));
        assert!(first_line.contains("session-three"));
        assert!(first_line.contains("session-four"));
    }

    #[test]
    fn render_model_text_terminal_hints_include_grouped_global_shortcuts() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Ctrl+G:b/i/s g c []/1-9 z ?"));
    }

    #[test]
    fn render_model_text_terminal_hints_include_focus_and_quit_shortcuts() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Tab:focus"));
        assert!(rendered.contains("^C×2"));
    }

    #[test]
    fn render_model_text_terminal_hints_remain_visible_at_standard_width() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions[0] = crate::model::SessionTab {
            id: "shell-0".to_string(),
            name: "Shell: feature/compact-footer".to_string(),
            tab_type: SessionTabType::Shell,
            vt: crate::model::VtState::new(24, 80),
        };

        let rendered = render_model_text(&model, 80, 24);

        assert!(rendered.contains("Ctrl+G:b/i/s g c []/1-9 z ?"));
        assert!(rendered.contains("Tab:focus"));
        assert!(rendered.contains("^C×2"));
    }

    #[test]
    fn render_model_text_branches_list_hints_remain_visible_at_standard_width() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 80, 24);

        assert!(rendered.contains("↑↓ mv  ←→ tab  ↵ wiz"));
        assert!(rendered.contains("mvf?"));
        assert!(rendered.contains("Esc→T"));
    }

    #[test]
    fn render_model_text_branch_detail_hints_remain_visible_at_standard_width() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/compact-detail".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from(
                "/tmp/demo/project-repo-feature-compact-detail",
            )),
        }];

        let rendered = render_model_text(&model, 80, 24);

        assert!(rendered.contains("←→ sec  ↵ act  S↵ sh"));
        assert!(rendered.contains("mvf?"));
        assert!(rendered.contains("Tab↔P  Esc←"));
    }

    #[test]
    fn render_model_text_generic_management_hints_remain_visible_at_standard_width() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 80, 24);

        assert!(rendered.contains("↑↓ sel  ←→ tab  ↵ act"));
        assert!(!rendered.contains("C-←→ sub"));
        assert!(rendered.contains("Tab pane"));
        assert!(rendered.contains("Esc term"));
    }

    #[test]
    fn render_model_text_branch_detail_title_includes_selected_branch_name() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/title-context".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-title-context")),
        }];

        let rendered = render_model_text(&model, 160, 24);
        let title_line = rendered
            .lines()
            .find(|line| line.contains("Overview") && line.contains("Sessions"))
            .expect("detail title line");
        assert!(
            title_line.contains("feature/title-context"),
            "detail title should keep the selected branch name visible"
        );
    }

    #[test]
    fn render_model_text_branch_detail_title_falls_back_without_selection() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches.clear();

        let rendered = render_model_text(&model, 160, 24);
        let title_line = rendered
            .lines()
            .find(|line| line.contains("Overview") && line.contains("Sessions"))
            .expect("detail title line");
        assert!(
            title_line.contains("No branch selected"),
            "detail title should fall back when no branch is selected"
        );
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

    fn write_spec_fixture(
        repo_path: &std::path::Path,
        spec_dir_name: &str,
        id: &str,
        title: &str,
        phase: &str,
        status: &str,
    ) {
        let spec_dir = repo_path.join("specs").join(spec_dir_name);
        fs::create_dir_all(&spec_dir).expect("create spec dir");
        fs::write(
            spec_dir.join("metadata.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": id,
                "title": title,
                "phase": phase,
                "status": status,
            }))
            .expect("serialize metadata"),
        )
        .expect("write metadata");
        fs::write(spec_dir.join("spec.md"), format!("# {title}\n\nbody\n")).expect("write spec");
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
    fn update_toggle_layer_shows_management_without_stealing_terminal_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        update(&mut model, Message::ToggleLayer);

        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn update_toggle_layer_hides_management_and_normalizes_tab_focus_to_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;

        update(&mut model, Message::ToggleLayer);

        assert_eq!(model.active_layer, ActiveLayer::Main);
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn update_toggle_layer_hides_management_and_normalizes_detail_focus_to_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::BranchDetail;

        update(&mut model, Message::ToggleLayer);

        assert_eq!(model.active_layer, ActiveLayer::Main);
        assert_eq!(model.active_focus, FocusPane::Terminal);
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
    fn load_initial_data_populates_specs_from_metadata() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        write_spec_fixture(
            dir.path(),
            "SPEC-9",
            "SPEC-9",
            "Later spec",
            "implementation",
            "in-progress",
        );
        write_spec_fixture(
            dir.path(),
            "SPEC-2",
            "SPEC-2",
            "Earlier spec",
            "done",
            "done",
        );

        let mut model = Model::new(dir.path().to_path_buf());
        load_initial_data(&mut model);

        assert_eq!(model.specs.spec_root.as_deref(), Some(dir.path()));
        assert_eq!(model.specs.specs.len(), 2);
        assert_eq!(model.specs.specs[0].id, "SPEC-2");
        assert_eq!(model.specs.specs[1].id, "SPEC-9");
        assert_eq!(model.specs.specs[0].title, "Earlier spec");
    }

    #[test]
    fn load_git_view_with_populates_divergence_and_pr_link_metadata() {
        let mut model = test_model();

        load_git_view_with(
            &mut model,
            |_repo_path| {
                Ok(vec![gwt_git::diff::FileEntry {
                    path: std::path::PathBuf::from("tracked.txt"),
                    status: gwt_git::diff::FileStatus::Staged,
                }])
            },
            |_repo_path| {
                Ok(vec![gwt_git::commit::CommitEntry {
                    hash: "abcdef1".into(),
                    subject: "Initial commit".into(),
                    author: "Alice".into(),
                    timestamp: "2026-04-04T00:00:00Z".into(),
                }])
            },
            |_repo_path| {
                Ok(vec![gwt_git::Branch {
                    name: "feature/live-meta".into(),
                    is_local: true,
                    is_remote: false,
                    is_head: true,
                    upstream: Some("origin/feature/live-meta".into()),
                    ahead: 2,
                    behind: 1,
                    last_commit_date: None,
                }])
            },
            |_repo_path| Ok(Some("https://example.com/pr/42".into())),
        );

        assert_eq!(
            model.git_view.divergence_summary.as_deref(),
            Some("Ahead 2 Behind 1")
        );
        assert_eq!(
            model.git_view.pr_link.as_deref(),
            Some("https://example.com/pr/42")
        );
    }

    #[test]
    fn load_git_view_with_omits_divergence_without_upstream() {
        let mut model = test_model();

        load_git_view_with(
            &mut model,
            |_repo_path| Ok(Vec::new()),
            |_repo_path| Ok(Vec::new()),
            |_repo_path| {
                Ok(vec![gwt_git::Branch {
                    name: "feature/no-upstream".into(),
                    is_local: true,
                    is_remote: false,
                    is_head: true,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                    last_commit_date: None,
                }])
            },
            |_repo_path| Ok(None),
        );

        assert!(model.git_view.divergence_summary.is_none());
        assert!(model.git_view.pr_link.is_none());
    }

    #[test]
    fn switch_management_tab_pr_dashboard_loads_prs_without_stealing_terminal_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        switch_management_tab_with(
            &mut model,
            ManagementTab::PrDashboard,
            |_repo_path| {
                Ok(vec![gwt_git::PrStatus {
                    number: 42,
                    title: "Wire PR dashboard".into(),
                    state: GitPrState::Open,
                    url: "https://example.com/pr/42".into(),
                    ci_status: "SUCCESS".into(),
                    mergeable: "MERGEABLE".into(),
                    review_status: "APPROVED".into(),
                }])
            },
            |_repo_path, _number| panic!("detail loader should not run for list-only focus"),
        );

        assert_eq!(model.management_tab, ManagementTab::PrDashboard);
        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::Terminal);
        assert_eq!(model.pr_dashboard.prs.len(), 1);
        assert_eq!(model.pr_dashboard.prs[0].number, 42);
        assert_eq!(model.pr_dashboard.prs[0].title, "Wire PR dashboard");
        assert_eq!(model.pr_dashboard.prs[0].ci_status, "success");
        assert_eq!(model.pr_dashboard.prs[0].review_status, "approved");
        assert!(model.pr_dashboard.prs[0].mergeable);
    }

    #[test]
    fn switch_management_tab_from_tab_content_lands_on_tab_content() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;

        switch_management_tab_with(
            &mut model,
            ManagementTab::Settings,
            |_repo_path| panic!("PR loader should not run for Settings"),
            |_repo_path, _number| panic!("detail loader should not run for Settings"),
        );

        assert_eq!(model.management_tab, ManagementTab::Settings);
        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn switch_management_tab_from_branch_detail_lands_on_tab_content() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::BranchDetail;

        switch_management_tab_with(
            &mut model,
            ManagementTab::Issues,
            |_repo_path| panic!("PR loader should not run for Issues"),
            |_repo_path, _number| panic!("detail loader should not run for Issues"),
        );

        assert_eq!(model.management_tab, ManagementTab::Issues);
        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn switch_management_tab_pr_dashboard_reloads_detail_when_open() {
        let mut model = test_model();
        screens::pr_dashboard::update(
            &mut model.pr_dashboard,
            screens::pr_dashboard::PrDashboardMessage::SetPrs(vec![
                screens::pr_dashboard::PrItem {
                    number: 42,
                    title: "Existing detail".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "pending".into(),
                    mergeable: true,
                    review_status: "review_required".into(),
                },
            ]),
        );
        model.pr_dashboard.detail_view = true;

        switch_management_tab_with(
            &mut model,
            ManagementTab::PrDashboard,
            |_repo_path| {
                Ok(vec![gwt_git::PrStatus {
                    number: 42,
                    title: "Existing detail".into(),
                    state: GitPrState::Open,
                    url: "https://example.com/pr/42".into(),
                    ci_status: "SUCCESS".into(),
                    mergeable: "MERGEABLE".into(),
                    review_status: "APPROVED".into(),
                }])
            },
            |_repo_path, number| {
                assert_eq!(number, 42);
                Ok(screens::pr_dashboard::PrDetailReport {
                    summary: "live detail".into(),
                    ci_status: "passing".into(),
                    merge_status: "ready".into(),
                    review_status: "approved".into(),
                    checks: vec!["lint: success".into()],
                })
            },
        );

        let detail = model
            .pr_dashboard
            .detail_report
            .as_ref()
            .expect("detail report refreshed on tab focus");
        assert_eq!(detail.summary, "live detail");
    }

    #[test]
    fn refresh_pr_dashboard_with_reloads_prs() {
        let mut model = test_model();
        model.management_tab = ManagementTab::PrDashboard;

        load_pr_dashboard_with(&mut model, |_repo_path| {
            Ok(vec![gwt_git::PrStatus {
                number: 7,
                title: "Initial".into(),
                state: GitPrState::Open,
                url: "https://example.com/pr/7".into(),
                ci_status: "PENDING".into(),
                mergeable: "UNKNOWN".into(),
                review_status: "REVIEW_REQUIRED".into(),
            }])
        });
        assert_eq!(model.pr_dashboard.prs.len(), 1);
        assert_eq!(model.pr_dashboard.prs[0].number, 7);

        refresh_pr_dashboard_with(
            &mut model,
            |_repo_path| {
                Ok(vec![gwt_git::PrStatus {
                    number: 8,
                    title: "Updated".into(),
                    state: GitPrState::Merged,
                    url: "https://example.com/pr/8".into(),
                    ci_status: "FAILURE".into(),
                    mergeable: "CONFLICTING".into(),
                    review_status: "CHANGES_REQUESTED".into(),
                }])
            },
            |_repo_path, _number| {
                Ok(screens::pr_dashboard::PrDetailReport {
                    summary: "CI failing".into(),
                    ci_status: "failing".into(),
                    merge_status: "conflicts".into(),
                    review_status: "changes_requested".into(),
                    checks: vec!["lint: failure".into()],
                })
            },
        );

        assert_eq!(model.pr_dashboard.prs.len(), 1);
        assert_eq!(model.pr_dashboard.prs[0].number, 8);
        assert_eq!(model.pr_dashboard.prs[0].title, "Updated");
        assert_eq!(model.pr_dashboard.prs[0].ci_status, "failure");
        assert_eq!(model.pr_dashboard.prs[0].review_status, "changes_requested");
        assert!(!model.pr_dashboard.prs[0].mergeable);
        assert_eq!(
            model.pr_dashboard.prs[0].state,
            screens::pr_dashboard::PrState::Merged
        );
    }

    #[test]
    fn refresh_pr_dashboard_with_in_detail_view_updates_detail_report() {
        let mut model = test_model();
        model.management_tab = ManagementTab::PrDashboard;
        screens::pr_dashboard::update(
            &mut model.pr_dashboard,
            screens::pr_dashboard::PrDashboardMessage::SetPrs(vec![
                screens::pr_dashboard::PrItem {
                    number: 8,
                    title: "Updated".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "success".into(),
                    mergeable: true,
                    review_status: "approved".into(),
                },
            ]),
        );
        model.pr_dashboard.detail_view = true;

        refresh_pr_dashboard_with(
            &mut model,
            |_repo_path| {
                Ok(vec![gwt_git::PrStatus {
                    number: 8,
                    title: "Updated".into(),
                    state: GitPrState::Open,
                    url: "https://example.com/pr/8".into(),
                    ci_status: "SUCCESS".into(),
                    mergeable: "MERGEABLE".into(),
                    review_status: "APPROVED".into(),
                }])
            },
            |_repo_path, number| {
                assert_eq!(number, 8);
                Ok(screens::pr_dashboard::PrDetailReport {
                    summary: "CI passing".into(),
                    ci_status: "passing".into(),
                    merge_status: "ready".into(),
                    review_status: "approved".into(),
                    checks: vec!["test: success".into()],
                })
            },
        );

        let detail = model
            .pr_dashboard
            .detail_report
            .as_ref()
            .expect("detail report refreshed");
        assert_eq!(detail.summary, "CI passing");
        assert_eq!(detail.checks, vec!["test: success"]);
    }

    #[test]
    fn parse_pr_dashboard_detail_report_json_extracts_checks_and_statuses() {
        let json = r#"{
            "title": "Add dashboard detail",
            "state": "OPEN",
            "mergeable": "CONFLICTING",
            "reviewDecision": "CHANGES_REQUESTED",
            "statusCheckRollup": [
                {"name": "lint", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"name": "test", "status": "COMPLETED", "conclusion": "FAILURE"}
            ]
        }"#;

        let detail = parse_pr_dashboard_detail_report_json(json).expect("detail report parsed");
        assert_eq!(detail.ci_status, "failing");
        assert_eq!(detail.merge_status, "conflicts");
        assert_eq!(detail.review_status, "changes_requested");
        assert_eq!(
            detail.checks,
            vec!["lint: success".to_string(), "test: failure".to_string()]
        );
    }

    #[test]
    fn route_key_to_management_pr_dashboard_enter_loads_detail_report() {
        let mut model = test_model();
        model.management_tab = ManagementTab::PrDashboard;
        screens::pr_dashboard::update(
            &mut model.pr_dashboard,
            screens::pr_dashboard::PrDashboardMessage::SetPrs(vec![
                screens::pr_dashboard::PrItem {
                    number: 42,
                    title: "Wire detail report".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "success".into(),
                    mergeable: true,
                    review_status: "approved".into(),
                },
            ]),
        );

        route_key_to_management_pr_dashboard_with(
            &mut model,
            key(KeyCode::Enter, KeyModifiers::NONE),
            |_repo_path, number| {
                assert_eq!(number, 42);
                Ok(screens::pr_dashboard::PrDetailReport {
                    summary: "CI passing".into(),
                    ci_status: "passing".into(),
                    merge_status: "ready".into(),
                    review_status: "approved".into(),
                    checks: vec!["lint: success".into()],
                })
            },
        );

        assert!(model.pr_dashboard.detail_view);
        let detail = model
            .pr_dashboard
            .detail_report
            .as_ref()
            .expect("detail report loaded");
        assert_eq!(detail.summary, "CI passing");
        assert_eq!(detail.checks, vec!["lint: success"]);
    }

    #[test]
    fn route_key_to_management_specs_enter_opens_detail_and_escape_returns_list() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-5".into(),
            title: "Local SPEC Management".into(),
            phase: "implementation".into(),
            status: "in-progress".into(),
        }];

        route_key_to_management(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(model.specs.detail_view);

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!model.specs.detail_view);
    }

    #[test]
    fn route_key_to_management_specs_left_right_cycle_sections_without_switching_tabs() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-5".into(),
            title: "Local SPEC Management".into(),
            phase: "implementation".into(),
            status: "in-progress".into(),
        }];
        model.specs.detail_view = true;

        route_key_to_management(&mut model, key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(model.management_tab, ManagementTab::Specs);
        assert_eq!(model.specs.detail_section, 1);

        route_key_to_management(&mut model, key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(model.management_tab, ManagementTab::Specs);
        assert_eq!(model.specs.detail_section, 0);
    }

    #[test]
    fn route_key_to_management_specs_shift_enter_opens_prefilled_wizard() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.spec_root = Some(model.repo_path.clone());
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-5".into(),
            title: "Local SPEC Management".into(),
            phase: "implementation".into(),
            status: "in-progress".into(),
        }];
        model.specs.detail_view = true;
        std::fs::create_dir_all(model.repo_path.join("specs/SPEC-5")).expect("create spec dir");
        std::fs::write(
            model.repo_path.join("specs/SPEC-5/spec.md"),
            "# SPEC-5\n\nLocal SPEC detail body\n",
        )
        .expect("write spec body");

        route_key_to_management(&mut model, key(KeyCode::Enter, KeyModifiers::SHIFT));

        let wizard = model
            .wizard
            .as_ref()
            .expect("wizard opened from spec detail");
        let context = wizard.spec_context.as_ref().expect("spec context captured");
        assert_eq!(context.spec_id, "SPEC-5");
        assert_eq!(context.title, "Local SPEC Management");
        assert_eq!(context.spec_body, "# SPEC-5\n\nLocal SPEC detail body\n");
        assert_eq!(wizard.branch_name, "feature/spec-5-local-spec-management");
    }

    #[test]
    fn route_key_to_management_specs_e_starts_phase_edit_from_detail() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-5".into(),
            title: "Local SPEC Management".into(),
            phase: "implementation".into(),
            status: "in-progress".into(),
        }];
        model.specs.detail_view = true;

        route_key_to_management(&mut model, key(KeyCode::Char('e'), KeyModifiers::NONE));

        assert!(model.specs.editing);
        assert_eq!(model.specs.edit_field, "implementation");
    }

    #[test]
    fn route_key_to_management_specs_ctrl_e_starts_section_edit_from_detail() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.spec_root = Some(model.repo_path.clone());
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-5002".into(),
            title: "Local SPEC Management".into(),
            phase: "implementation".into(),
            status: "in-progress".into(),
        }];
        model.specs.detail_view = true;
        std::fs::create_dir_all(model.repo_path.join("specs/SPEC-5002")).expect("create spec dir");
        std::fs::write(
            model.repo_path.join("specs/SPEC-5002/spec.md"),
            "# SPEC-5002\n\nSection edit body\n",
        )
        .expect("write spec body");

        route_key_to_management(&mut model, key(KeyCode::Char('e'), KeyModifiers::CONTROL));

        assert!(model.specs.detail_editing);
        assert!(model.specs.detail_edit_buffer.contains("Section edit body"));
    }

    #[test]
    fn route_key_to_management_specs_shift_e_starts_raw_file_edit_from_detail() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.spec_root = Some(model.repo_path.clone());
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-5003".into(),
            title: "Local SPEC Management".into(),
            phase: "implementation".into(),
            status: "in-progress".into(),
        }];
        model.specs.detail_view = true;
        std::fs::create_dir_all(model.repo_path.join("specs/SPEC-5003")).expect("create spec dir");
        std::fs::write(
            model.repo_path.join("specs/SPEC-5003/spec.md"),
            "# SPEC-5003\n\n## Background\n\nfull file body\n",
        )
        .expect("write spec body");

        route_key_to_management(&mut model, key(KeyCode::Char('E'), KeyModifiers::SHIFT));

        assert!(model.specs.detail_editing);
        assert!(model.specs.detail_edit_buffer.contains("# SPEC-5003"));
        assert!(model.specs.detail_edit_buffer.contains("## Background"));
    }

    #[test]
    fn route_key_to_management_specs_s_starts_status_edit_from_detail() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-5".into(),
            title: "Local SPEC Management".into(),
            phase: "implementation".into(),
            status: "in-progress".into(),
        }];
        model.specs.detail_view = true;

        route_key_to_management(&mut model, key(KeyCode::Char('s'), KeyModifiers::NONE));

        assert!(model.specs.editing);
        assert_eq!(model.specs.edit_field, "in-progress");
    }

    #[test]
    fn route_key_to_management_specs_search_to_detail_then_e_starts_phase_edit() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-5".into(),
            title: "Local SPEC Management".into(),
            phase: "implementation".into(),
            status: "in-progress".into(),
        }];
        model.specs.search_active = true;
        model.specs.search_query = "spec".into();

        route_key_to_management(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Char('e'), KeyModifiers::NONE));

        assert!(model.specs.detail_view);
        assert!(!model.specs.search_active);
        assert!(model.specs.editing);
        assert_eq!(model.specs.edit_field, "implementation");
    }

    #[test]
    fn route_key_to_management_specs_down_cycles_phase_selection_menu() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.specs = vec![screens::specs::SpecItem {
            id: "SPEC-5".into(),
            title: "Local SPEC Management".into(),
            phase: "implementation".into(),
            status: "in-progress".into(),
        }];
        model.specs.detail_view = true;

        route_key_to_management(&mut model, key(KeyCode::Char('e'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Down, KeyModifiers::NONE));

        assert!(model.specs.editing);
        assert_eq!(model.specs.selected, 0);
        assert_eq!(model.specs.edit_field, "done");
    }

    #[test]
    fn route_key_to_management_pr_dashboard_move_in_detail_view_reloads_selected_pr_detail() {
        let mut model = test_model();
        model.management_tab = ManagementTab::PrDashboard;
        screens::pr_dashboard::update(
            &mut model.pr_dashboard,
            screens::pr_dashboard::PrDashboardMessage::SetPrs(vec![
                screens::pr_dashboard::PrItem {
                    number: 41,
                    title: "First".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "pending".into(),
                    mergeable: true,
                    review_status: "review_required".into(),
                },
                screens::pr_dashboard::PrItem {
                    number: 42,
                    title: "Second".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "success".into(),
                    mergeable: true,
                    review_status: "approved".into(),
                },
            ]),
        );
        model.pr_dashboard.detail_view = true;

        route_key_to_management_pr_dashboard_with(
            &mut model,
            key(KeyCode::Down, KeyModifiers::NONE),
            |_repo_path, number| {
                assert_eq!(number, 42);
                Ok(screens::pr_dashboard::PrDetailReport {
                    summary: "second detail".into(),
                    ci_status: "passing".into(),
                    merge_status: "ready".into(),
                    review_status: "approved".into(),
                    checks: vec!["test: success".into()],
                })
            },
        );

        assert_eq!(model.pr_dashboard.selected, 1);
        let detail = model
            .pr_dashboard
            .detail_report
            .as_ref()
            .expect("detail report reloaded for moved selection");
        assert_eq!(detail.summary, "second detail");
    }

    #[test]
    fn route_key_to_management_pr_dashboard_esc_closes_detail_view_and_preserves_selection() {
        let mut model = test_model();
        model.management_tab = ManagementTab::PrDashboard;
        screens::pr_dashboard::update(
            &mut model.pr_dashboard,
            screens::pr_dashboard::PrDashboardMessage::SetPrs(vec![
                screens::pr_dashboard::PrItem {
                    number: 41,
                    title: "First".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "pending".into(),
                    mergeable: true,
                    review_status: "review_required".into(),
                },
                screens::pr_dashboard::PrItem {
                    number: 42,
                    title: "Second".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "success".into(),
                    mergeable: true,
                    review_status: "approved".into(),
                },
            ]),
        );
        model.pr_dashboard.selected = 1;
        model.pr_dashboard.detail_view = true;
        model.pr_dashboard.detail_report = Some(screens::pr_dashboard::PrDetailReport {
            summary: "loaded".into(),
            ci_status: "passing".into(),
            merge_status: "ready".into(),
            review_status: "approved".into(),
            checks: vec!["lint: success".into()],
        });

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert!(!model.pr_dashboard.detail_view);
        assert_eq!(model.pr_dashboard.selected, 1);
        assert_eq!(
            model.pr_dashboard.selected_pr().map(|pr| pr.number),
            Some(42)
        );
        assert!(model.pr_dashboard.detail_report.is_none());
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
            Some(screens::wizard::SpecContext::new(
                "SPEC-42",
                "My Feature",
                "# SPEC-42\n\nBody\n",
            )),
            detected,
            &cache,
        );

        assert_eq!(wizard.branch_name, "feature/spec-42-my-feature");
        let ctx = wizard.spec_context.as_ref().unwrap();
        assert_eq!(ctx.spec_id, "SPEC-42");
        assert_eq!(ctx.title, "My Feature");
        assert_eq!(ctx.spec_body, "# SPEC-42\n\nBody\n");
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
            Some(screens::wizard::SpecContext::new(
                "SPEC-42",
                "My Feature",
                "",
            )),
            vec![],
            &cache,
        );

        assert_eq!(wizard.step, screens::wizard::WizardStep::BranchTypeSelect);
        assert_eq!(wizard.branch_name, "feature/spec-42-my-feature");
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
    fn configure_existing_branch_wizard_with_sessions_loads_newest_entry_per_agent() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let cache = VersionCache::new();
        let detected = vec![
            detected_agent(AgentId::ClaudeCode, Some("1.0.55")),
            detected_agent(AgentId::Codex, Some("0.5.1")),
        ];
        let repo_path = PathBuf::from("/tmp/repo");
        let branch = "feature/test";
        let now = Utc::now();

        persist_agent_session(
            dir.path(),
            repo_path.to_str().unwrap(),
            branch,
            AgentId::Codex,
            now - Duration::minutes(10),
            Some("gpt-5.2-codex"),
            Some("medium"),
            Some("0.5.0"),
            None,
            false,
        );
        persist_agent_session(
            dir.path(),
            repo_path.to_str().unwrap(),
            branch,
            AgentId::Codex,
            now - Duration::minutes(1),
            Some("gpt-5.3-codex"),
            Some("high"),
            Some("latest"),
            Some("sess-new"),
            true,
        );
        persist_agent_session(
            dir.path(),
            repo_path.to_str().unwrap(),
            branch,
            AgentId::ClaudeCode,
            now - Duration::minutes(5),
            Some("sonnet"),
            None,
            Some("1.0.54"),
            None,
            false,
        );
        persist_agent_session(
            dir.path(),
            repo_path.to_str().unwrap(),
            "feature/other",
            AgentId::Gemini,
            now - Duration::minutes(2),
            Some("gemini-2.5-pro"),
            None,
            Some("latest"),
            Some("sess-other"),
            false,
        );

        let (mut wizard, _) = prepare_wizard_startup(None, detected, &cache);
        configure_existing_branch_wizard_with_sessions(&mut wizard, &repo_path, dir.path(), branch);

        assert_eq!(wizard.step, screens::wizard::WizardStep::QuickStart);
        assert!(wizard.has_quick_start);
        assert_eq!(wizard.branch_name, branch);
        assert_eq!(wizard.quick_start_entries.len(), 2);
        assert_eq!(wizard.quick_start_entries[0].agent_id, "codex");
        assert_eq!(
            wizard.quick_start_entries[0].model.as_deref(),
            Some("gpt-5.3-codex")
        );
        assert_eq!(
            wizard.quick_start_entries[0].resume_session_id.as_deref(),
            Some("sess-new")
        );
        assert_eq!(wizard.quick_start_entries[1].agent_id, "claude");
    }

    #[test]
    fn branch_session_summaries_with_filters_to_selected_branch_and_marks_active() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let repo_path = dir.path().join("repo");
        let selected_worktree = repo_path.join("wt-feature-test");
        let other_worktree = repo_path.join("wt-feature-other");
        fs::create_dir_all(&selected_worktree).expect("create selected worktree");
        fs::create_dir_all(&other_worktree).expect("create other worktree");

        let mut model = Model::new(repo_path.clone());
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(selected_worktree.clone()),
        }];

        let mut matching = AgentSession::new(&selected_worktree, "feature/test", AgentId::Codex);
        matching.model = Some("gpt-5.3-codex".to_string());
        matching.reasoning_level = Some("high".to_string());
        matching.display_name = "Codex".to_string();
        matching.save(dir.path()).expect("persist matching session");

        let mut stale_branch =
            AgentSession::new(&selected_worktree, "feature/other", AgentId::ClaudeCode);
        stale_branch.display_name = "Claude Code".to_string();
        stale_branch.save(dir.path()).expect("persist stale branch");

        let mut stale_worktree =
            AgentSession::new(&other_worktree, "feature/test", AgentId::Gemini);
        stale_worktree.display_name = "Gemini CLI".to_string();
        stale_worktree
            .save(dir.path())
            .expect("persist stale worktree");

        model.sessions = vec![
            crate::model::SessionTab {
                id: matching.id.clone(),
                name: "Codex".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "codex".to_string(),
                    color: crate::model::AgentColor::Blue,
                },
                vt: crate::model::VtState::new(24, 80),
            },
            crate::model::SessionTab {
                id: stale_branch.id.clone(),
                name: "Claude Code".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "claude".to_string(),
                    color: crate::model::AgentColor::Green,
                },
                vt: crate::model::VtState::new(24, 80),
            },
            crate::model::SessionTab {
                id: stale_worktree.id.clone(),
                name: "Gemini CLI".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "gemini".to_string(),
                    color: crate::model::AgentColor::Cyan,
                },
                vt: crate::model::VtState::new(24, 80),
            },
            crate::model::SessionTab {
                id: "shell-branch".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
            },
        ];
        model.active_session = 0;

        let summaries = branch_session_summaries_with(&model, dir.path());

        assert_eq!(
            summaries,
            vec![
                screens::branches::DetailSessionSummary {
                    kind: "Agent",
                    name: "Codex".to_string(),
                    detail: Some("gpt-5.3-codex · high".to_string()),
                    active: true,
                },
                screens::branches::DetailSessionSummary {
                    kind: "Shell",
                    name: "Shell: feature/test".to_string(),
                    detail: None,
                    active: false,
                },
            ]
        );
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
    fn build_launch_config_from_wizard_uses_resume_session_id_for_quick_start_resume() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "sonnet".to_string(),
            branch_name: "feature/spec-42".to_string(),
            mode: "resume".to_string(),
            resume_session_id: Some("sess-123".to_string()),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(config.args.contains(&"--resume".to_string()));
        assert!(config.args.contains(&"sess-123".to_string()));
    }

    #[test]
    fn build_launch_config_from_wizard_falls_back_to_continue_without_resume_session_id() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "sonnet".to_string(),
            branch_name: "feature/spec-42".to_string(),
            mode: "resume".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(config.args.contains(&"--continue".to_string()));
        assert!(!config.args.contains(&"--resume".to_string()));
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
    fn materialize_pending_launch_with_persists_quick_start_restore_fields() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut model = test_model();
        let wizard = screens::wizard::WizardState {
            agent_id: "codex".to_string(),
            model: "gpt-5.3-codex".to_string(),
            reasoning: "high".to_string(),
            version: "latest".to_string(),
            branch_name: "feature/spec-42".to_string(),
            mode: "resume".to_string(),
            resume_session_id: Some("sess-abc".to_string()),
            skip_perms: true,
            ..Default::default()
        };
        model.pending_launch_config = Some(build_launch_config_from_wizard(&wizard));

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let entry = fs::read_dir(dir.path())
            .expect("read sessions dir")
            .next()
            .expect("session entry")
            .expect("dir entry")
            .path();
        let persisted = AgentSession::load(&entry).expect("load persisted session");
        assert_eq!(persisted.reasoning_level.as_deref(), Some("high"));
        assert!(persisted.skip_permissions);
        assert_eq!(persisted.agent_session_id.as_deref(), Some("sess-abc"));
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
        wizard.spec_context = Some(screens::wizard::SpecContext::new(
            "SPEC-42",
            "My Feature",
            "# SPEC-42\n\nDetailed implementation notes",
        ));

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
        wizard.spec_context = Some(screens::wizard::SpecContext::new(
            "SPEC-7",
            "Voice settings",
            "# Voice settings\n\nCapture the selected microphone and language.\n",
        ));

        let context = wizard_branch_suggestion_context(&wizard);

        assert!(context.contains("SPEC: SPEC-7 - Voice settings"));
        assert!(context.contains("SPEC body:"));
        assert!(context.contains("Capture the selected microphone and language."));
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
    fn handle_voice_start_recording_with_runtime_error_sets_error_state() {
        let mut model = test_model();

        handle_voice_message_with_runtime(
            &mut model,
            VoiceInputMessage::StartRecording,
            true,
            &mut FakeVoiceRuntime::start_error("backend missing"),
        );

        assert_eq!(model.voice.status, crate::input::voice::VoiceStatus::Error);
        assert_eq!(
            model.voice.error_message.as_deref(),
            Some("backend missing")
        );
    }

    #[test]
    fn handle_voice_start_recording_toggle_stops_and_injects_transcript() {
        let mut model = test_model();
        let mut runtime = FakeVoiceRuntime::success("git status");

        handle_voice_message_with_runtime(
            &mut model,
            VoiceInputMessage::StartRecording,
            true,
            &mut runtime,
        );
        assert_eq!(
            model.voice.status,
            crate::input::voice::VoiceStatus::Recording
        );

        handle_voice_message_with_runtime(
            &mut model,
            VoiceInputMessage::StartRecording,
            true,
            &mut runtime,
        );

        assert_eq!(model.voice.status, crate::input::voice::VoiceStatus::Idle);
        assert_eq!(model.voice.buffer, "git status");
        let pending = model
            .pending_pty_inputs
            .pop_front()
            .expect("pty input queued");
        assert_eq!(pending.bytes, b"git status".to_vec());
    }

    #[test]
    fn handle_voice_stop_recording_with_runtime_error_sets_error_state() {
        let mut model = test_model();
        let mut runtime = FakeVoiceRuntime::stop_error("transcription failed");

        handle_voice_message_with_runtime(
            &mut model,
            VoiceInputMessage::StartRecording,
            true,
            &mut runtime,
        );
        assert_eq!(
            model.voice.status,
            crate::input::voice::VoiceStatus::Recording
        );

        handle_voice_message_with_runtime(
            &mut model,
            VoiceInputMessage::StopRecording,
            true,
            &mut runtime,
        );

        assert_eq!(model.voice.status, crate::input::voice::VoiceStatus::Error);
        assert_eq!(
            model.voice.error_message.as_deref(),
            Some("transcription failed")
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
    fn route_key_to_management_logs_esc_closes_detail_view_and_preserves_selection() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;
        model.logs.entries = vec![
            Notification::new(Severity::Info, "core", "first"),
            Notification::new(Severity::Warn, "core", "second"),
        ];
        model.logs.selected = 1;
        model.logs.detail_view = true;

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert!(!model.logs.detail_view);
        assert_eq!(model.logs.selected, 1);
        assert_eq!(
            model
                .logs
                .selected_entry()
                .map(|entry| entry.message.as_str()),
            Some("second")
        );
    }

    #[test]
    fn route_key_to_management_logs_filter_controls_still_work_after_detail_close_support() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;

        route_key_to_management(&mut model, key(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(
            model.logs.filter_level,
            screens::logs::FilterLevel::ErrorOnly
        );

        route_key_to_management(&mut model, key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(model.logs.filter_level, screens::logs::FilterLevel::DebugUp);
    }

    #[test]
    fn route_key_to_management_logs_esc_without_warn_returns_terminal_focus() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;
        model.active_focus = FocusPane::TabContent;
        model.logs.entries = vec![
            Notification::new(Severity::Info, "core", "first"),
            Notification::new(Severity::Warn, "core", "second"),
        ];
        model.logs.selected = 1;

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::Terminal);
        assert_eq!(model.management_tab, ManagementTab::Logs);
        assert_eq!(model.logs.selected, 1);
    }

    #[test]
    fn route_key_to_management_logs_esc_with_warn_still_dismisses_warning() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;
        model.active_focus = FocusPane::TabContent;
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert!(model.current_notification.is_none());
    }

    #[test]
    fn route_key_to_management_profiles_esc_without_warn_returns_terminal_focus_in_list_mode() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::Terminal);
        assert_eq!(model.profiles.mode, screens::profiles::ProfileMode::List);
    }

    #[test]
    fn route_key_to_management_profiles_esc_with_warn_still_dismisses_warning_in_list_mode() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert!(model.current_notification.is_none());
        assert_eq!(model.profiles.mode, screens::profiles::ProfileMode::List);
    }

    #[test]
    fn route_key_to_management_profiles_esc_in_create_mode_still_cancels_form() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;
        model.profiles.mode = screens::profiles::ProfileMode::Create;
        model.profiles.input_name = "demo".into();

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert_eq!(model.profiles.mode, screens::profiles::ProfileMode::List);
        assert!(model.profiles.input_name.is_empty());
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
    fn route_key_to_management_issues_esc_closes_detail_view_and_preserves_selection() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Issues;
        model.issues.issues = vec![
            screens::issues::IssueItem {
                number: 1,
                title: "First".into(),
                state: "open".into(),
                labels: vec!["ux".into()],
                body: "First body".into(),
            },
            screens::issues::IssueItem {
                number: 2,
                title: "Second".into(),
                state: "open".into(),
                labels: vec!["bug".into()],
                body: "Second body".into(),
            },
        ];
        model.issues.selected = 1;
        model.issues.detail_view = true;

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert!(!model.issues.detail_view);
        assert_eq!(model.issues.selected, 1);
        assert_eq!(
            model.issues.selected_issue().map(|issue| issue.number),
            Some(2)
        );
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
    fn route_key_to_branch_detail_sessions_moves_selection() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-test")),
        }];
        model.branches.detail_section = 3;

        model.sessions = vec![
            crate::model::SessionTab {
                id: "shell-0".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
            },
            crate::model::SessionTab {
                id: "shell-1".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
            },
        ];

        route_key_to_branch_detail(&mut model, key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(model.branches.detail_session_selected, 1);

        route_key_to_branch_detail(&mut model, key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(model.branches.detail_session_selected, 0);
    }

    #[test]
    fn route_key_to_branch_detail_sessions_enter_focuses_selected_session() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-test")),
        }];
        model.branches.detail_section = 3;
        model.active_focus = FocusPane::BranchDetail;

        model.sessions = vec![
            crate::model::SessionTab {
                id: "shell-0".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
            },
            crate::model::SessionTab {
                id: "shell-1".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
            },
        ];
        model.branches.detail_session_selected = 1;

        route_key_to_branch_detail(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(model.active_session, 1);
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn route_key_to_branch_detail_sessions_enter_clamps_stale_selection() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-test")),
        }];
        model.branches.detail_section = 3;
        model.active_focus = FocusPane::BranchDetail;

        model.sessions = vec![
            crate::model::SessionTab {
                id: "shell-0".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
            },
            crate::model::SessionTab {
                id: "shell-1".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
            },
        ];
        model.branches.detail_session_selected = 99;

        route_key_to_branch_detail(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(model.branches.detail_session_selected, 1);
        assert_eq!(model.active_session, 1);
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn route_key_to_branch_detail_shift_enter_opens_shell_for_selected_branch() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/direct-actions".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-direct-actions")),
        }];
        model.branches.detail_section = 0;
        model.active_focus = FocusPane::BranchDetail;
        let initial_sessions = model.sessions.len();

        route_key_to_branch_detail(&mut model, key(KeyCode::Enter, KeyModifiers::SHIFT));

        assert_eq!(model.sessions.len(), initial_sessions + 1);
        assert_eq!(
            model.active_session, initial_sessions,
            "new shell session should become active"
        );
        assert_eq!(model.active_focus, FocusPane::Terminal);
        assert_eq!(
            model.sessions.last().map(|session| session.name.as_str()),
            Some("Shell: feature/direct-actions")
        );
    }

    #[test]
    fn route_key_to_branch_detail_ctrl_c_opens_delete_confirm() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/direct-actions".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-direct-actions")),
        }];
        model.branches.detail_section = 0;
        model.active_focus = FocusPane::BranchDetail;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(model.confirm.visible);
        assert!(
            model.confirm.message.contains("feature/direct-actions"),
            "delete confirmation should reference the selected branch"
        );
    }

    #[test]
    fn route_key_to_branch_detail_shift_enter_ignores_branches_without_worktree() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/no-worktree".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: None,
        }];
        model.branches.detail_section = 0;
        model.active_focus = FocusPane::BranchDetail;
        let initial_sessions = model.sessions.len();

        route_key_to_branch_detail(&mut model, key(KeyCode::Enter, KeyModifiers::SHIFT));

        assert_eq!(model.sessions.len(), initial_sessions);
        assert_eq!(model.active_focus, FocusPane::BranchDetail);
        assert!(!model.branches.pending_open_shell);
    }

    #[test]
    fn route_key_to_branch_detail_ctrl_c_ignores_branches_without_worktree() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/no-worktree".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: None,
        }];
        model.branches.detail_section = 0;
        model.active_focus = FocusPane::BranchDetail;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(!model.confirm.visible);
        assert!(!model.branches.pending_delete_worktree);
    }

    #[test]
    fn route_key_to_branch_detail_esc_returns_to_tab_content_focus() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/esc-back".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-esc-back")),
        }];
        model.branches.selected = 0;
        model.branches.detail_section = 2;
        model.active_focus = FocusPane::BranchDetail;

        route_key_to_branch_detail(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn route_key_to_branch_detail_esc_preserves_detail_context() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/esc-back".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-esc-back")),
        }];
        model.branches.selected = 0;
        model.branches.detail_section = 3;
        model.branches.detail_session_selected = 4;
        model.active_focus = FocusPane::BranchDetail;

        route_key_to_branch_detail(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.branches.selected, 0);
        assert_eq!(model.branches.detail_section, 3);
        assert_eq!(model.branches.detail_session_selected, 4);
        assert_eq!(
            model
                .branches
                .selected_branch()
                .map(|branch| branch.name.as_str()),
            Some("feature/esc-back")
        );
    }

    #[test]
    fn route_key_to_branch_detail_esc_with_warn_preserves_notification_and_returns_to_list() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/esc-back".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-esc-back")),
        }];
        model.branches.selected = 0;
        model.branches.detail_section = 3;
        model.branches.detail_session_selected = 4;
        model.active_focus = FocusPane::BranchDetail;
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        route_key_to_branch_detail(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert!(model.current_notification.is_some());
        assert_eq!(model.branches.selected, 0);
        assert_eq!(model.branches.detail_section, 3);
        assert_eq!(model.branches.detail_session_selected, 4);
    }

    #[test]
    fn route_key_to_branch_detail_esc_with_warn_allows_second_escape_to_dismiss_from_list() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/esc-back".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-esc-back")),
        }];
        model.active_focus = FocusPane::BranchDetail;
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        route_key_to_branch_detail(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert!(model.current_notification.is_none());
    }

    #[test]
    fn route_key_to_branch_detail_m_toggles_view_mode() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/view-mode".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-view-mode")),
        }];
        model.active_focus = FocusPane::BranchDetail;
        model.branches.detail_section = 0;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('m'), KeyModifiers::NONE));

        assert_eq!(model.branches.view_mode, screens::branches::ViewMode::Local);
        assert_eq!(model.active_focus, FocusPane::BranchDetail);
    }

    #[test]
    fn route_key_to_branch_detail_v_switches_to_git_view() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.detail_section = 2;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('v'), KeyModifiers::NONE));

        assert_eq!(model.management_tab, ManagementTab::GitView);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn route_key_to_branch_detail_f_starts_search_and_returns_to_list_focus() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.detail_section = 1;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('f'), KeyModifiers::NONE));

        assert!(model.branches.search_active);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn route_key_to_branch_detail_h_toggles_help_overlay() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('h'), KeyModifiers::NONE));

        assert!(model.help_visible);
    }

    #[test]
    fn render_model_text_branch_detail_hints_are_section_sensitive() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/direct-actions".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-direct-actions")),
        }];
        model.branches.detail_section = 0;
        model.branches.docker_containers = vec![docker_container(
            "abc123",
            "web",
            gwt_docker::ContainerStatus::Running,
        )];

        let overview = render_model_text(&model, 200, 24);
        assert!(overview.contains("Shift+Enter:shell"));
        assert!(overview.contains("Ctrl+C:delete"));
        assert!(overview.contains("T:stop"));

        model.branches.branches[0].worktree_path = None;
        let no_worktree = render_model_text(&model, 200, 24);
        assert!(!no_worktree.contains("Shift+Enter:shell"));
        assert!(!no_worktree.contains("Ctrl+C:delete"));
        assert!(no_worktree.contains("Enter:launch"));

        model.branches.detail_section = 3;
        let sessions = render_model_text(&model, 200, 24);
        assert!(sessions.contains("↑↓:session"));
        assert!(sessions.contains("Enter:focus"));
    }

    #[test]
    fn render_model_text_branch_detail_hints_include_branch_local_mnemonics() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/mnemonics".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-mnemonics")),
        }];

        let rendered = render_model_text(&model, 220, 24);
        assert!(rendered.contains("m:view"));
        assert!(rendered.contains("v:git"));
        assert!(rendered.contains("f:search"));
        assert!(rendered.contains("?:help"));
    }

    #[test]
    fn route_key_to_management_branches_h_toggles_help_overlay() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;

        route_key_to_management(&mut model, key(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(model.help_visible);
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
    fn update_key_input_tab_on_non_branches_management_skips_branch_detail_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Tab, KeyModifiers::NONE)),
        );

        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn update_key_input_backtab_on_non_branches_management_skips_branch_detail_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Logs;
        model.active_focus = FocusPane::Terminal;

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::BackTab, KeyModifiers::SHIFT)),
        );

        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn update_key_input_tab_on_branches_still_cycles_into_branch_detail_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Tab, KeyModifiers::NONE)),
        );

        assert_eq!(model.active_focus, FocusPane::BranchDetail);
    }

    #[test]
    fn update_key_input_tab_on_non_branches_management_normalizes_stale_branch_detail_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::BranchDetail;

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Tab, KeyModifiers::NONE)),
        );

        assert_eq!(model.active_focus, FocusPane::Terminal);
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
