//! Native menubar wiring (Tauri menu).

use crate::state::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, SubmenuBuilder};
use tauri::{AppHandle, Manager, Wry};

pub const MENU_ACTION_EVENT: &str = "menu-action";

pub const MENU_ID_FILE_NEW_WINDOW: &str = "file-new-window";
pub const MENU_ID_FILE_OPEN_PROJECT: &str = "file-open-project";
pub const MENU_ID_FILE_CLOSE_PROJECT: &str = "file-close-project";

pub const MENU_ID_VIEW_TOGGLE_SIDEBAR: &str = "view-toggle-sidebar";
pub const MENU_ID_VIEW_LAUNCH_AGENT: &str = "view-launch-agent";
pub const MENU_ID_VIEW_LIST_TERMINALS: &str = "view-list-terminals";
pub const MENU_ID_VIEW_TERMINAL_DIAGNOSTICS: &str = "view-terminal-diagnostics";

pub const MENU_ID_GIT_CLEANUP_WORKTREES: &str = "git-cleanup-worktrees";

pub const MENU_ID_SETTINGS_PREFERENCES: &str = "settings-preferences";
pub const MENU_ID_HELP_ABOUT: &str = "help-about";
pub const MENU_ID_DEBUG_OS_ENV: &str = "debug-os-env";

pub const WINDOW_FOCUS_MENU_PREFIX: &str = "window-focus::";

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

pub fn window_focus_menu_id(window_label: &str) -> String {
    format!("{WINDOW_FOCUS_MENU_PREFIX}{window_label}")
}

pub fn parse_window_focus_menu_id(id: &str) -> Option<&str> {
    id.strip_prefix(WINDOW_FOCUS_MENU_PREFIX)
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
    let file = SubmenuBuilder::new(app, "File")
        .item(&file_new_window)
        .separator()
        .item(&file_open_project)
        .item(&file_close_project)
        .build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .cut()
        .copy()
        .paste()
        .separator()
        .select_all()
        .build()?;

    let view_toggle_sidebar = MenuItem::with_id(
        app,
        MENU_ID_VIEW_TOGGLE_SIDEBAR,
        "Toggle Sidebar",
        true,
        Some("CmdOrCtrl+B"),
    )?;
    let view_launch_agent = MenuItem::with_id(
        app,
        MENU_ID_VIEW_LAUNCH_AGENT,
        "Launch Agent...",
        true,
        None::<&str>,
    )?;
    let view_list_terminals = MenuItem::with_id(
        app,
        MENU_ID_VIEW_LIST_TERMINALS,
        "List Terminals",
        true,
        None::<&str>,
    )?;
    let view_terminal_diagnostics = MenuItem::with_id(
        app,
        MENU_ID_VIEW_TERMINAL_DIAGNOSTICS,
        "Terminal Diagnostics",
        true,
        None::<&str>,
    )?;
    let view = SubmenuBuilder::new(app, "View")
        .item(&view_toggle_sidebar)
        .separator()
        .item(&view_launch_agent)
        .item(&view_list_terminals)
        .item(&view_terminal_diagnostics)
        .build()?;

    let git_cleanup = MenuItem::with_id(
        app,
        MENU_ID_GIT_CLEANUP_WORKTREES,
        "Cleanup Worktrees...",
        true,
        Some("CmdOrCtrl+Shift+K"),
    )?;
    let git = SubmenuBuilder::new(app, "Git").item(&git_cleanup).build()?;

    let window = build_window_submenu(app, state)?;

    let settings_prefs = MenuItem::with_id(
        app,
        MENU_ID_SETTINGS_PREFERENCES,
        "Preferences...",
        true,
        Some("CmdOrCtrl+,"),
    )?;
    let settings = SubmenuBuilder::new(app, "Settings")
        .item(&settings_prefs)
        .build()?;

    let help_about = MenuItem::with_id(app, MENU_ID_HELP_ABOUT, "About gwt", true, None::<&str>)?;
    let help = SubmenuBuilder::new(app, "Help").item(&help_about).build()?;

    menu.append(&file)?;
    menu.append(&edit)?;
    menu.append(&view)?;
    menu.append(&git)?;
    menu.append(&window)?;
    menu.append(&settings)?;
    menu.append(&help)?;
    Ok(menu)
}

    let debug_os_env = MenuItem::with_id(
        app,
        MENU_ID_DEBUG_OS_ENV,
        "Show Captured Environment",
        true,
        None::<&str>,
    )?;
    let debug = SubmenuBuilder::new(app, "Debug")
        .item(&debug_os_env)
        .build()?;

    let help_about = MenuItem::with_id(app, MENU_ID_HELP_ABOUT, "About gwt", true, None::<&str>)?;
    let settings_prefs = MenuItem::with_id(
        app,
        MENU_ID_SETTINGS_PREFERENCES,
        "Preferences...",
        true,
        Some("CmdOrCtrl+,"),
    )?;
    let gwt = SubmenuBuilder::new(app, app_menu_label)
        .item(&help_about)
        .separator()
        .item(&settings_prefs)
        .build()?;

    menu.append(&gwt)?;
    menu.append(&file)?;
    menu.append(&edit)?;
    menu.append(&view)?;
    menu.append(&git)?;
    menu.append(&window)?;
    menu.append(&debug)?;
    Ok(menu)
}

