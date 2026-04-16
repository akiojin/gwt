use crate::{
    persistence::{
        CanvasViewport, PersistedWindowState, PersistedWorkspaceState, WindowGeometry,
        WindowProcessStatus,
    },
    preset::WindowPreset,
    protocol::{ArrangeMode, FocusCycleDirection},
};

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
        let open_indices = self
            .persisted
            .windows
            .iter()
            .enumerate()
            .filter_map(|(index, window)| (!window.minimized).then_some(index))
            .collect::<Vec<_>>();
        if open_indices.is_empty() {
            return false;
        }
        match mode {
            ArrangeMode::Tile => self.arrange_tile(bounds, &open_indices),
            ArrangeMode::Stack => self.arrange_stack(bounds, &open_indices),
        }
        self.reassign_z_indexes();
        true
    }

    pub fn focus_window(&mut self, id: &str, bounds: Option<WindowGeometry>) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        self.bring_to_front(index);
        if let Some(b) = bounds {
            self.center_window(id, b);
        }
        true
    }

    pub fn cycle_focus(
        &mut self,
        direction: FocusCycleDirection,
        bounds: WindowGeometry,
    ) -> Option<String> {
        // Use stable creation order (array index), not z_index order.
        let eligible: Vec<usize> = self
            .persisted
            .windows
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.minimized)
            .map(|(i, _)| i)
            .collect();
        if eligible.is_empty() {
            return None;
        }
        if eligible.len() == 1 {
            let id = self.persisted.windows[eligible[0]].id.clone();
            self.center_window(&id, bounds);
            return Some(id);
        }

        // Find the currently focused window (highest z_index) within eligible.
        let current_idx = eligible
            .iter()
            .copied()
            .max_by_key(|&i| self.persisted.windows[i].z_index)?;
        let pos = eligible.iter().position(|&i| i == current_idx)?;

        let next_pos = match direction {
            FocusCycleDirection::Forward => (pos + 1) % eligible.len(),
            FocusCycleDirection::Backward => (pos + eligible.len() - 1) % eligible.len(),
        };
        let next_id = self.persisted.windows[eligible[next_pos]].id.clone();
        let _ = self.focus_window(&next_id, None);
        self.center_window(&next_id, bounds);
        Some(next_id)
    }

    pub fn add_window(
        &mut self,
        preset: WindowPreset,
        bounds: WindowGeometry,
    ) -> PersistedWindowState {
        self.add_window_with_title(preset, preset.title(), true, bounds)
    }

    pub fn add_window_with_title(
        &mut self,
        preset: WindowPreset,
        title: impl Into<String>,
        persist: bool,
        bounds: WindowGeometry,
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
                x: bounds.x + (bounds.width - width) / 2.0,
                y: bounds.y + (bounds.height - height) / 2.0,
                width,
                height,
            },
            z_index: self.persisted.next_z_index,
            status: if preset.requires_process() {
                WindowProcessStatus::Starting
            } else {
                WindowProcessStatus::Ready
            },
            minimized: false,
            maximized: false,
            pre_maximize_geometry: None,
            persist,
        };
        self.persisted.next_z_index += 1;
        self.persisted.windows.push(window.clone());
        window
    }

    pub fn maximize_window(&mut self, id: &str, bounds: WindowGeometry) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        let window = &mut self.persisted.windows[index];
        if window.maximized {
            if let Some(geometry) = window.pre_maximize_geometry.take() {
                window.geometry = geometry;
            }
            window.maximized = false;
            window.minimized = false;
        } else {
            window.pre_maximize_geometry = Some(window.geometry.clone());
            window.geometry = WindowGeometry {
                x: bounds.x + ARRANGE_PADDING,
                y: bounds.y + ARRANGE_PADDING,
                width: (bounds.width - ARRANGE_PADDING * 2.0).max(0.0),
                height: (bounds.height - ARRANGE_PADDING * 2.0).max(0.0),
            };
            window.minimized = false;
            window.maximized = true;
        }
        self.bring_to_front(index);
        true
    }

    pub fn minimize_window(&mut self, id: &str) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        let window = &mut self.persisted.windows[index];
        if window.minimized {
            window.minimized = false;
        } else {
            if window.maximized {
                if let Some(geometry) = window.pre_maximize_geometry.take() {
                    window.geometry = geometry;
                }
                window.maximized = false;
            }
            window.minimized = true;
        }
        self.bring_to_front(index);
        true
    }

    pub fn restore_window(&mut self, id: &str) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        let window = &mut self.persisted.windows[index];
        if window.maximized {
            if let Some(geometry) = window.pre_maximize_geometry.take() {
                window.geometry = geometry;
            }
            window.maximized = false;
            window.minimized = false;
        } else if window.minimized {
            window.minimized = false;
        } else {
            return false;
        }
        self.bring_to_front(index);
        true
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

    fn arrange_tile(&mut self, bounds: WindowGeometry, open_indices: &[usize]) {
        let count = open_indices.len();
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

        for (index, window_index) in open_indices.iter().enumerate() {
            let window = &mut self.persisted.windows[*window_index];
            let column = index % columns;
            let row = index / columns;
            window.geometry = WindowGeometry {
                x: bounds.x + ARRANGE_PADDING + column as f64 * (width + ARRANGE_PADDING),
                y: bounds.y + ARRANGE_PADDING + row as f64 * (height + ARRANGE_PADDING),
                width,
                height,
            };
            window.maximized = false;
            window.pre_maximize_geometry = None;
        }
    }

    fn arrange_stack(&mut self, bounds: WindowGeometry, open_indices: &[usize]) {
        let available_width = (bounds.width - STACK_START_INSET * 2.0).max(MIN_WINDOW_WIDTH);
        let available_height = (bounds.height - STACK_START_INSET * 2.0).max(MIN_WINDOW_HEIGHT);

        for (index, window_index) in open_indices.iter().enumerate() {
            let window = &mut self.persisted.windows[*window_index];
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
            window.maximized = false;
            window.pre_maximize_geometry = None;
        }
    }

    fn reassign_z_indexes(&mut self) {
        for (index, window) in self.persisted.windows.iter_mut().enumerate() {
            window.z_index = (index as u32) + 1;
        }
        self.persisted.next_z_index = self.persisted.windows.len() as u32 + 1;
    }

    fn center_window(&mut self, id: &str, bounds: WindowGeometry) -> bool {
        let Some(window) = self.persisted.windows.iter().find(|window| window.id == id) else {
            return false;
        };

        let zoom = self.persisted.viewport.zoom;
        let window_center_x = window.geometry.x + window.geometry.width / 2.0;
        let window_center_y = window.geometry.y + window.geometry.height / 2.0;
        self.persisted.viewport.x = bounds.width * zoom / 2.0 - window_center_x * zoom;
        self.persisted.viewport.y = bounds.height * zoom / 2.0 - window_center_y * zoom;
        true
    }

    fn window_index(&self, id: &str) -> Option<usize> {
        self.persisted
            .windows
            .iter()
            .position(|window| window.id == id)
    }

    fn bring_to_front(&mut self, index: usize) {
        if let Some(window) = self.persisted.windows.get_mut(index) {
            window.z_index = self.persisted.next_z_index;
            self.persisted.next_z_index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        persistence::{default_workspace_state, WindowProcessStatus},
        protocol::FocusCycleDirection,
    };

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
        assert!(workspace.focus_window("claude-1", None));
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
        let window = workspace.add_window(WindowPreset::Shell, arrange_bounds());
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
        let window = workspace.add_window(WindowPreset::FileTree, arrange_bounds());
        assert_eq!(window.title, "File Tree");
        assert_eq!(window.preset, WindowPreset::FileTree);
        assert_eq!(window.status, WindowProcessStatus::Ready);
    }

    #[test]
    fn adding_branches_window_marks_it_ready_without_process() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::Branches, arrange_bounds());
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
        workspace.add_window(WindowPreset::FileTree, arrange_bounds());
        workspace.add_window(WindowPreset::Branches, arrange_bounds());

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
        workspace.add_window(WindowPreset::FileTree, arrange_bounds());

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

    #[test]
    fn cycling_focus_forward_brings_next_window_to_front_and_centers_it() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        workspace.add_window(WindowPreset::Shell, arrange_bounds());
        // Array order: [claude-1(z=1), codex-1(z=2), shell-1(z=3)]
        // Current focus: shell-1 (highest z). Forward wraps to claude-1.

        let focused = workspace
            .cycle_focus(
                FocusCycleDirection::Forward,
                WindowGeometry {
                    x: 0.0,
                    y: 0.0,
                    width: 1200.0,
                    height: 800.0,
                },
            )
            .expect("focused window");

        assert_eq!(focused, "claude-1");
        assert_eq!(workspace.window("claude-1").expect("claude").z_index, 4);
        assert_eq!(workspace.persisted().next_z_index, 5);
    }

    #[test]
    fn cycling_focus_backward_wraps_and_preserves_zoom_when_centering() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        workspace.add_window(WindowPreset::Shell, arrange_bounds());
        // Array order: [claude-1(z=1), codex-1(z=2), shell-1(z=3)]
        // Current focus: shell-1 (highest z). Backward goes to codex-1.
        workspace.update_viewport(CanvasViewport {
            x: 12.0,
            y: -8.0,
            zoom: 1.25,
        });

        let focused = workspace
            .cycle_focus(
                FocusCycleDirection::Backward,
                WindowGeometry {
                    x: 0.0,
                    y: 0.0,
                    width: 1000.0,
                    height: 800.0,
                },
            )
            .expect("focused window");

        assert_eq!(focused, "codex-1");
        assert_eq!(workspace.window("codex-1").expect("codex").z_index, 4);
        assert_eq!(workspace.persisted().next_z_index, 5);
    }

    #[test]
    fn maximizing_window_toggles_between_viewport_bounds_and_original_geometry() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let original = workspace
            .window("claude-1")
            .expect("claude")
            .geometry
            .clone();

        assert!(workspace.maximize_window("claude-1", arrange_bounds()));

        let maximized = workspace.window("claude-1").expect("claude");
        assert!(maximized.maximized);
        assert!(!maximized.minimized);
        assert_eq!(maximized.pre_maximize_geometry, Some(original.clone()));
        assert_eq!(
            maximized.geometry,
            WindowGeometry {
                x: 124.0,
                y: 64.0,
                width: 952.0,
                height: 712.0,
            }
        );

        assert!(workspace.maximize_window("claude-1", arrange_bounds()));

        let restored = workspace.window("claude-1").expect("claude");
        assert!(!restored.maximized);
        assert!(!restored.minimized);
        assert_eq!(restored.pre_maximize_geometry, None);
        assert_eq!(restored.geometry, original);
    }

    #[test]
    fn minimizing_window_toggles_and_preserves_normal_geometry() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let original = workspace
            .window("claude-1")
            .expect("claude")
            .geometry
            .clone();

        assert!(workspace.minimize_window("claude-1"));
        assert!(workspace.window("claude-1").expect("claude").minimized);
        assert_eq!(
            workspace.window("claude-1").expect("claude").geometry,
            original
        );

        assert!(workspace.minimize_window("claude-1"));
        assert!(!workspace.window("claude-1").expect("claude").minimized);
        assert_eq!(
            workspace.window("claude-1").expect("claude").geometry,
            original
        );
    }

    #[test]
    fn restoring_window_clears_maximized_and_minimized_states() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let original = workspace
            .window("claude-1")
            .expect("claude")
            .geometry
            .clone();

        assert!(workspace.maximize_window("claude-1", arrange_bounds()));
        assert!(workspace.restore_window("claude-1"));
        let restored = workspace.window("claude-1").expect("claude");
        assert_eq!(restored.geometry, original);
        assert!(!restored.maximized);
        assert!(!restored.minimized);

        assert!(workspace.minimize_window("claude-1"));
        assert!(workspace.restore_window("claude-1"));
        assert!(!workspace.window("claude-1").expect("claude").minimized);
        assert_eq!(
            workspace.window("claude-1").expect("claude").geometry,
            original
        );
    }

    #[test]
    fn minimizing_maximized_window_restores_original_geometry_before_collapsing() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let original = workspace
            .window("claude-1")
            .expect("claude")
            .geometry
            .clone();

        assert!(workspace.maximize_window("claude-1", arrange_bounds()));
        assert!(workspace.minimize_window("claude-1"));

        let minimized = workspace.window("claude-1").expect("claude");
        assert!(minimized.minimized);
        assert!(!minimized.maximized);
        assert_eq!(minimized.geometry, original);
        assert_eq!(minimized.pre_maximize_geometry, None);

        assert!(workspace.restore_window("claude-1"));
        let restored = workspace.window("claude-1").expect("claude");
        assert_eq!(restored.geometry, original);
        assert!(!restored.minimized);
        assert!(!restored.maximized);
    }

    #[test]
    fn cycling_focus_skips_minimized_windows() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        workspace.add_window(WindowPreset::Shell, arrange_bounds());
        assert!(workspace.minimize_window("codex-1"));

        let focused = workspace
            .cycle_focus(
                FocusCycleDirection::Forward,
                WindowGeometry {
                    x: 0.0,
                    y: 0.0,
                    width: 1200.0,
                    height: 800.0,
                },
            )
            .expect("focused window");

        assert_eq!(focused, "claude-1");
        assert_eq!(workspace.window("claude-1").expect("claude").z_index, 5);
    }

    #[test]
    fn arranging_windows_skips_minimized_windows() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        workspace.add_window(WindowPreset::Shell, arrange_bounds());
        let minimized_geometry = workspace.window("codex-1").expect("codex").geometry.clone();
        assert!(workspace.minimize_window("codex-1"));

        assert!(workspace.arrange_windows(ArrangeMode::Tile, arrange_bounds()));

        let claude = workspace.window("claude-1").expect("claude");
        let shell = workspace.window("shell-1").expect("shell");
        let codex = workspace.window("codex-1").expect("codex");

        assert_eq!(claude.geometry.x, 124.0);
        assert_eq!(claude.geometry.y, 64.0);
        assert_eq!(shell.geometry.x, 612.0);
        assert_eq!(shell.geometry.y, 64.0);
        assert!(codex.minimized);
        assert_eq!(codex.geometry, minimized_geometry);
    }
}
