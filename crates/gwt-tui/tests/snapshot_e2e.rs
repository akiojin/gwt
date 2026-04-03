//! E2E snapshot tests for gwt-tui screens.
//!
//! Uses ratatui TestBackend + insta for screenshot-style testing.
//! Each test renders a screen to a fixed-size buffer and compares
//! against a stored snapshot (.snap file).

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use gwt_notification::Severity;
use gwt_tui::app;
use gwt_tui::input::keybind::KeybindRegistry;
use gwt_tui::message::Message;
use gwt_tui::model::{ActiveLayer, FocusPane, ManagementTab, Model, SessionLayout};
use gwt_tui::screens::branches::{BranchCategory, BranchItem, BranchesMessage};
use gwt_tui::screens::logs::{LogEntry, LogsMessage};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn test_model() -> Model {
    Model::new(PathBuf::from("/tmp/test-repo"))
}

/// Create a KeyEvent (Press only, matching the event.rs filter).
fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn ctrl(ch: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char(ch),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn backspace() -> KeyEvent {
    press(KeyCode::Backspace)
}

#[derive(Debug, Clone)]
enum DispatchStatus {
    Consumed(Message),
    Forwarded,
}

/// Simulate the full key input pipeline: keybind registry → app::update.
/// Returns whether the keybind layer consumed the event or forwarded it.
fn send_key(model: &mut Model, keybinds: &mut KeybindRegistry, key: KeyEvent) -> DispatchStatus {
    let forwarded_key = key;
    match keybinds.process_key(key) {
        Some(msg) => {
            app::update(model, msg.clone());
            DispatchStatus::Consumed(msg)
        }
        None => {
            app::update(model, Message::KeyInput(forwarded_key));
            DispatchStatus::Forwarded
        }
    }
}

/// Render the current model state to a string for snapshot comparison.
fn render_to_string(model: &Model, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            app::view(model, frame);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    buffer_to_string(&buffer)
}

/// Convert a ratatui Buffer to a readable string representation.
fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
    let mut output = String::new();
    let area = buffer.area;
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &buffer[(x, y)];
            output.push_str(cell.symbol());
        }
        // Trim trailing spaces for cleaner snapshots
        let trimmed = output.trim_end();
        output.truncate(trimmed.len());
        output.push('\n');
    }
    output
}

fn sample_branches() -> Vec<BranchItem> {
    vec![
        BranchItem {
            name: "main".to_string(),
            is_head: true,
            is_local: true,
            category: BranchCategory::Main,
        },
        BranchItem {
            name: "feature/api".to_string(),
            is_head: false,
            is_local: true,
            category: BranchCategory::Feature,
        },
        BranchItem {
            name: "feature/app-shell".to_string(),
            is_head: false,
            is_local: true,
            category: BranchCategory::Feature,
        },
        BranchItem {
            name: "origin/release/1.0".to_string(),
            is_head: false,
            is_local: false,
            category: BranchCategory::Other,
        },
    ]
}

fn sample_log_entries() -> Vec<LogEntry> {
    let timestamp: DateTime<Utc> = "2026-04-03T02:43:22.996912Z".parse().unwrap();

    vec![
        LogEntry {
            id: 1,
            severity: Severity::Error,
            source: "core".to_string(),
            message: "Failed to connect".to_string(),
            detail: Some("connection timed out".to_string()),
            timestamp,
        },
        LogEntry {
            id: 2,
            severity: Severity::Warn,
            source: "tui".to_string(),
            message: "Slow render".to_string(),
            detail: None,
            timestamp,
        },
        LogEntry {
            id: 3,
            severity: Severity::Info,
            source: "core".to_string(),
            message: "Started session".to_string(),
            detail: None,
            timestamp,
        },
        LogEntry {
            id: 4,
            severity: Severity::Debug,
            source: "pty".to_string(),
            message: "Buffer flush".to_string(),
            detail: None,
            timestamp,
        },
    ]
}

// ============================================================
// Screen Snapshots
// ============================================================

#[test]
fn snapshot_initial_management_layer() {
    let model = test_model();
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("initial_management_layer", output);
}

#[test]
fn snapshot_branches_tab() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Branches;
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("branches_tab", output);
}

#[test]
fn snapshot_issues_tab() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Issues;
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("issues_tab", output);
}

#[test]
fn snapshot_git_view_tab() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::GitView;
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("git_view_tab", output);
}

#[test]
fn snapshot_versions_tab() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Versions;
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("versions_tab", output);
}

#[test]
fn snapshot_settings_tab() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Settings;
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("settings_tab", output);
}