fn build_window_submenu(
    app: &AppHandle<Wry>,
    state: &AppState,
) -> tauri::Result<tauri::menu::Submenu<Wry>> {
    let entries = collect_window_entries(app, state);

    let mut builder = SubmenuBuilder::new(app, "Window");

    if entries.is_empty() {
        let none = MenuItem::with_id(
            app,
            "window-none",
            "No Project Windows",
            false,
            None::<&str>,
        )?;
        builder = builder.item(&none);
    } else {
        let mut sorted = entries;
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

    let toggle_sidebar = MenuItem::with_id(
        app,
        MENU_ID_VIEW_TOGGLE_SIDEBAR,
        "Toggle Sidebar",
        true,
        Some("CmdOrCtrl+B"),
    )?;
    let launch_agent = MenuItem::with_id(
        app,
        MENU_ID_VIEW_LAUNCH_AGENT,
        "Launch Agent...",
        true,
        None::<&str>,
    )?;
    let list_terminals = MenuItem::with_id(
        app,
        MENU_ID_VIEW_LIST_TERMINALS,
        "List Terminals",
        true,
        None::<&str>,
    )?;
    let terminal_diagnostics = MenuItem::with_id(
        app,
        MENU_ID_VIEW_TERMINAL_DIAGNOSTICS,
        "Terminal Diagnostics",
        true,
        None::<&str>,
    )?;

    builder = builder
        .separator()
        .item(&toggle_sidebar)
        .separator()
        .item(&launch_agent)
        .item(&list_terminals)
        .item(&terminal_diagnostics);

    builder.build()
}

fn collect_window_entries(app: &AppHandle<Wry>, state: &AppState) -> Vec<WindowMenuEntry> {
    let projects = match state.window_projects.lock() {
        Ok(m) => m.clone(),
        Err(_) => HashMap::new(),
    };

    if projects.is_empty() {
        return vec![];
    }

    // Determine focused window by scanning (stable API).
    let focused_label = app
        .webview_windows()
        .into_iter()
        .find_map(|(label, w)| w.is_focused().ok().and_then(|f| f.then_some(label)))
        .unwrap_or_else(|| "main".to_string());

    let mut raw: Vec<(String, String)> = Vec::new();
    for (label, path) in projects {
        let Some(window) = app.get_webview_window(&label) else {
            continue;
        };
        if window.is_visible().ok() == Some(false) {
            continue;
        }
        raw.push((label, path));
    }

    let displays = disambiguate_project_displays(&raw);
    raw.into_iter()
        .map(|(label, path)| WindowMenuEntry {
            focused: label == focused_label,
            display: displays
                .get(&label)
                .cloned()
                .unwrap_or_else(|| fallback_display_from_path(&path)),
            window_label: label,
            project_path: path,
        })
        .collect()
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
}
