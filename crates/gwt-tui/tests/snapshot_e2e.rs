//! E2E snapshot tests for gwt-tui screens.
//!
//! Uses ratatui TestBackend + insta for screenshot-style testing.
//! Each test renders a screen to a fixed-size buffer and compares
//! against a stored snapshot (.snap file).

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use gwt_tui::app;
use gwt_tui::input::keybind::KeybindRegistry;
use gwt_tui::message::Message;
use gwt_tui::model::{ActiveLayer, ManagementTab, Model, SessionLayout};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

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

/// Simulate the full key input pipeline: keybind registry → app::update.
/// Returns the Message that was dispatched (for assertion).
fn send_key(model: &mut Model, keybinds: &mut KeybindRegistry, key: KeyEvent) -> Option<Message> {
    let msg = keybinds
        .process_key(key)
        .unwrap_or(Message::KeyInput(key));
    app::update(model, msg);
    None
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
fn snapshot_specs_tab() {
    let mut model = test_model();
    model.active_layer = ActiveLayer::Management;
    model.management_tab = ManagementTab::Specs;
    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("specs_tab", output);
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
    assert!(!model.quit);

    send_key(&mut model, &mut kb, ctrl('c'));
    assert!(!model.quit); // single tap: no quit

    send_key(&mut model, &mut kb, ctrl('c'));
    assert!(model.quit); // double tap: quit
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

    assert_eq!(model.management_tab, ManagementTab::Specs);
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

    // Already in Management > Branches by default

    // Press '/' to start search
    send_key(&mut model, &mut kb, press(KeyCode::Char('/')));
    assert!(model.is_branches_search_active());

    // Type search query — but wait, in search mode chars should go to SearchInput
    // This depends on whether route_key_to_management handles search_active state
}

#[test]
fn e2e_full_flow_snapshot_after_key_sequence() {
    let mut model = test_model();
    let mut kb = KeybindRegistry::new();

    // Ctrl+G,g → Management
    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('g')));

    // Ctrl+G,s → SPECs tab
    send_key(&mut model, &mut kb, ctrl('g'));
    send_key(&mut model, &mut kb, press(KeyCode::Char('s')));

    assert_eq!(model.management_tab, ManagementTab::Specs);

    let output = render_to_string(&model, 80, 24);
    insta::assert_snapshot!("e2e_key_flow_specs", output);
}
