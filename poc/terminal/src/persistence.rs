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

pub fn default_workspace_state() -> PersistedWorkspaceState {
    PersistedWorkspaceState {
        viewport: CanvasViewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        },
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
                persist: true,
            },
        ],
        next_z_index: 3,
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

pub fn load_workspace_state(path: &Path) -> std::io::Result<PersistedWorkspaceState> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(serde_json::from_str(&content)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(default_workspace_state()),
        Err(error) => Err(error),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_workspace_contains_claude_and_codex_windows() {
        let state = default_workspace_state();
        assert_eq!(
            state.viewport,
            CanvasViewport {
                x: 0.0,
                y: 0.0,
                zoom: 1.0
            }
        );
        let titles: Vec<&str> = state
            .windows
            .iter()
            .map(|window| window.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Claude", "Codex"]);
        assert_eq!(state.next_z_index, 3);
    }

    #[test]
    fn save_and_load_workspace_round_trip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("workspace.json");
        let state = PersistedWorkspaceState {
            viewport: CanvasViewport {
                x: 120.0,
                y: -48.0,
                zoom: 1.4,
            },
            windows: vec![PersistedWindowState {
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
                persist: true,
            }],
            next_z_index: 5,
        };

        save_workspace_state(&path, &state).expect("save should succeed");
        let loaded = load_workspace_state(&path).expect("load should succeed");
        assert_eq!(loaded, state);
    }

    #[test]
    fn load_workspace_state_defaults_viewport_for_legacy_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("workspace.json");
        std::fs::write(
            &path,
            r#"{
  "windows": [],
  "next_z_index": 3
}"#,
        )
        .expect("legacy workspace write");

        let loaded = load_workspace_state(&path).expect("legacy load should succeed");
        assert_eq!(loaded.viewport, default_canvas_viewport());
        assert_eq!(loaded.next_z_index, 3);
    }
}
