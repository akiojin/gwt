use crate::persistence::{
    CanvasViewport, PersistedWindowState, PersistedWorkspaceState, WindowGeometry,
    WindowProcessStatus,
};
use crate::preset::WindowPreset;

#[derive(Debug, Clone)]
pub struct WorkspaceState {
    persisted: PersistedWorkspaceState,
}

impl WorkspaceState {
    pub fn from_persisted(persisted: PersistedWorkspaceState) -> Self {
        Self { persisted }
    }

    pub fn persisted(&self) -> &PersistedWorkspaceState {
        &self.persisted
    }

    pub fn window(&self, id: &str) -> Option<&PersistedWindowState> {
        self.persisted.windows.iter().find(|window| window.id == id)
    }

    pub fn set_status(&mut self, id: &str, status: WindowProcessStatus) -> bool {
        let Some(window) = self
            .persisted
            .windows
            .iter_mut()
            .find(|window| window.id == id)
        else {
            return false;
        };
        window.status = status;
        true
    }

    pub fn update_viewport(&mut self, viewport: CanvasViewport) {
        self.persisted.viewport = viewport;
    }

    pub fn focus_window(&mut self, id: &str) -> bool {
        let Some(window) = self
            .persisted
            .windows
            .iter_mut()
            .find(|window| window.id == id)
        else {
            return false;
        };
        window.z_index = self.persisted.next_z_index;
        self.persisted.next_z_index += 1;
        true
    }

    pub fn add_window(&mut self, preset: WindowPreset) -> PersistedWindowState {
        let count = self
            .persisted
            .windows
            .iter()
            .filter(|window| window.preset == preset)
            .count()
            + 1;
        let (width, height) = preset.default_size();
        let window = PersistedWindowState {
            id: format!("{}-{count}", preset.id_prefix()),
            title: preset.title().to_string(),
            preset,
            geometry: WindowGeometry {
                x: 120.0 + (self.persisted.windows.len() as f64 * 28.0),
                y: 96.0 + (self.persisted.windows.len() as f64 * 24.0),
                width,
                height,
            },
            z_index: self.persisted.next_z_index,
            status: if preset.requires_process() {
                WindowProcessStatus::Starting
            } else {
                WindowProcessStatus::Ready
            },
        };
        self.persisted.next_z_index += 1;
        self.persisted.windows.push(window.clone());
        window
    }

    pub fn update_geometry(&mut self, id: &str, geometry: WindowGeometry) -> bool {
        let Some(window) = self
            .persisted
            .windows
            .iter_mut()
            .find(|window| window.id == id)
        else {
            return false;
        };
        window.geometry = geometry;
        true
    }

    pub fn close_window(&mut self, id: &str) -> bool {
        let initial_len = self.persisted.windows.len();
        self.persisted.windows.retain(|window| window.id != id);
        self.persisted.windows.len() != initial_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{default_workspace_state, WindowProcessStatus};

    #[test]
    fn focusing_window_brings_it_to_front() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        assert!(workspace.focus_window("claude-1"));
        let claude = workspace
            .persisted()
            .windows
            .iter()
            .find(|window| window.id == "claude-1")
            .expect("claude window");
        assert_eq!(claude.z_index, 3);
        assert_eq!(workspace.persisted().next_z_index, 4);
    }

    #[test]
    fn adding_window_appends_shell_with_next_z_index() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::Shell);
        assert_eq!(window.title, "Shell");
        assert_eq!(window.preset, WindowPreset::Shell);
        assert_eq!(window.z_index, 3);
        assert_eq!(workspace.persisted().windows.len(), 3);
        assert_eq!(workspace.persisted().next_z_index, 4);
        assert_eq!(window.status, WindowProcessStatus::Starting);
    }

    #[test]
    fn adding_file_tree_window_marks_it_ready_without_process() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::FileTree);
        assert_eq!(window.title, "File Tree");
        assert_eq!(window.preset, WindowPreset::FileTree);
        assert_eq!(window.status, WindowProcessStatus::Ready);
    }

    #[test]
    fn adding_branches_window_marks_it_ready_without_process() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::Branches);
        assert_eq!(window.title, "Branches");
        assert_eq!(window.preset, WindowPreset::Branches);
        assert_eq!(window.status, WindowProcessStatus::Ready);
    }

    #[test]
    fn updating_geometry_replaces_window_geometry() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let updated = workspace.update_geometry(
            "codex-1",
            WindowGeometry {
                x: 120.0,
                y: 150.0,
                width: 900.0,
                height: 500.0,
            },
        );
        assert!(updated);
        let codex = workspace
            .persisted()
            .windows
            .iter()
            .find(|window| window.id == "codex-1")
            .expect("codex window");
        assert_eq!(codex.geometry.width, 900.0);
        assert_eq!(codex.geometry.height, 500.0);
    }

    #[test]
    fn closing_window_removes_it_from_workspace() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        assert!(workspace.close_window("codex-1"));
        assert_eq!(workspace.persisted().windows.len(), 1);
        assert!(workspace
            .persisted()
            .windows
            .iter()
            .all(|window| window.id != "codex-1"));
    }

    #[test]
    fn updating_viewport_replaces_canvas_transform_state() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        workspace.update_viewport(CanvasViewport {
            x: 180.0,
            y: -90.0,
            zoom: 1.35,
        });
        assert_eq!(
            workspace.persisted().viewport,
            CanvasViewport {
                x: 180.0,
                y: -90.0,
                zoom: 1.35
            }
        );
    }
}
