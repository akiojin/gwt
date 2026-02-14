//! Native menubar wiring (Tauri menu).

use crate::state::AppState;
use gwt_core::config::ProfilesConfig;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, SubmenuBuilder};
use tauri::{AppHandle, Manager, Wry};

pub const MENU_ACTION_EVENT: &str = "menu-action";

pub const MENU_ID_FILE_NEW_WINDOW: &str = "file-new-window";
pub const MENU_ID_FILE_OPEN_PROJECT: &str = "file-open-project";
pub const MENU_ID_FILE_CLOSE_PROJECT: &str = "file-close-project";

pub const MENU_ID_TOOLS_NEW_TERMINAL: &str = "tools-new-terminal";
pub const MENU_ID_TOOLS_LAUNCH_AGENT: &str = "tools-launch-agent";
pub const MENU_ID_TOOLS_LIST_TERMINALS: &str = "tools-list-terminals";
pub const MENU_ID_TOOLS_TERMINAL_DIAGNOSTICS: &str = "tools-terminal-diagnostics";

pub const MENU_ID_GIT_CLEANUP_WORKTREES: &str = "git-cleanup-worktrees";
pub const MENU_ID_GIT_VERSION_HISTORY: &str = "git-version-history";

pub const MENU_ID_EDIT_COPY: &str = "edit-copy";
pub const MENU_ID_EDIT_PASTE: &str = "edit-paste";

pub const MENU_ID_WINDOW_PREVIOUS_TAB: &str = "window-previous-tab";
pub const MENU_ID_WINDOW_NEXT_TAB: &str = "window-next-tab";
pub const MENU_ID_WINDOW_NEXT_WINDOW: &str = "window-next-window";
pub const MENU_ID_WINDOW_PREVIOUS_WINDOW: &str = "window-previous-window";
pub const MENU_ID_WINDOW_MINIMIZE: &str = "window-minimize";
pub const MENU_ID_WINDOW_ZOOM: &str = "window-zoom";
pub const MENU_ID_WINDOW_BRING_ALL_TO_FRONT: &str = "window-bring-all-to-front";

pub const MENU_ID_SETTINGS_PREFERENCES: &str = "settings-preferences";
pub const MENU_ID_HELP_ABOUT: &str = "help-about";
pub const MENU_ID_HELP_CHECK_UPDATES: &str = "help-check-updates";

pub const RECENT_PROJECT_PREFIX: &str = "recent-project::";
pub const WINDOW_FOCUS_MENU_PREFIX: &str = "window-focus::";
pub const WINDOW_TAB_FOCUS_MENU_PREFIX: &str = "window-tab-focus::";
const MIGRATION_DISPLAY_PREFIX: &str = "Migrating: ";