#[test]
fn snapshot_logs_tab() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Logs;
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("logs_tab", output);
}

#[test]
fn snapshot_logs_tab_filter_active() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Logs;
    app::update(
        &mut model,
        Message::Logs(LogsMessage::SetEntries(sample_log_entries())),
    );
    app::update(&mut model, Message::KeyInput(press(KeyCode::Char('f'))));
    app::update(&mut model, Message::KeyInput(press(KeyCode::Char('d'))));
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("logs_tab_filter_active", output);
}

#[test]
fn e2e_notifications_land_in_structured_log_for_all_severities() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Logs;

    let notifications = [
        gwt_notification::Notification::new(Severity::Debug, "pty", "Buffer flush"),
        gwt_notification::Notification::new(Severity::Info, "core", "Started session"),
        gwt_notification::Notification::new(Severity::Warn, "tui", "Slow render"),
        gwt_notification::Notification::new(Severity::Error, "core", "Failed to connect"),
    ];

    for notification in notifications {
        app::update(&mut model, Message::Notify(notification));
    }

    app::update(&mut model, Message::DismissError);

    let output = render_to_string(&model, 160, 24);
    assert!(output.contains("DEBUG"));
    assert!(output.contains("INFO"));
    assert!(output.contains("WARN"));
    assert!(output.contains("ERROR"));
    assert!(output.contains("pty"));
    assert!(output.contains("core"));
    assert!(output.contains("tui"));
    assert!(output.contains("Buffer flush"));
    assert!(output.contains("Started session"));
    assert!(output.contains("Slow render"));
    assert!(output.contains("Failed to connect"));
}

#[test]
fn e2e_info_notification_appears_in_status_bar_and_auto_dismisses() {
    let mut model = test_model();
    let notification = gwt_notification::Notification::new(Severity::Info, "core", "Started");

    app::update(&mut model, Message::Notify(notification));

    let output = render_to_string(&model, 120, 24);
    assert!(output.contains("INFO core: Started"));

    for _ in 0..50 {
        app::update(&mut model, Message::Tick);
    }

    let output = render_to_string(&model, 120, 24);
    assert!(!output.contains("INFO core: Started"));
    assert!(!output.contains("Started"));
}

#[test]
fn snapshot_pr_dashboard_tab() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::PrDashboard;
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("pr_dashboard_tab", output);
}

#[test]
fn snapshot_profiles_tab() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Profiles;
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("profiles_tab", output);
}

// ============================================================
// Initialization Screen Snapshots
// ============================================================

#[test]
fn snapshot_initialization_clone_wizard() {
    let model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("initialization_clone_wizard", output);
}

#[test]
fn snapshot_initialization_bare_migration() {
    let model = Model::new_initialization(PathBuf::from("/tmp/bare"), true);
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("initialization_bare_migration", output);
}

#[test]
fn snapshot_initialization_with_url_typed() {
    let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
    // Simulate typing a URL
    if let Some(init) = model.initialization_mut() {
        init.url_input = "https://github.com/user/repo.git".to_string();
    }
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("initialization_url_typed", output);
}

#[test]
fn snapshot_initialization_clone_error() {
    let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
    if let Some(init) = model.initialization_mut() {
        init.url_input = "https://bad.url/repo.git".to_string();
        init.clone_status = gwt_tui::screens::initialization::CloneStatus::Error(
            "fatal: repository not found".to_string(),
        );
    }
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("initialization_clone_error", output);
}

#[test]
fn e2e_initialization_esc_exits() {
    let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
    // In initialization, KeyInput is routed directly (no keybind registry)
    app::update(&mut model, Message::KeyInput(press(KeyCode::Esc)));
    assert!(model.quit);
}

#[test]
fn e2e_initialization_typing_url() {
    let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
    app::update(&mut model, Message::KeyInput(press(KeyCode::Char('h'))));
    app::update(&mut model, Message::KeyInput(press(KeyCode::Char('i'))));
    let init = model.initialization().unwrap();
    assert_eq!(init.url_input, "hi");
}

// ============================================================
// User Flow Snapshots
// ============================================================

#[test]
fn snapshot_toggle_to_main_layer() {
    let mut model = test_model();
    app::update(&mut model, Message::ToggleLayer);
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("main_layer_empty", output);
}

#[test]
fn snapshot_tab_grid_toggle() {
    let mut model = test_model();
    // Switch to main layer first
    app::update(&mut model, Message::ToggleLayer);
    // Toggle layout
    app::update(&mut model, Message::ToggleSessionLayout);
    let expected_layout = if model.session_layout == SessionLayout::Grid {
        "grid"
    } else {
        "tab"
    };
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!(format!("session_layout_{}", expected_layout), output);
}

