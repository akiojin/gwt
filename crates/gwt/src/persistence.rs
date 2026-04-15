use crate::preset::WindowPreset;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasViewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

pub fn default_canvas_viewport() -> CanvasViewport {
    CanvasViewport {
        x: 0.0,
        y: 0.0,
        zoom: 1.0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowProcessStatus {
    Starting,
    Running,
    Ready,
    Exited,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedWindowState {
    pub id: String,
    pub title: String,
    pub preset: WindowPreset,
    pub geometry: WindowGeometry,
    pub z_index: u32,
    pub status: WindowProcessStatus,
    #[serde(default)]
    pub minimized: bool,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub pre_maximize_geometry: Option<WindowGeometry>,
    #[serde(default = "default_persist_window")]
    pub persist: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedWorkspaceState {
    #[serde(default = "default_canvas_viewport")]
    pub viewport: CanvasViewport,
    pub windows: Vec<PersistedWindowState>,
    pub next_z_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    Git,
    Bare,
    NonRepo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProjectEntry {
    pub path: PathBuf,
    pub title: String,
    pub kind: ProjectKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedProjectTabState {
    pub id: String,
    pub title: String,
    pub project_root: PathBuf,
    pub kind: ProjectKind,
    pub workspace: PersistedWorkspaceState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedAppState {
    #[serde(default)]
    pub tabs: Vec<PersistedProjectTabState>,
    pub active_tab_id: Option<String>,
    #[serde(default)]
    pub recent_projects: Vec<RecentProjectEntry>,
}

pub fn empty_workspace_state() -> PersistedWorkspaceState {
    PersistedWorkspaceState {
        viewport: default_canvas_viewport(),
        windows: Vec::new(),
        next_z_index: 1,
    }
}

pub fn default_workspace_state() -> PersistedWorkspaceState {
    PersistedWorkspaceState {
        viewport: default_canvas_viewport(),
        windows: vec![
            PersistedWindowState {
                id: "claude-1".to_string(),
                title: "Claude".to_string(),
                preset: WindowPreset::Claude,
                geometry: WindowGeometry {
                    x: 80.0,
                    y: 64.0,
                    width: 720.0,
                    height: 420.0,
                },
                z_index: 1,
                status: WindowProcessStatus::Starting,
                minimized: false,
                maximized: false,
                pre_maximize_geometry: None,
                persist: true,
            },
            PersistedWindowState {
                id: "codex-1".to_string(),
                title: "Codex".to_string(),
                preset: WindowPreset::Codex,
                geometry: WindowGeometry {
                    x: 460.0,
                    y: 140.0,
                    width: 720.0,
                    height: 420.0,
                },
                z_index: 2,
                status: WindowProcessStatus::Starting,
                minimized: false,
                maximized: false,
                pre_maximize_geometry: None,
                persist: true,
            },
        ],
        next_z_index: 3,
    }
}

pub fn default_app_state() -> PersistedAppState {
    PersistedAppState {
        tabs: Vec::new(),
        active_tab_id: None,
        recent_projects: Vec::new(),
    }
}

pub fn pause_process_windows_for_restore(state: &mut PersistedAppState) {
    for tab in &mut state.tabs {
        for window in &mut tab.workspace.windows {
            if window.preset.requires_process() {
                window.status = WindowProcessStatus::Exited;
            }
        }
    }
}

fn default_persist_window() -> bool {
    true
}

pub fn workspace_state_path() -> PathBuf {
    gwt_core::paths::gwt_home()
        .join("poc")
        .join("terminal")
        .join("workspace.json")
}

pub fn load_app_state(
    path: &Path,
    legacy_project_root: &Path,
    legacy_kind: ProjectKind,
) -> std::io::Result<PersistedAppState> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(default_app_state());
        }
        Err(error) => return Err(error),
    };

    let value: serde_json::Value = serde_json::from_str(&content)?;
    if value.get("tabs").is_some() {
        return Ok(serde_json::from_value(value)?);
    }

    let workspace: PersistedWorkspaceState = serde_json::from_value(value)?;
    let title = project_title_from_path(legacy_project_root);
    Ok(PersistedAppState {
        active_tab_id: Some("project-1".to_string()),
        recent_projects: vec![RecentProjectEntry {
            path: legacy_project_root.to_path_buf(),
            title: title.clone(),
            kind: legacy_kind,
        }],
        tabs: vec![PersistedProjectTabState {
            id: "project-1".to_string(),
            title,
            project_root: legacy_project_root.to_path_buf(),
            kind: legacy_kind,
            workspace,
        }],
    })
}

pub fn save_app_state(path: &Path, state: &PersistedAppState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        gwt_core::paths::ensure_dir(parent)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
    }
    let content = serde_json::to_string_pretty(state)?;
    std::fs::write(path, content)?;
    Ok(())
}

pub fn project_title_from_path(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn empty_workspace_contains_no_windows() {
        let state = empty_workspace_state();
        assert_eq!(state.viewport, default_canvas_viewport());
        assert!(state.windows.is_empty());
        assert_eq!(state.next_z_index, 1);
    }

    #[test]
    fn default_workspace_contains_claude_and_codex_windows() {
        let state = default_workspace_state();
        assert_eq!(state.viewport, default_canvas_viewport());
        let titles: Vec<&str> = state
            .windows
            .iter()
            .map(|window| window.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Claude", "Codex"]);
        assert!(state.windows.iter().all(|window| !window.minimized));
        assert!(state.windows.iter().all(|window| !window.maximized));
        assert!(state
            .windows
            .iter()
            .all(|window| window.pre_maximize_geometry.is_none()));
        assert_eq!(state.next_z_index, 3);
    }

    #[test]
    fn load_app_state_defaults_to_empty_state_for_missing_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("workspace.json");

        let loaded =
            load_app_state(&path, &dir.path().join("workspace"), ProjectKind::Git).expect("load");
        assert_eq!(loaded, default_app_state());
    }

    #[test]
    fn save_and_load_app_state_round_trip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("workspace.json");
        let project_root = dir.path().join("demo");
        let state = PersistedAppState {
            active_tab_id: Some("project-2".to_string()),
            recent_projects: vec![
                RecentProjectEntry {
                    path: project_root.clone(),
                    title: "demo".to_string(),
                    kind: ProjectKind::Git,
                },
                RecentProjectEntry {
                    path: dir.path().join("notes"),
                    title: "notes".to_string(),
                    kind: ProjectKind::NonRepo,
                },
            ],
            tabs: vec![
                PersistedProjectTabState {
                    id: "project-1".to_string(),
                    title: "demo".to_string(),
                    project_root: project_root.clone(),
                    kind: ProjectKind::Git,
                    workspace: PersistedWorkspaceState {
                        viewport: CanvasViewport {
                            x: 120.0,
                            y: -48.0,
                            zoom: 1.4,
                        },
                        windows: vec![
                            PersistedWindowState {
                                id: "shell-1".to_string(),
                                title: "Shell".to_string(),
                                preset: WindowPreset::Shell,
                                geometry: WindowGeometry {
                                    x: 10.0,
                                    y: 20.0,
                                    width: 640.0,
                                    height: 420.0,
                                },
                                z_index: 4,
                                status: WindowProcessStatus::Running,
                                minimized: false,
                                maximized: true,
                                pre_maximize_geometry: Some(WindowGeometry {
                                    x: 48.0,
                                    y: 64.0,
                                    width: 720.0,
                                    height: 480.0,
                                }),
                                persist: true,
                            },
                            PersistedWindowState {
                                id: "branches-1".to_string(),
                                title: "Branches".to_string(),
                                preset: WindowPreset::Branches,
                                geometry: WindowGeometry {
                                    x: 36.0,
                                    y: 48.0,
                                    width: 540.0,
                                    height: 360.0,
                                },
                                z_index: 5,
                                status: WindowProcessStatus::Ready,
                                minimized: true,
                                maximized: false,
                                pre_maximize_geometry: None,
                                persist: true,
                            },
                        ],
                        next_z_index: 6,
                    },
                },
                PersistedProjectTabState {
                    id: "project-2".to_string(),
                    title: "notes".to_string(),
                    project_root: dir.path().join("notes"),
                    kind: ProjectKind::NonRepo,
                    workspace: empty_workspace_state(),
                },
            ],
        };

        save_app_state(&path, &state).expect("save should succeed");
        let loaded = load_app_state(&path, &project_root, ProjectKind::Git).expect("load");
        assert_eq!(loaded, state);
    }

    #[test]
    fn load_app_state_migrates_legacy_workspace_file_to_single_project_tab() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("workspace.json");
        let legacy_project_root = dir.path().join("workspace");
        std::fs::write(
            &path,
            r#"{
  "windows": [
    {
      "id": "shell-1",
      "title": "Shell",
      "preset": "shell",
      "geometry": { "x": 20.0, "y": 40.0, "width": 640.0, "height": 420.0 },
      "z_index": 1,
      "status": "ready",
      "persist": true
    }
  ],
  "next_z_index": 2
}"#,
        )
        .expect("legacy workspace write");

        let loaded =
            load_app_state(&path, &legacy_project_root, ProjectKind::Git).expect("legacy load");
        assert_eq!(loaded.active_tab_id.as_deref(), Some("project-1"));
        assert_eq!(loaded.tabs.len(), 1);
        assert_eq!(loaded.recent_projects.len(), 1);
        assert_eq!(loaded.tabs[0].project_root, legacy_project_root);
        assert_eq!(loaded.tabs[0].title, "workspace");
        assert_eq!(loaded.tabs[0].kind, ProjectKind::Git);
        assert_eq!(loaded.tabs[0].workspace.viewport, default_canvas_viewport());
        assert_eq!(loaded.tabs[0].workspace.next_z_index, 2);
        assert!(!loaded.tabs[0].workspace.windows[0].minimized);
        assert!(!loaded.tabs[0].workspace.windows[0].maximized);
        assert!(loaded.tabs[0].workspace.windows[0]
            .pre_maximize_geometry
            .is_none());
    }

    #[test]
    fn pause_process_windows_for_restore_marks_only_process_windows_exited() {
        let mut state = PersistedAppState {
            active_tab_id: Some("project-1".to_string()),
            recent_projects: Vec::new(),
            tabs: vec![PersistedProjectTabState {
                id: "project-1".to_string(),
                title: "demo".to_string(),
                project_root: PathBuf::from("/tmp/demo"),
                kind: ProjectKind::Git,
                workspace: PersistedWorkspaceState {
                    viewport: default_canvas_viewport(),
                    windows: vec![
                        PersistedWindowState {
                            id: "shell-1".to_string(),
                            title: "Shell".to_string(),
                            preset: WindowPreset::Shell,
                            geometry: WindowGeometry {
                                x: 0.0,
                                y: 0.0,
                                width: 640.0,
                                height: 420.0,
                            },
                            z_index: 1,
                            status: WindowProcessStatus::Running,
                            minimized: false,
                            maximized: false,
                            pre_maximize_geometry: None,
                            persist: true,
                        },
                        PersistedWindowState {
                            id: "branches-1".to_string(),
                            title: "Branches".to_string(),
                            preset: WindowPreset::Branches,
                            geometry: WindowGeometry {
                                x: 0.0,
                                y: 0.0,
                                width: 640.0,
                                height: 420.0,
                            },
                            z_index: 2,
                            status: WindowProcessStatus::Ready,
                            minimized: false,
                            maximized: false,
                            pre_maximize_geometry: None,
                            persist: true,
                        },
                    ],
                    next_z_index: 3,
                },
            }],
        };

        pause_process_windows_for_restore(&mut state);

        assert_eq!(
            state.tabs[0].workspace.windows[0].status,
            WindowProcessStatus::Exited
        );
        assert_eq!(
            state.tabs[0].workspace.windows[1].status,
            WindowProcessStatus::Ready
        );
    }
}