#[derive(Debug, Clone, Serialize)]
pub struct MenuActionPayload {
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowMenuEntry {
    pub window_label: String,
    pub project_path: String,
    pub display: String,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowTabMenuEntry {
    pub tab_id: String,
    pub label: String,
    pub active: bool,
}

pub fn window_focus_menu_id(window_label: &str) -> String {
    format!("{WINDOW_FOCUS_MENU_PREFIX}{window_label}")
}

pub fn window_tab_focus_menu_id(tab_id: &str) -> String {
    format!("{WINDOW_TAB_FOCUS_MENU_PREFIX}{tab_id}")
}

pub fn parse_recent_project_menu_id(id: &str) -> Option<&str> {
    id.strip_prefix(RECENT_PROJECT_PREFIX)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

pub fn parse_window_focus_menu_id(id: &str) -> Option<&str> {
    id.strip_prefix(WINDOW_FOCUS_MENU_PREFIX)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

pub fn parse_window_tab_focus_menu_id(id: &str) -> Option<&str> {
    id.strip_prefix(WINDOW_TAB_FOCUS_MENU_PREFIX)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

pub fn rebuild_menu(app: &AppHandle<Wry>) -> tauri::Result<()> {
    let state = app.state::<AppState>();
    let menu = build_menu(app, &state)?;
    let _ = app.set_menu(menu)?;
    Ok(())
}

pub fn build_menu(app: &AppHandle<Wry>, state: &AppState) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;

    let app_menu_label = app.package_info().name.clone();

    let file_new_window = MenuItem::with_id(
        app,
        MENU_ID_FILE_NEW_WINDOW,
        "New Window",
        true,
        Some("CmdOrCtrl+N"),
    )?;
    let file_open_project = MenuItem::with_id(
        app,
        MENU_ID_FILE_OPEN_PROJECT,
        "Open Project...",
        true,
        Some("CmdOrCtrl+O"),
    )?;
    let file_close_project = MenuItem::with_id(
        app,
        MENU_ID_FILE_CLOSE_PROJECT,
        "Close Project",
        true,
        None::<&str>,
    )?;
    let open_recent = build_open_recent_submenu(app)?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&file_new_window)
        .separator()
        .item(&file_open_project)
        .item(&open_recent)
        .separator()
        .item(&file_close_project)
        .build()?;

    let edit_copy = MenuItem::with_id(app, MENU_ID_EDIT_COPY, "Copy", true, Some("CmdOrCtrl+C"))?;
    let edit_paste =
        MenuItem::with_id(app, MENU_ID_EDIT_PASTE, "Paste", true, Some("CmdOrCtrl+V"))?;

    // Keep Undo/Redo/Cut/Select All as native actions and custom-bind Copy/Paste
    // so keyboard events can be handled by the app.
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .item(&edit_copy)
        .item(&edit_paste)
        .separator()
        .select_all()
        .build()?;

    let git_cleanup_worktrees = MenuItem::with_id(
        app,
        MENU_ID_GIT_CLEANUP_WORKTREES,
        "Cleanup Worktrees...",
        true,
        Some("CmdOrCtrl+Shift+K"),
    )?;
    let mut git_builder = SubmenuBuilder::new(app, "Git");

    if should_show_version_history_menu(app, state) {
        let version_history = MenuItem::with_id(
            app,
            MENU_ID_GIT_VERSION_HISTORY,
            "Version History...",
            true,
            None::<&str>,
        )?;
        git_builder = git_builder.item(&version_history).separator();
    }

    let git = git_builder.item(&git_cleanup_worktrees).build()?;

    let tools_new_terminal = MenuItem::with_id(
        app,
        MENU_ID_TOOLS_NEW_TERMINAL,
        "New Terminal",
        true,
        None::<&str>,
    )?;
    let tools_launch_agent = MenuItem::with_id(
        app,
        MENU_ID_TOOLS_LAUNCH_AGENT,
        "Launch Agent...",
        true,
        None::<&str>,
    )?;
    let tools_list_terminals = MenuItem::with_id(
        app,
        MENU_ID_TOOLS_LIST_TERMINALS,
        "List Terminals",
        true,
        None::<&str>,
    )?;
    let tools_terminal_diagnostics = MenuItem::with_id(
        app,
        MENU_ID_TOOLS_TERMINAL_DIAGNOSTICS,
        "Terminal Diagnostics",
        true,
        None::<&str>,
    )?;
    let tools = SubmenuBuilder::new(app, "Tools")
        .item(&tools_new_terminal)
        .separator()
        .item(&tools_launch_agent)
        .item(&tools_list_terminals)
        .item(&tools_terminal_diagnostics)
        .build()?;

    let window = build_window_submenu(app, state)?;
    let help_about = MenuItem::with_id(app, MENU_ID_HELP_ABOUT, "About gwt", true, None::<&str>)?;
    let help_check_updates = MenuItem::with_id(
        app,
        MENU_ID_HELP_CHECK_UPDATES,
        "Check for Updates...",
        true,
        None::<&str>,
    )?;
    let settings_prefs = MenuItem::with_id(
        app,
        MENU_ID_SETTINGS_PREFERENCES,
        "Preferences...",
        true,
        Some("CmdOrCtrl+,"),
    )?;

    #[cfg(target_os = "macos")]
    let gwt = SubmenuBuilder::new(app, app_menu_label)
        .item(&help_about)
        .item(&help_check_updates)
        .separator()
        .item(&settings_prefs)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    #[cfg(not(target_os = "macos"))]
    let gwt = SubmenuBuilder::new(app, app_menu_label)
        .item(&help_about)
        .item(&help_check_updates)
        .separator()
        .item(&settings_prefs)
        .build()?;

    menu.append(&gwt)?;
    menu.append(&file)?;
    menu.append(&edit)?;
    menu.append(&git)?;
    menu.append(&tools)?;
    menu.append(&window)?;
    Ok(menu)
}

fn build_open_recent_submenu(app: &AppHandle<Wry>) -> tauri::Result<tauri::menu::Submenu<Wry>> {
    let projects = gwt_core::config::load_recent_projects();
    let mut builder = SubmenuBuilder::new(app, "Open Recent");

    if projects.is_empty() {
        let none = MenuItem::with_id(
            app,
            "recent-none",
            "No Recent Projects",
            false,
            None::<&str>,
        )?;
        builder = builder.item(&none);
    } else {
        for entry in projects.into_iter().take(10) {
            let id = format!("{}{}", RECENT_PROJECT_PREFIX, entry.path);
            let item = MenuItem::with_id(app, id, &entry.path, true, None::<&str>)?;
            builder = builder.item(&item);
        }
    }

    builder.build()
}

fn should_show_version_history_menu(app: &AppHandle<Wry>, state: &AppState) -> bool {
    // Only show when there is an open project in the currently focused window
    // and AI settings are configured.
    let focused_label = focused_window_label(app);

    if state.project_for_window(&focused_label).is_none() {
        return false;
    }

    let Ok(profiles) = ProfilesConfig::load() else {
        return false;
    };
    let ai = profiles.resolve_active_ai_settings();
    ai.resolved.is_some()
}

fn build_window_submenu(
    app: &AppHandle<Wry>,
    state: &AppState,
) -> tauri::Result<tauri::menu::Submenu<Wry>> {
    let tab_entries = collect_agent_tab_entries(app, state);
    let window_entries = collect_window_entries(app, state);

    let mut builder = SubmenuBuilder::new(app, "Window");

    // 1. Tab switching items
    let previous_tab = MenuItem::with_id(
        app,
        MENU_ID_WINDOW_PREVIOUS_TAB,
        "Previous Tab",
        true,
        Some("CmdOrCtrl+Shift+["),
    )?;
    let next_tab = MenuItem::with_id(
        app,
        MENU_ID_WINDOW_NEXT_TAB,
        "Next Tab",
        true,
        Some("CmdOrCtrl+Shift+]"),
    )?;
    builder = builder.item(&previous_tab).item(&next_tab).separator();

    // 2. Window switching items
    let previous_window = MenuItem::with_id(
        app,
        MENU_ID_WINDOW_PREVIOUS_WINDOW,
        "Previous Window",
        true,
        None::<&str>,
    )?;
    let next_window = MenuItem::with_id(
        app,
        MENU_ID_WINDOW_NEXT_WINDOW,
        "Next Window",
        true,
        None::<&str>,
    )?;
    builder = builder
        .item(&previous_window)
        .item(&next_window)
        .separator();

    // 3. Tab list
    if tab_entries.is_empty() {
        let none_tabs = MenuItem::with_id(
            app,
            "window-tabs-none",
            "No Agent Tabs",
            false,
            None::<&str>,
        )?;
        builder = builder.item(&none_tabs);
    } else {
        for e in tab_entries {
            let item = CheckMenuItem::with_id(
                app,
                window_tab_focus_menu_id(&e.tab_id),
                &e.label,
                true,
                e.active,
                None::<&str>,
            )?;
            builder = builder.item(&item);
        }
    }

    builder = builder.separator();

    // 4. Minimize / Zoom (macOS only)
    #[cfg(target_os = "macos")]
    {
        let minimize = MenuItem::with_id(
            app,
            MENU_ID_WINDOW_MINIMIZE,
            "Minimize",
            true,
            Some("CmdOrCtrl+M"),
        )?;
        let zoom = MenuItem::with_id(app, MENU_ID_WINDOW_ZOOM, "Zoom", true, None::<&str>)?;
        builder = builder.item(&minimize).item(&zoom).separator();
    }

    // 5. Window list
    if window_entries.is_empty() {
        let none_windows = MenuItem::with_id(
            app,
            "window-none",
            "No Project Windows",
            false,
            None::<&str>,
        )?;
        builder = builder.item(&none_windows);
    } else {
        let mut sorted = window_entries;
        sorted.sort_by(|a, b| a.display.cmp(&b.display));

        for e in sorted {
            let item = CheckMenuItem::with_id(
                app,
                window_focus_menu_id(&e.window_label),
                &e.display,
                true,
                e.focused,
                None::<&str>,
            )?;
            builder = builder.item(&item);
        }
    }

    // 6. Bring All to Front (macOS only)
    #[cfg(target_os = "macos")]
    {
        let bring_all = MenuItem::with_id(
            app,
            MENU_ID_WINDOW_BRING_ALL_TO_FRONT,
            "Bring All to Front",
            true,
            None::<&str>,
        )?;
        builder = builder.separator().item(&bring_all);
    }

    builder.build()
}

fn collect_agent_tab_entries(app: &AppHandle<Wry>, state: &AppState) -> Vec<WindowTabMenuEntry> {
    let focused_label = focused_window_label(app);
    let window_tabs = state.window_agent_tabs_for_window(&focused_label);
    let active_tab_id = window_tabs.active_tab_id;

    window_tabs
        .tabs
        .into_iter()
        .map(|tab| {
            let active = active_tab_id.as_deref() == Some(tab.id.as_str());
            WindowTabMenuEntry {
                tab_id: tab.id,
                label: tab.label,
                active,
            }
        })
        .collect()
}

fn collect_window_entries(app: &AppHandle<Wry>, state: &AppState) -> Vec<WindowMenuEntry> {
    let projects = match state.window_projects.lock() {
        Ok(m) => m.clone(),
        Err(_) => HashMap::new(),
    };
    let migrations = state.window_migrations_snapshot();

    if projects.is_empty() && migrations.is_empty() {
        return vec![];
    }

    // Determine focused window by scanning (stable API).
    let focused_label = focused_window_label(app);

    let mut project_raw: Vec<(String, String)> = Vec::new();
    for (label, path) in &projects {
        if !is_window_visible(app, label) {
            continue;
        }
        project_raw.push((label.clone(), path.clone()));
    }

    let mut migration_raw: Vec<(String, String)> = Vec::new();
    for (label, migration) in migrations {
        // A window with an opened project should use the project display,
        // not an additional migration entry.
        if projects.contains_key(&label) {
            continue;
        }
        if !is_window_visible(app, &label) {
            continue;
        }
        migration_raw.push((label, migration.source_root));
    }

    let project_displays = disambiguate_project_displays(&project_raw);
    let migration_displays = disambiguate_migration_displays(&migration_raw);

    let mut out: Vec<WindowMenuEntry> = Vec::with_capacity(project_raw.len() + migration_raw.len());
    for (label, path) in project_raw {
        out.push(WindowMenuEntry {
            focused: label == focused_label,
            display: project_displays
                .get(&label)
                .cloned()
                .unwrap_or_else(|| fallback_display_from_path(&path)),
            window_label: label,
            project_path: path,
        });
    }

    for (label, path) in migration_raw {
        out.push(WindowMenuEntry {
            focused: label == focused_label,
            display: migration_displays
                .get(&label)
                .cloned()
                .unwrap_or_else(|| migration_display_from_path(&path)),
            window_label: label,
            project_path: path,
        });
    }

    out
}

fn is_window_visible(app: &AppHandle<Wry>, label: &str) -> bool {
    let Some(window) = app.get_webview_window(label) else {
        return false;
    };
    window.is_visible().ok() != Some(false)
}

fn migration_display_from_path(project_path: &str) -> String {
    format!(
        "{MIGRATION_DISPLAY_PREFIX}{}",
        fallback_display_from_path(project_path)
    )
}

fn focused_window_label(app: &AppHandle<Wry>) -> String {
    app.webview_windows()
        .into_iter()
        .find_map(|(label, w)| w.is_focused().ok().and_then(|f| f.then_some(label)))
        .unwrap_or_else(|| "main".to_string())
}

fn fallback_display_from_path(project_path: &str) -> String {
    Path::new(project_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| project_path.to_string())
}

fn disambiguate_project_displays(entries: &[(String, String)]) -> HashMap<String, String> {
    let mut base_counts: HashMap<String, usize> = HashMap::new();
    let mut bases: HashMap<String, String> = HashMap::new();

    for (label, path) in entries {
        let base = fallback_display_from_path(path);
        *base_counts.entry(base.clone()).or_insert(0) += 1;
        bases.insert(label.clone(), base);
    }

    let mut out: HashMap<String, String> = HashMap::new();
    for (label, path) in entries {
        let base = bases
            .get(label)
            .cloned()
            .unwrap_or_else(|| fallback_display_from_path(path));
        let count = base_counts.get(&base).copied().unwrap_or(1);
        if count <= 1 {
            out.insert(label.clone(), base);
        } else {
            out.insert(label.clone(), format!("{base} - {path}"));
        }
    }

    out
}

fn disambiguate_migration_displays(entries: &[(String, String)]) -> HashMap<String, String> {
    let mut base_counts: HashMap<String, usize> = HashMap::new();
    let mut bases: HashMap<String, String> = HashMap::new();

    for (label, path) in entries {
        let base = migration_display_from_path(path);
        *base_counts.entry(base.clone()).or_insert(0) += 1;
        bases.insert(label.clone(), base);
    }

    let mut out: HashMap<String, String> = HashMap::new();
    for (label, path) in entries {
        let base = bases
            .get(label)
            .cloned()
            .unwrap_or_else(|| migration_display_from_path(path));
        let count = base_counts.get(&base).copied().unwrap_or(1);
        if count <= 1 {
            out.insert(label.clone(), base);
        } else {
            out.insert(label.clone(), format!("{base} - {path}"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_window_focus_menu_id_rejects_non_matching() {
        assert_eq!(parse_window_focus_menu_id("file-open-project"), None);
        assert_eq!(parse_window_focus_menu_id("window-focus::"), None);
        assert_eq!(parse_window_focus_menu_id("window-focus::   "), None);
    }

    #[test]
    fn parse_window_focus_menu_id_extracts_label() {
        assert_eq!(
            parse_window_focus_menu_id("window-focus::main"),
            Some("main")
        );
        assert_eq!(
            parse_window_focus_menu_id("window-focus::project-123"),
            Some("project-123")
        );
    }

    #[test]
    fn parse_window_tab_focus_menu_id_rejects_non_matching() {
        assert_eq!(parse_window_tab_focus_menu_id("window-focus::main"), None);
        assert_eq!(parse_window_tab_focus_menu_id("window-tab-focus::"), None);
        assert_eq!(parse_window_tab_focus_menu_id("window-tab-focus::  "), None);
    }

    #[test]
    fn parse_window_tab_focus_menu_id_extracts_id() {
        assert_eq!(
            parse_window_tab_focus_menu_id("window-tab-focus::agent-pane-1"),
            Some("agent-pane-1")
        );
    }

    #[test]
    fn disambiguate_project_displays_uses_basename_when_unique() {
        let entries = vec![
            ("w1".to_string(), "/a/repo1".to_string()),
            ("w2".to_string(), "/b/repo2".to_string()),
        ];
        let map = disambiguate_project_displays(&entries);
        assert_eq!(map.get("w1").unwrap(), "repo1");
        assert_eq!(map.get("w2").unwrap(), "repo2");
    }

    #[test]
    fn disambiguate_project_displays_adds_path_when_duplicate_basename() {
        let entries = vec![
            ("w1".to_string(), "/a/repo".to_string()),
            ("w2".to_string(), "/b/repo".to_string()),
        ];
        let map = disambiguate_project_displays(&entries);
        assert_eq!(map.get("w1").unwrap(), "repo - /a/repo");
        assert_eq!(map.get("w2").unwrap(), "repo - /b/repo");
    }

    #[test]
    fn disambiguate_migration_displays_prefixes_label() {
        let entries = vec![("w1".to_string(), "/a/repo1".to_string())];
        let map = disambiguate_migration_displays(&entries);
        assert_eq!(map.get("w1").unwrap(), "Migrating: repo1");
    }

    #[test]
    fn disambiguate_migration_displays_adds_path_when_duplicate_basename() {
        let entries = vec![
            ("w1".to_string(), "/a/repo".to_string()),
            ("w2".to_string(), "/b/repo".to_string()),
        ];
        let map = disambiguate_migration_displays(&entries);
        assert_eq!(map.get("w1").unwrap(), "Migrating: repo - /a/repo");
        assert_eq!(map.get("w2").unwrap(), "Migrating: repo - /b/repo");
    }
}