#[test]
fn snapshot_error_overlay() {
    let mut model = test_model();
    app::update(
        &mut model,
        Message::PushError("Something went wrong".to_string()),
    );
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("error_overlay", output);
}

#[test]
fn e2e_error_modal_queue_preserved_after_dismiss() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();

    app::update(&mut model, Message::PushError("First failure".to_string()));
    app::update(&mut model, Message::PushError("Second failure".to_string()));

    let output = render_to_string(&model, 80, 24);
    assert!(output.contains("Error (1 of 2)"));
    assert!(output.contains("First failure"));
    assert!(output.contains("Press Enter or Esc to dismiss"));

    let status = send_key(&mut model, &mut kb, press(KeyCode::Enter));
    assert!(matches!(status, DispatchStatus::Forwarded));

    let output = render_to_string(&model, 80, 24);
    assert!(output.contains("Second failure"));
    assert!(!output.contains("First failure"));
    assert!(!output.contains("1 of 2"));
}

#[test]
fn e2e_error_modal_survives_burst_of_100_errors() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();

    for i in 0..100 {
        app::update(&mut model, Message::PushError(format!("Burst failure {i}")));
        let output = render_to_string(&model, 80, 24);
        assert!(output.contains("Burst failure 0"));
    }

    let output = render_to_string(&model, 80, 24);
    assert!(output.contains("Burst failure 0"));

    let status = send_key(&mut model, &mut kb, press(KeyCode::Enter));
    assert!(matches!(status, DispatchStatus::Forwarded));

    let output = render_to_string(&model, 80, 24);
    assert!(output.contains("Burst failure 1"));
    assert!(!output.contains("Burst failure 0"));
}

#[test]
fn snapshot_management_tab_switch() {
    let mut model = test_model();
    app::update(
        &mut model,
        Message::SwitchManagementTab(ManagementTab::Settings),
    );
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("switched_to_settings", output);
}

#[test]
fn snapshot_wide_terminal() {
    let model = test_model();
    let output = render_to_string(&model, 120, 40);
    insta::assert_snapshot!("wide_terminal_120x40", output);
}

#[test]
fn snapshot_narrow_terminal() {
    let model = test_model();
    let output = render_to_string(&model, 40, 15);
    insta::assert_snapshot!("narrow_terminal_40x15", output);
}

// ============================================================
// Key Input E2E Tests — full pipeline: key → keybind → update → render
// ============================================================

#[test]
fn e2e_ctrl_g_g_toggles_to_main() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();
    // Default is now Management
    assert_eq!(model.active_layer, ActiveLayer::Management);

    // Ctrl+G,g → toggle to Main
    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('g')));
    assert_eq!(model.active_layer, ActiveLayer::Main);
}

#[test]
fn e2e_ctrl_g_g_toggles_back_to_management() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();

    // Toggle to Main
    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('g')));
    assert_eq!(model.active_layer, ActiveLayer::Main);

    // Toggle back to Management
    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('g')));
    assert_eq!(model.active_layer, ActiveLayer::Management);
}

#[test]
fn e2e_j_k_navigates_branches_in_management() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();

    // Default is Management + Branches
    assert_eq!(model.active_layer, ActiveLayer::Management);
    assert_eq!(model.management_tab, ManagementTab::Branches);

    // Press 'j' — should be processed (not ignored)
    send_key(&mut model, &mut kb, press(KeyCode::Char('j')));
    // Verify render doesn't panic after key input
    let output = render_to_string(&model, 80, 24);
    assert!(output.contains("Branches"));
}

#[test]
fn e2e_ctrl_g_c_creates_new_shell() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();
    let initial_count = model.session_count();

    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('c')));

    assert_eq!(model.session_count(), initial_count + 1);
}

#[test]
fn e2e_ctrl_g_z_toggles_layout() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();
    assert_eq!(model.session_layout, SessionLayout::Tab);

    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('z')));

    assert_eq!(model.session_layout, SessionLayout::Grid);
}

#[test]
fn e2e_ctrl_g_n_opens_wizard() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();
    assert!(!model.has_wizard());

    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('n')));

    assert!(model.has_wizard());
}

