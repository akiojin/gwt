use std::path::{Path, PathBuf};

use gwt_agent::AgentColor;
use serde::{Deserialize, Serialize};

use crate::preset::WindowPreset;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowState {
    #[serde(alias = "starting", alias = "ready")]
    Running,
    Waiting,
    #[serde(alias = "exited")]
    Stopped,
    Error,
}

pub type WindowProcessStatus = WindowState;

impl WindowState {
    #[allow(non_upper_case_globals)]
    pub const Starting: Self = Self::Running;
    #[allow(non_upper_case_globals)]
    pub const Ready: Self = Self::Running;
    #[allow(non_upper_case_globals)]
    pub const Exited: Self = Self::Stopped;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedWindowState {
    pub id: String,
    pub title: String,
    pub preset: WindowPreset,
    pub geometry: WindowGeometry,
    pub z_index: u32,
    pub status: WindowState,
    #[serde(default)]
    pub minimized: bool,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub pre_maximize_geometry: Option<WindowGeometry>,
    #[serde(default = "default_persist_window")]
    pub persist: bool,
    /// Identifier of the agent occupying this window. Persisted across
    /// restarts so SPEC #2133 の色付けが復元される。`preset.Agent` の
    /// window では launch wizard 起動時に設定される。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Wire-only color hint sent to the WebView. 送信直前に backend が
    /// [`agent_id`] (または `preset` のフォールバック) から計算する。
    /// disk には書かない (`skip_deserializing` + skip_serializing_if で
    /// 読み書き両方向に漏らさない)。SPEC #2133 FR-008.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub agent_color: Option<AgentColor>,
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
pub struct PersistedSessionTabState {
    pub id: String,
    pub title: String,
    pub project_root: PathBuf,
    pub kind: ProjectKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedSessionState {
    #[serde(default)]
    pub tabs: Vec<PersistedSessionTabState>,
    pub active_tab_id: Option<String>,
    #[serde(default)]
    pub recent_projects: Vec<RecentProjectEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LegacyPersistedProjectTabState {
    pub id: String,
    pub title: String,
    pub project_root: PathBuf,
    pub kind: ProjectKind,
    pub workspace: PersistedWorkspaceState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LegacyPersistedAppState {
    #[serde(default)]
    pub tabs: Vec<LegacyPersistedProjectTabState>,
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
                status: WindowState::Running,
                minimized: false,
                maximized: false,
                pre_maximize_geometry: None,
                persist: true,
                agent_id: None,
                agent_color: None,
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
                status: WindowState::Running,
                minimized: false,
                maximized: false,
                pre_maximize_geometry: None,
                persist: true,
                agent_id: None,
                agent_color: None,
            },
        ],
        next_z_index: 3,
    }
}

pub fn default_session_state() -> PersistedSessionState {
    PersistedSessionState {
        tabs: Vec::new(),
        active_tab_id: None,
        recent_projects: Vec::new(),
    }
}

pub fn pause_process_windows_for_restore(state: &mut PersistedWorkspaceState) {
    for window in &mut state.windows {
        if window.preset.requires_process() {
            window.status = WindowState::Stopped;
        }
    }
}

fn default_persist_window() -> bool {
    true
}

pub fn legacy_workspace_state_path() -> PathBuf {
    gwt_core::paths::gwt_home()
        .join("poc")
        .join("terminal")
        .join("workspace.json")
}

pub fn workspace_state_path(project_root: &Path) -> PathBuf {
    let repo_hash = gwt_core::paths::project_scope_hash(project_root);
    gwt_core::paths::gwt_project_dir(&repo_hash).join("workspace.json")
}

pub fn load_session_state(path: &Path) -> std::io::Result<PersistedSessionState> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(default_session_state());
        }
        Err(error) => return Err(error),
    };
    serde_json::from_str(&content).map_err(Into::into)
}

pub fn save_session_state(path: &Path, state: &PersistedSessionState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        gwt_core::paths::ensure_dir(parent)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
    }
    let content = serde_json::to_string_pretty(state)?;
    std::fs::write(path, content)?;
    Ok(())
}

