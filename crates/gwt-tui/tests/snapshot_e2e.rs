//! E2E snapshot tests for gwt-tui screens.
//!
//! Uses ratatui TestBackend + insta for screenshot-style testing.
//! Each test renders a screen to a fixed-size buffer and compares
//! against a stored snapshot (.snap file).

use std::path::PathBuf;

use gwt_tui::app;
use gwt_tui::message::Message;
use gwt_tui::model::{ActiveLayer, ManagementTab, Model, SessionLayout};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn test_model() -> Model {
    Model::new(PathBuf::from("/tmp/test-repo"))
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