#[test]
fn e2e_ctrl_c_double_tap_quits() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();
    model.active_layer = ActiveLayer::Main;
    assert!(!model.quit);

    let first = send_key(&mut model, &mut kb, ctrl('c'));
    assert!(matches!(first, DispatchStatus::Forwarded));
    assert!(!model.quit); // single tap: no quit
    assert_eq!(model.pending_pty_inputs().len(), 1);
    let forwarded = model.pending_pty_inputs().back().unwrap();
    assert_eq!(forwarded.session_id, "shell-0");
    assert_eq!(forwarded.bytes, vec![0x03]);

    let second = send_key(&mut model, &mut kb, ctrl('c'));
    assert!(matches!(second, DispatchStatus::Consumed(Message::Quit)));
    assert!(model.quit); // double tap: quit
    assert_eq!(model.pending_pty_inputs().len(), 1);
}

#[test]
fn e2e_ctrl_g_q_quits() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();
    assert!(!model.quit);

    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('q')));

    assert!(model.quit);
}

#[test]
fn e2e_management_tab_switch_via_ctrl_g_s() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();

    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('s')));

    assert_eq!(model.management_tab, ManagementTab::Settings);
    assert_eq!(model.active_layer, ActiveLayer::Management);
}

#[test]
fn e2e_management_tab_switch_via_ctrl_g_i() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();

    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('i')));

    assert_eq!(model.management_tab, ManagementTab::Issues);
}

#[test]
fn e2e_search_in_branches() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();
    app::update(
        &mut model,
        Message::Branches(BranchesMessage::SetBranches(sample_branches())),
    );

    // Already in Management > Branches by default

    // Press '/' to start search
    let search_start = send_key(&mut model, &mut kb, press(KeyCode::Char('/')));
    assert!(matches!(search_start, DispatchStatus::Forwarded));
    assert!(model.is_branches_search_active());

    let _ = send_key(&mut model, &mut kb, press(KeyCode::Char('a')));
    let _ = send_key(&mut model, &mut kb, press(KeyCode::Char('p')));
    let _ = send_key(&mut model, &mut kb, press(KeyCode::Char('i')));

    assert_eq!(model.branches_search_query(), "api");
    assert_eq!(
        model.filtered_branch_names(),
        vec!["feature/api".to_string()]
    );

    let _ = send_key(&mut model, &mut kb, backspace());

    assert_eq!(model.branches_search_query(), "ap");
    assert_eq!(
        model.filtered_branch_names(),
        vec!["feature/api".to_string(), "feature/app-shell".to_string(),]
    );
}

#[test]
fn e2e_full_flow_snapshot_after_key_sequence() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();

    // Ctrl+G,g → Management
    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('g')));

    // Ctrl+G,s → Settings tab
    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('s')));

    assert_eq!(model.management_tab, ManagementTab::Settings);

    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("e2e_key_flow_settings", output);
}

// ============================================================
// Branch Detail View E2E Tests
// ============================================================

#[test]
fn snapshot_branches_with_detail_split() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Branches;
    app::update(
        &mut model,
        Message::Branches(BranchesMessage::SetBranches(sample_branches())),
    );
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("branches_detail_split", output);
}

#[test]
fn e2e_branch_detail_arrows_cycle_sections() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();
    app::update(
        &mut model,
        Message::Branches(BranchesMessage::SetBranches(sample_branches())),
    );

    // Focus on BranchDetail pane (Tab twice from default TabContent)
    send_key(&mut model, &mut kb, press(KeyCode::Tab));
    assert_eq!(model.active_focus, FocusPane::BranchDetail);

    // Default section is 0 (Overview)
    assert_eq!(model.branches_detail_section(), 0);

    // Right -> section 1
    send_key(&mut model, &mut kb, press(KeyCode::Right));
    assert_eq!(model.branches_detail_section(), 1);

    // Right -> section 2
    send_key(&mut model, &mut kb, press(KeyCode::Right));
    assert_eq!(model.branches_detail_section(), 2);

    // Left -> section 1
    send_key(&mut model, &mut kb, press(KeyCode::Left));
    assert_eq!(model.branches_detail_section(), 1);
}

#[test]
fn snapshot_branches_action_modal() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Branches;
    app::update(
        &mut model,
        Message::Branches(BranchesMessage::SetBranches(sample_branches())),
    );
    // Open action modal
    app::update(
        &mut model,
        Message::Branches(BranchesMessage::OpenActionModal),
    );
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("branches_action_modal", output);
}

#[test]
fn e2e_action_modal_select_triggers_launch_agent() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();
    app::update(
        &mut model,
        Message::Branches(BranchesMessage::SetBranches(sample_branches())),
    );
    // Open action modal
    app::update(
        &mut model,
        Message::Branches(BranchesMessage::OpenActionModal),
    );
    // Select first action (Launch Agent) via Enter
    send_key(&mut model, &mut kb, press(KeyCode::Enter));
    assert!(model.branches_pending_launch_agent());
}