pub fn load_workspace_state(path: &Path) -> std::io::Result<PersistedWorkspaceState> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(empty_workspace_state());
        }
        Err(error) => return Err(error),
    };
    serde_json::from_str(&content).map_err(Into::into)
}

pub fn load_restored_workspace_state(
    project_root: &Path,
) -> std::io::Result<PersistedWorkspaceState> {
    let mut workspace = load_workspace_state(&workspace_state_path(project_root))?;
    pause_process_windows_for_restore(&mut workspace);
    Ok(workspace)
}

pub fn save_workspace_state(path: &Path, state: &PersistedWorkspaceState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        gwt_core::paths::ensure_dir(parent)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
    }
    let content = serde_json::to_string_pretty(state)?;
    std::fs::write(path, content)?;
    Ok(())
}

pub fn migrate_legacy_workspace_state(
    legacy_path: &Path,
    session_path: &Path,
    fallback_project_root: &Path,
    fallback_kind: ProjectKind,
) -> std::io::Result<()> {
    if session_path.exists() || !legacy_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(legacy_path)?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    let (session_state, workspaces) = if value.get("tabs").is_some() {
        let legacy: LegacyPersistedAppState = serde_json::from_value(value)?;
        (
            PersistedSessionState {
                tabs: legacy
                    .tabs
                    .iter()
                    .map(|tab| PersistedSessionTabState {
                        id: tab.id.clone(),
                        title: tab.title.clone(),
                        project_root: tab.project_root.clone(),
                        kind: tab.kind,
                    })
                    .collect(),
                active_tab_id: legacy.active_tab_id,
                recent_projects: legacy.recent_projects,
            },
            legacy
                .tabs
                .into_iter()
                .map(|tab| (tab.project_root, tab.workspace))
                .collect::<Vec<_>>(),
        )
    } else {
        let workspace: PersistedWorkspaceState = serde_json::from_value(value)?;
        let title = project_title_from_path(fallback_project_root);
        (
            PersistedSessionState {
                tabs: vec![PersistedSessionTabState {
                    id: "project-1".to_string(),
                    title: title.clone(),
                    project_root: fallback_project_root.to_path_buf(),
                    kind: fallback_kind,
                }],
                active_tab_id: Some("project-1".to_string()),
                recent_projects: vec![RecentProjectEntry {
                    path: fallback_project_root.to_path_buf(),
                    title,
                    kind: fallback_kind,
                }],
            },
            vec![(fallback_project_root.to_path_buf(), workspace)],
        )
    };

    for (project_root, workspace) in workspaces {
        let path = workspace_state_path(&project_root);
        if path.exists() {
            continue;
        }
        save_workspace_state(&path, &workspace)?;
    }

    save_session_state(session_path, &session_state)?;
    std::fs::remove_file(legacy_path)?;
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
    use tempfile::tempdir;

    use super::*;

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
    fn load_session_state_defaults_to_empty_state_for_missing_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.json");

        let loaded = load_session_state(&path).expect("load");
        assert_eq!(loaded, default_session_state());
    }

    #[test]
    fn save_and_load_session_state_round_trip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.json");
        let project_root = dir.path().join("demo");
        let state = PersistedSessionState {
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
                PersistedSessionTabState {
                    id: "project-1".to_string(),
                    title: "demo".to_string(),
                    project_root,
                    kind: ProjectKind::Git,
                },
                PersistedSessionTabState {
                    id: "project-2".to_string(),
                    title: "notes".to_string(),
                    project_root: dir.path().join("notes"),
                    kind: ProjectKind::NonRepo,
                },
            ],
        };

        save_session_state(&path, &state).expect("save should succeed");
        let loaded = load_session_state(&path).expect("load");
        assert_eq!(loaded, state);
    }

    #[test]
    fn load_workspace_state_defaults_to_empty_state_for_missing_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("workspace.json");

        let loaded = load_workspace_state(&path).expect("load");
        assert_eq!(loaded, empty_workspace_state());
    }

    #[test]
    fn save_and_load_workspace_state_round_trip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("workspace.json");
        let state = PersistedWorkspaceState {
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
                    agent_id: None,
                    agent_color: None,
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
                    status: WindowState::Running,
                    minimized: true,
                    maximized: false,
                    pre_maximize_geometry: None,
                    persist: true,
                    agent_id: None,
                    agent_color: None,
                },
            ],
            next_z_index: 6,
        };

        save_workspace_state(&path, &state).expect("save should succeed");
        let loaded = load_workspace_state(&path).expect("load");
        assert_eq!(loaded, state);
    }

    #[test]
    fn load_workspace_state_accepts_legacy_workspace_payload_without_new_fields() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("workspace.json");
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

        let loaded = load_workspace_state(&path).expect("legacy load");
        assert_eq!(loaded.viewport, default_canvas_viewport());
        assert_eq!(loaded.next_z_index, 2);
        assert!(!loaded.windows[0].minimized);
        assert!(!loaded.windows[0].maximized);
        assert!(loaded.windows[0].pre_maximize_geometry.is_none());
        // SPEC #2133: legacy window payloads predate agent_id / agent_color.
        // serde `default` は無い値を None に初期化する。
        assert!(loaded.windows[0].agent_id.is_none());
        assert!(loaded.windows[0].agent_color.is_none());
    }

    #[test]
    fn persisted_window_state_accepts_new_agent_id_field() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("workspace.json");
        std::fs::write(
            &path,
            r#"{
  "windows": [
    {
      "id": "claude-1",
      "title": "Claude",
      "preset": "claude",
      "geometry": { "x": 0.0, "y": 0.0, "width": 640.0, "height": 420.0 },
      "z_index": 1,
      "status": "ready",
      "persist": true,
      "agent_id": "claude"
    }
  ],
  "next_z_index": 2
}"#,
        )
        .expect("write new-format workspace");

        let loaded = load_workspace_state(&path).expect("load");
        assert_eq!(loaded.windows[0].agent_id.as_deref(), Some("claude"));
        // agent_color は wire 専用なので disk 読み込みでは常に None
        assert!(loaded.windows[0].agent_color.is_none());
    }

    #[test]
    fn persisted_window_state_serializes_agent_color_without_persisting() {
        // wire serialize では agent_color を含むが、deserialize は無視する。
        // disk 読み書きの round-trip では agent_color 情報が落ちることを確認。
        let original = PersistedWindowState {
            id: "w-1".into(),
            title: "Claude".into(),
            preset: WindowPreset::Claude,
            geometry: WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 200.0,
            },
            z_index: 1,
            status: WindowProcessStatus::Running,
            minimized: false,
            maximized: false,
            pre_maximize_geometry: None,
            persist: true,
            agent_id: Some("claude".into()),
            agent_color: Some(AgentColor::Yellow),
        };
        let json = serde_json::to_string(&original).expect("serialize");
        assert!(
            json.contains("\"agent_id\":\"claude\""),
            "agent_id should be serialized: {json}"
        );
        assert!(
            json.contains("\"agent_color\":\"yellow\""),
            "agent_color should be serialized as snake_case: {json}"
        );

        let parsed: PersistedWindowState = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed.agent_id.as_deref(), Some("claude"));
        assert!(
            parsed.agent_color.is_none(),
            "agent_color must be wire-only: skip_deserializing drops it"
        );
    }

    #[test]
    fn workspace_state_path_uses_project_scoped_storage() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempdir().expect("tempdir");
        let path = workspace_state_path(dir.path());
        let hash = gwt_core::paths::project_scope_hash(dir.path());
        assert!(path.ends_with(format!("projects/{}/workspace.json", hash.as_str())));
    }

    #[test]
    fn load_restored_workspace_state_pauses_process_windows() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempdir().expect("tempdir");
        let project_root = dir.path().join("project");
        std::fs::create_dir_all(&project_root).expect("project dir");
        let state = PersistedWorkspaceState {
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
                    agent_id: None,
                    agent_color: None,
                },
                PersistedWindowState {
                    id: "file-tree-1".to_string(),
                    title: "Files".to_string(),
                    preset: WindowPreset::FileTree,
                    geometry: WindowGeometry {
                        x: 0.0,
                        y: 0.0,
                        width: 400.0,
                        height: 500.0,
                    },
                    z_index: 2,
                    status: WindowState::Running,
                    minimized: false,
                    maximized: false,
                    pre_maximize_geometry: None,
                    persist: true,
                    agent_id: None,
                    agent_color: None,
                },
            ],
            next_z_index: 3,
        };
        save_workspace_state(&workspace_state_path(&project_root), &state).expect("save");

        let restored = load_restored_workspace_state(&project_root).expect("restore");
        assert_eq!(restored.windows[0].status, WindowState::Stopped);
        assert_eq!(restored.windows[1].status, WindowState::Running);
    }

    #[test]
    fn migrate_legacy_app_state_splits_session_and_project_workspaces() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempdir().expect("tempdir");
        let legacy_path = dir.path().join("legacy-workspace.json");
        let session_path = dir.path().join("session.json");
        let project_one = dir.path().join("project-one");
        let project_two = dir.path().join("project-two");
        let project_one_json =
            serde_json::to_string(&project_one.display().to_string()).expect("project one json");
        let project_two_json =
            serde_json::to_string(&project_two.display().to_string()).expect("project two json");
        std::fs::create_dir_all(&project_one).expect("project one dir");
        std::fs::create_dir_all(&project_two).expect("project two dir");
        std::fs::write(
            &legacy_path,
            format!(
                r#"{{
  "tabs": [
    {{
      "id": "project-1",
      "title": "project-one",
      "project_root": {},
      "kind": "git",
      "workspace": {{
        "viewport": {{ "x": 12.0, "y": -8.0, "zoom": 1.1 }},
        "windows": [],
        "next_z_index": 3
      }}
    }},
    {{
      "id": "project-2",
      "title": "project-two",
      "project_root": {},
      "kind": "non_repo",
      "workspace": {{
        "viewport": {{ "x": 0.0, "y": 0.0, "zoom": 1.0 }},
        "windows": [],
        "next_z_index": 5
      }}
    }}
  ],
  "active_tab_id": "project-2",
  "recent_projects": [
    {{ "path": {}, "title": "project-two", "kind": "non_repo" }}
  ]
}}"#,
                project_one_json, project_two_json, project_two_json
            ),
        )
        .expect("legacy workspace write");

        migrate_legacy_workspace_state(&legacy_path, &session_path, &project_one, ProjectKind::Git)
            .expect("migrate");

        let session = load_session_state(&session_path).expect("session");
        assert_eq!(session.tabs.len(), 2);
        assert_eq!(session.active_tab_id.as_deref(), Some("project-2"));
        assert_eq!(session.recent_projects.len(), 1);

        let workspace_one = load_workspace_state(&workspace_state_path(&project_one)).expect("one");
        let workspace_two = load_workspace_state(&workspace_state_path(&project_two)).expect("two");
        assert_eq!(workspace_one.viewport.x, 12.0);
        assert_eq!(workspace_one.next_z_index, 3);
        assert_eq!(workspace_two.next_z_index, 5);
        assert!(!legacy_path.exists());
    }

    #[test]
    fn migrate_legacy_single_workspace_uses_fallback_project_target() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempdir().expect("tempdir");
        let legacy_path = dir.path().join("legacy-workspace.json");
        let session_path = dir.path().join("session.json");
        let project_root = dir.path().join("workspace");
        std::fs::create_dir_all(&project_root).expect("project dir");
        std::fs::write(
            &legacy_path,
            r#"{
  "windows": [],
  "next_z_index": 2
}"#,
        )
        .expect("legacy workspace write");

        migrate_legacy_workspace_state(
            &legacy_path,
            &session_path,
            &project_root,
            ProjectKind::NonRepo,
        )
        .expect("migrate");

        let session = load_session_state(&session_path).expect("session");
        assert_eq!(session.tabs.len(), 1);
        assert_eq!(session.tabs[0].project_root, project_root);
        assert_eq!(session.tabs[0].kind, ProjectKind::NonRepo);
        assert_eq!(session.tabs[0].title, "workspace");
        let workspace =
            load_workspace_state(&workspace_state_path(&project_root)).expect("workspace");
        assert_eq!(workspace.next_z_index, 2);
        assert!(!legacy_path.exists());
    }

    #[test]
    fn migrate_legacy_workspace_state_keeps_existing_new_workspace() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempdir().expect("tempdir");
        let legacy_path = dir.path().join("legacy-workspace.json");
        let session_path = dir.path().join("session.json");
        let project_root = dir.path().join("project");
        let project_root_json =
            serde_json::to_string(&project_root.display().to_string()).expect("project root json");
        std::fs::create_dir_all(&project_root).expect("project dir");
        std::fs::write(
            &legacy_path,
            format!(
                r#"{{
  "tabs": [
    {{
      "id": "project-1",
      "title": "project",
      "project_root": {},
      "kind": "git",
      "workspace": {{
        "viewport": {{ "x": 99.0, "y": 0.0, "zoom": 1.0 }},
        "windows": [],
        "next_z_index": 7
      }}
    }}
  ],
  "active_tab_id": "project-1",
  "recent_projects": []
}}"#,
                project_root_json
            ),
        )
        .expect("legacy workspace write");

        let existing = PersistedWorkspaceState {
            viewport: CanvasViewport {
                x: 10.0,
                y: 20.0,
                zoom: 1.0,
            },
            windows: Vec::new(),
            next_z_index: 3,
        };
        save_workspace_state(&workspace_state_path(&project_root), &existing).expect("existing");

        migrate_legacy_workspace_state(
            &legacy_path,
            &session_path,
            &project_root,
            ProjectKind::Git,
        )
        .expect("migrate");

        let workspace =
            load_workspace_state(&workspace_state_path(&project_root)).expect("workspace");
        assert_eq!(workspace, existing);
        assert!(!legacy_path.exists());
    }

    #[test]
    fn pause_process_windows_for_restore_marks_only_process_windows_exited() {
        let mut state = PersistedWorkspaceState {
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
                    agent_id: None,
                    agent_color: None,
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
                    status: WindowState::Running,
                    minimized: false,
                    maximized: false,
                    pre_maximize_geometry: None,
                    persist: true,
                    agent_id: None,
                    agent_color: None,
                },
            ],
            next_z_index: 3,
        };

        pause_process_windows_for_restore(&mut state);

        assert_eq!(state.windows[0].status, WindowState::Stopped);
        assert_eq!(state.windows[1].status, WindowState::Running);
    }

    #[test]
    fn window_state_round_trips_modern_variants_and_accepts_legacy_status_names() {
        let waiting = serde_json::from_str::<WindowState>(r#""waiting""#).expect("waiting");
        let running = serde_json::from_str::<WindowState>(r#""running""#).expect("running");
        let stopped = serde_json::from_str::<WindowState>(r#""stopped""#).expect("stopped");
        let error = serde_json::from_str::<WindowState>(r#""error""#).expect("error");

        assert_eq!(waiting, WindowState::Waiting);
        assert_eq!(running, WindowState::Running);
        assert_eq!(stopped, WindowState::Stopped);
        assert_eq!(error, WindowState::Error);

        let legacy_starting =
            serde_json::from_str::<WindowState>(r#""starting""#).expect("legacy starting");
        let legacy_ready = serde_json::from_str::<WindowState>(r#""ready""#).expect("legacy ready");
        let legacy_exited =
            serde_json::from_str::<WindowState>(r#""exited""#).expect("legacy exited");

        assert_eq!(legacy_starting, WindowState::Running);
        assert_eq!(legacy_ready, WindowState::Running);
        assert_eq!(legacy_exited, WindowState::Stopped);
        assert_eq!(
            serde_json::to_string(&WindowState::Waiting).expect("serialize"),
            r#""waiting""#
        );
    }
}
