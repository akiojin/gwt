use crate::persistence::{
    CanvasViewport, PersistedWindowState, PersistedWorkspaceState, WindowGeometry,
    WindowProcessStatus,
};
use crate::preset::WindowPreset;
use crate::protocol::ArrangeMode;

const ARRANGE_PADDING: f64 = 24.0;
const STACK_OFFSET_X: f64 = 28.0;
const STACK_OFFSET_Y: f64 = 24.0;
const STACK_START_INSET: f64 = 48.0;
const MIN_WINDOW_WIDTH: f64 = 360.0;
const MIN_WINDOW_HEIGHT: f64 = 260.0;

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

    pub fn persistable_state(&self) -> PersistedWorkspaceState {
        let mut persisted = self.persisted.clone();
        persisted.windows.retain(|window| window.persist);
        persisted.next_z_index = persisted.windows.len() as u32 + 1;
        persisted
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

    pub fn arrange_windows(&mut self, mode: ArrangeMode, bounds: WindowGeometry) -> bool {
        if self.persisted.windows.is_empty() {
            return false;
        }

        self.persisted.windows.sort_by_key(|window| window.z_index);
        match mode {
            ArrangeMode::Tile => self.arrange_tile(bounds),
            ArrangeMode::Stack => self.arrange_stack(bounds),
        }
        self.reassign_z_indexes();
        true
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
        self.add_window_with_title(preset, preset.title(), true)
    }

    pub fn add_window_with_title(
        &mut self,
        preset: WindowPreset,
        title: impl Into<String>,
        persist: bool,
    ) -> PersistedWindowState {
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
            title: title.into(),
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
            persist,
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

    fn arrange_tile(&mut self, bounds: WindowGeometry) {
        let count = self.persisted.windows.len();
        let columns = (count as f64).sqrt().ceil() as usize;
        let rows = count.div_ceil(columns);
        let available_width = (bounds.width
            - ARRANGE_PADDING * 2.0
            - ARRANGE_PADDING * (columns.saturating_sub(1)) as f64)
            / columns as f64;
        let available_height = (bounds.height
            - ARRANGE_PADDING * 2.0
            - ARRANGE_PADDING * (rows.saturating_sub(1)) as f64)
            / rows as f64;
        let width = available_width.max(MIN_WINDOW_WIDTH);
        let height = available_height.max(MIN_WINDOW_HEIGHT);

        for (index, window) in self.persisted.windows.iter_mut().enumerate() {
            let column = index % columns;
            let row = index / columns;
            window.geometry = WindowGeometry {
                x: bounds.x + ARRANGE_PADDING + column as f64 * (width + ARRANGE_PADDING),
                y: bounds.y + ARRANGE_PADDING + row as f64 * (height + ARRANGE_PADDING),
                width,
                height,
            };
        }
    }

    fn arrange_stack(&mut self, bounds: WindowGeometry) {
        let available_width = (bounds.width - STACK_START_INSET * 2.0).max(MIN_WINDOW_WIDTH);
        let available_height = (bounds.height - STACK_START_INSET * 2.0).max(MIN_WINDOW_HEIGHT);

        for (index, window) in self.persisted.windows.iter_mut().enumerate() {
            window.geometry = WindowGeometry {
                x: bounds.x + STACK_START_INSET + index as f64 * STACK_OFFSET_X,
                y: bounds.y + STACK_START_INSET + index as f64 * STACK_OFFSET_Y,
                width: window
                    .geometry
                    .width
                    .min(available_width)
                    .max(MIN_WINDOW_WIDTH),
                height: window
                    .geometry
                    .height
                    .min(available_height)
                    .max(MIN_WINDOW_HEIGHT),
            };
        }
    }

    fn reassign_z_indexes(&mut self) {
        for (index, window) in self.persisted.windows.iter_mut().enumerate() {
            window.z_index = (index as u32) + 1;
        }
        self.persisted.next_z_index = self.persisted.windows.len() as u32 + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{default_workspace_state, WindowProcessStatus};

    fn arrange_bounds() -> WindowGeometry {
        WindowGeometry {
            x: 100.0,
            y: 40.0,
            width: 1000.0,
            height: 760.0,
        }
    }

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

    #[test]
    fn tile_arrangement_places_windows_on_a_grid() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        workspace.add_window(WindowPreset::FileTree);
        workspace.add_window(WindowPreset::Branches);

        assert!(workspace.arrange_windows(ArrangeMode::Tile, arrange_bounds()));

        let claude = workspace.window("claude-1").expect("claude");
        let codex = workspace.window("codex-1").expect("codex");
        let file_tree = workspace.window("file-tree-1").expect("file tree");
        let branches = workspace.window("branches-1").expect("branches");

        assert_eq!(claude.geometry.x, 124.0);
        assert_eq!(claude.geometry.y, 64.0);
        assert_eq!(codex.geometry.x, 612.0);
        assert_eq!(codex.geometry.y, 64.0);
        assert_eq!(file_tree.geometry.x, 124.0);
        assert_eq!(file_tree.geometry.y, 432.0);
        assert_eq!(branches.geometry.x, 612.0);
        assert_eq!(branches.geometry.y, 432.0);
        assert_eq!(branches.z_index, 4);
        assert_eq!(workspace.persisted().next_z_index, 5);
    }

    #[test]
    fn stack_arrangement_overlaps_windows_with_offsets() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        workspace.add_window(WindowPreset::FileTree);

        assert!(workspace.arrange_windows(ArrangeMode::Stack, arrange_bounds()));

        let claude = workspace.window("claude-1").expect("claude");
        let codex = workspace.window("codex-1").expect("codex");
        let file_tree = workspace.window("file-tree-1").expect("file tree");

        assert_eq!(claude.geometry.x, 148.0);
        assert_eq!(claude.geometry.y, 88.0);
        assert_eq!(codex.geometry.x, 176.0);
        assert_eq!(codex.geometry.y, 112.0);
        assert_eq!(file_tree.geometry.x, 204.0);
        assert_eq!(file_tree.geometry.y, 136.0);
        assert!(codex.geometry.x < claude.geometry.x + claude.geometry.width);
        assert!(codex.geometry.y < claude.geometry.y + claude.geometry.height);
        assert_eq!(workspace.persisted().next_z_index, 4);
    }
}
