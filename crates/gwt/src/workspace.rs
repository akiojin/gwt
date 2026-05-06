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

    /// Record the AgentId command name on the given window so later snapshots
    /// can populate `agent_color` correctly. SPEC #2133 FR-007 / シナリオ 1.
    pub fn set_agent_id(&mut self, id: &str, agent_id: impl Into<String>) -> bool {
        let Some(window) = self
            .persisted
            .windows
            .iter_mut()
            .find(|window| window.id == id)
        else {
            return false;
        };
        window.agent_id = Some(agent_id.into());
        true
    }

    pub fn set_purpose_title(&mut self, id: &str, title: Option<String>) -> bool {
        let Some(window) = self
            .persisted
            .windows
            .iter_mut()
            .find(|window| window.id == id)
        else {
            return false;
        };
        window.purpose_title = title.and_then(normalize_title);
        true
    }

    pub fn set_dynamic_title(&mut self, id: &str, title: Option<String>) -> bool {
        let Some(window) = self
            .persisted
            .windows
            .iter_mut()
            .find(|window| window.id == id)
        else {
            return false;
        };
        window.dynamic_title = title.and_then(normalize_title);
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
            ArrangeMode::Align => self.arrange_align(bounds, &open_indices),
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
        let (width, height) = preset.default_size();
        let center_x = bounds.x + (bounds.width - width) / 2.0;
        let center_y = bounds.y + (bounds.height - height) / 2.0;

        // Walk the cascade diagonal starting at the viewport center and pick
        // the first slot that no visible window already occupies. The search
        // is bounded by the visible window count so it always terminates, and
        // we never wrap back onto an occupied slot — honoring the user's
        // "重なりを避ける" requirement even past the first cascade ring.
        let visible_window_count = self
            .persisted
            .windows
            .iter()
            .filter(|w| !w.minimized && !w.maximized)
            .count();
        let mut step = 0usize;
        while step <= visible_window_count {
            let candidate_x = center_x + (step as f64) * STACK_OFFSET_X;
            let candidate_y = center_y + (step as f64) * STACK_OFFSET_Y;
            let occupied = self.persisted.windows.iter().any(|w| {
                !w.minimized
                    && !w.maximized
                    && (w.geometry.x - candidate_x).abs() < 1.0
                    && (w.geometry.y - candidate_y).abs() < 1.0
            });
            if !occupied {
                break;
            }
            step += 1;
        }
        let step = step as f64;

        let window = PersistedWindowState {
            id: self.next_window_id(preset),
            title: title.into(),
            preset,
            geometry: WindowGeometry {
                x: center_x + step * STACK_OFFSET_X,
                y: center_y + step * STACK_OFFSET_Y,
                width,
                height,
            },
            z_index: self.persisted.next_z_index,
            status: WindowProcessStatus::Running,
            minimized: false,
            maximized: false,
            pre_maximize_geometry: None,
            persist,
            purpose_title: None,
            dynamic_title: None,
            agent_id: None,
            agent_color: None,
            tab_group_id: None,
            tab_group_active: false,
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
        let group_id = self
            .window(id)
            .and_then(|window| window.tab_group_id.clone());
        self.persisted.windows.retain(|window| window.id != id);
        let changed = self.persisted.windows.len() != initial_len;
        if changed {
            if let Some(group_id) = group_id {
                self.normalize_group(&group_id);
            }
        }
        changed
    }

    pub fn dock_window_tab(&mut self, id: &str, target_id: &str) -> bool {
        if id == target_id {
            return false;
        }
        if self.window_index(id).is_none() {
            return false;
        }
        let Some(target_index) = self.window_index(target_id) else {
            return false;
        };
        let group_id = self.persisted.windows[target_index]
            .tab_group_id
            .clone()
            .unwrap_or_else(|| format!("group-{}", self.persisted.windows[target_index].id));
        let group_geometry = self.persisted.windows[target_index].geometry.clone();
        let next_z_index = self.persisted.next_z_index;
        self.persisted.next_z_index += 1;

        for window in &mut self.persisted.windows {
            if window.id == id
                || window.id == target_id
                || window.tab_group_id.as_deref() == Some(&group_id)
            {
                window.tab_group_id = Some(group_id.clone());
                window.tab_group_active = window.id == id;
                window.geometry = group_geometry.clone();
                window.minimized = false;
                window.maximized = false;
                window.pre_maximize_geometry = None;
                window.z_index = next_z_index;
            }
        }
        true
    }

    pub fn activate_window_tab(&mut self, id: &str) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        let Some(group_id) = self.persisted.windows[index].tab_group_id.clone() else {
            return self.focus_window(id, None);
        };
        let group_geometry = self.persisted.windows[index].geometry.clone();
        let next_z_index = self.persisted.next_z_index;
        self.persisted.next_z_index += 1;
        for window in &mut self.persisted.windows {
            if window.tab_group_id.as_deref() == Some(&group_id) {
                window.tab_group_active = window.id == id;
                window.geometry = group_geometry.clone();
                window.z_index = next_z_index;
            }
        }
        true
    }

    pub fn detach_window_tab(&mut self, id: &str, geometry: WindowGeometry) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        let Some(group_id) = self.persisted.windows[index].tab_group_id.clone() else {
            self.persisted.windows[index].geometry = geometry;
            self.bring_to_front(index);
            return true;
        };
        self.persisted.windows[index].tab_group_id = None;
        self.persisted.windows[index].tab_group_active = false;
        self.persisted.windows[index].geometry = geometry;
        self.persisted.windows[index].minimized = false;
        self.persisted.windows[index].maximized = false;
        self.persisted.windows[index].pre_maximize_geometry = None;
        self.bring_to_front(index);
        self.normalize_group(&group_id);
        true
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

    fn arrange_align(&mut self, bounds: WindowGeometry, open_indices: &[usize]) {
        let count = open_indices.len();
        let columns = (count as f64).sqrt().ceil() as usize;
        let rows = count.div_ceil(columns);
        let cell_width = ((bounds.width
            - ARRANGE_PADDING * 2.0
            - ARRANGE_PADDING * (columns.saturating_sub(1)) as f64)
            / columns as f64)
            .max(MIN_WINDOW_WIDTH);
        let cell_height = ((bounds.height
            - ARRANGE_PADDING * 2.0
            - ARRANGE_PADDING * (rows.saturating_sub(1)) as f64)
            / rows as f64)
            .max(MIN_WINDOW_HEIGHT);

        for (index, window_index) in open_indices.iter().enumerate() {
            let window = &mut self.persisted.windows[*window_index];
            let column = index % columns;
            let row = index / columns;
            if let Some(geometry) = window.pre_maximize_geometry.take() {
                window.geometry.width = geometry.width;
                window.geometry.height = geometry.height;
            }
            window.geometry.x =
                bounds.x + ARRANGE_PADDING + column as f64 * (cell_width + ARRANGE_PADDING);
            window.geometry.y =
                bounds.y + ARRANGE_PADDING + row as f64 * (cell_height + ARRANGE_PADDING);
            window.maximized = false;
        }
    }

    fn reassign_z_indexes(&mut self) {
        for (index, window) in self.persisted.windows.iter_mut().enumerate() {
            window.z_index = (index as u32) + 1;
        }
        self.persisted.next_z_index = self.persisted.windows.len() as u32 + 1;
    }

    fn next_window_id(&self, preset: WindowPreset) -> String {
        let prefix = preset.id_prefix();
        let next_suffix = self
            .persisted
            .windows
            .iter()
            .filter(|window| window.preset == preset)
            .filter_map(|window| {
                window
                    .id
                    .strip_prefix(prefix)
                    .and_then(|suffix| suffix.strip_prefix('-'))
                    .and_then(|suffix| suffix.parse::<u32>().ok())
            })
            .max()
            .unwrap_or(0)
            + 1;
        format!("{prefix}-{next_suffix}")
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

    fn normalize_group(&mut self, group_id: &str) {
        let group_indices = self
            .persisted
            .windows
            .iter()
            .enumerate()
            .filter_map(|(index, window)| {
                (window.tab_group_id.as_deref() == Some(group_id)).then_some(index)
            })
            .collect::<Vec<_>>();
        if group_indices.len() <= 1 {
            for index in group_indices {
                let window = &mut self.persisted.windows[index];
                window.tab_group_id = None;
                window.tab_group_active = false;
            }
            return;
        }
        if group_indices
            .iter()
            .any(|index| self.persisted.windows[*index].tab_group_active)
        {
            return;
        }
        if let Some(index) = group_indices.first() {
            self.persisted.windows[*index].tab_group_active = true;
        }
    }
}

fn normalize_title(title: String) -> Option<String> {
    let trimmed = title.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        persistence::{
            default_canvas_viewport, default_workspace_state, empty_workspace_state,
            WindowProcessStatus,
        },
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
        assert_eq!(window.status, WindowProcessStatus::Running);
    }

    #[test]
    fn adding_file_tree_window_marks_it_running_without_process() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::FileTree, arrange_bounds());
        assert_eq!(window.title, "File Tree");
        assert_eq!(window.preset, WindowPreset::FileTree);
        assert_eq!(window.status, WindowProcessStatus::Running);
    }

    #[test]
    fn adding_branches_window_marks_it_running_without_process() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::Branches, arrange_bounds());
        assert_eq!(window.title, "Branches");
        assert_eq!(window.preset, WindowPreset::Branches);
        assert_eq!(window.status, WindowProcessStatus::Running);
    }

    #[test]
    fn adding_window_centers_in_bounds() {
        let mut workspace = WorkspaceState::from_persisted(empty_workspace_state());
        let bounds = WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 920.0,
        };

        let window = workspace.add_window(WindowPreset::Shell, bounds);

        // Shell preset is 720x420 so the geometry must center inside bounds.
        assert_eq!(window.geometry.x, 360.0);
        assert_eq!(window.geometry.y, 250.0);
    }

    #[test]
    fn adding_multiple_windows_cascades_from_center() {
        let mut workspace = WorkspaceState::from_persisted(empty_workspace_state());
        let bounds = WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 920.0,
        };

        let first = workspace.add_window(WindowPreset::Shell, bounds.clone());
        let second = workspace.add_window(WindowPreset::Shell, bounds.clone());
        let third = workspace.add_window(WindowPreset::Shell, bounds);

        assert_eq!((first.geometry.x, first.geometry.y), (360.0, 250.0));
        assert_eq!((second.geometry.x, second.geometry.y), (388.0, 274.0));
        assert_eq!((third.geometry.x, third.geometry.y), (416.0, 298.0));
    }

    #[test]
    fn adding_window_keeps_cascading_past_eight_to_avoid_overlap() {
        let mut workspace = WorkspaceState::from_persisted(empty_workspace_state());
        let bounds = WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 920.0,
        };

        // Fill the first cascade ring with 8 windows (steps 0..7).
        for _ in 0..8 {
            workspace.add_window(WindowPreset::Shell, bounds.clone());
        }
        // The 9th launch must keep cascading (step 8) instead of collapsing
        // back onto the viewport center where window #1 already lives.
        let ninth = workspace.add_window(WindowPreset::Shell, bounds.clone());

        assert_eq!(
            (ninth.geometry.x, ninth.geometry.y),
            (360.0 + 8.0 * 28.0, 250.0 + 8.0 * 24.0),
        );
        // And the 10th continues from there without overlapping the 9th.
        let tenth = workspace.add_window(WindowPreset::Shell, bounds);
        assert_eq!(
            (tenth.geometry.x, tenth.geometry.y),
            (360.0 + 9.0 * 28.0, 250.0 + 9.0 * 24.0),
        );
    }

    #[test]
    fn adding_window_skips_minimized_collisions() {
        let mut workspace = WorkspaceState::from_persisted(empty_workspace_state());
        let bounds = WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 920.0,
        };

        let first = workspace.add_window(WindowPreset::Shell, bounds.clone());
        assert!(workspace.minimize_window(&first.id));

        // A minimized window must not block the cascade slot it was created in,
        // so the next launch lands back at the viewport center.
        let next = workspace.add_window(WindowPreset::Shell, bounds);
        assert_eq!((next.geometry.x, next.geometry.y), (360.0, 250.0));
    }

    #[test]
    fn adding_agent_window_uses_new_id_when_lower_suffix_was_closed() {
        let mut workspace = WorkspaceState::from_persisted(PersistedWorkspaceState {
            viewport: default_canvas_viewport(),
            windows: vec![PersistedWindowState {
                id: "agent-2".to_string(),
                title: "Agent".to_string(),
                preset: WindowPreset::Agent,
                geometry: WindowGeometry {
                    x: 80.0,
                    y: 64.0,
                    width: 720.0,
                    height: 420.0,
                },
                z_index: 1,
                status: WindowProcessStatus::Running,
                minimized: false,
                maximized: false,
                pre_maximize_geometry: None,
                persist: false,
                purpose_title: None,
                dynamic_title: None,
                agent_id: None,
                agent_color: None,
                tab_group_id: None,
                tab_group_active: false,
            }],
            next_z_index: 2,
        });

        let window = workspace.add_window(WindowPreset::Agent, arrange_bounds());

        assert_eq!(window.id, "agent-3");
        assert_eq!(workspace.persisted().windows.len(), 2);
        assert!(workspace.window("agent-2").is_some());
        assert!(workspace.window("agent-3").is_some());
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
    fn align_arrangement_places_windows_on_grid_without_resizing() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        workspace.add_window(WindowPreset::FileTree, arrange_bounds());

        let original = workspace
            .persisted()
            .windows
            .iter()
            .map(|window| {
                (
                    window.id.clone(),
                    window.geometry.width,
                    window.geometry.height,
                )
            })
            .collect::<Vec<_>>();

        assert!(workspace.arrange_windows(ArrangeMode::Align, arrange_bounds()));

        for (id, width, height) in original {
            let window = workspace.window(&id).expect("window");
            assert_eq!(
                window.geometry.width, width,
                "{id} width should be preserved"
            );
            assert_eq!(
                window.geometry.height, height,
                "{id} height should be preserved"
            );
        }

        let claude = workspace.window("claude-1").expect("claude");
        let codex = workspace.window("codex-1").expect("codex");
        let file_tree = workspace.window("file-tree-1").expect("file tree");

        assert_eq!(claude.geometry.x, 124.0);
        assert_eq!(claude.geometry.y, 64.0);
        assert_eq!(codex.geometry.x, 612.0);
        assert_eq!(codex.geometry.y, 64.0);
        assert_eq!(file_tree.geometry.x, 124.0);
        assert_eq!(file_tree.geometry.y, 432.0);
    }

    #[test]
    fn align_arrangement_restores_maximized_window_size_before_positioning() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let original = workspace
            .window("claude-1")
            .expect("claude")
            .geometry
            .clone();

        assert!(workspace.maximize_window("claude-1", arrange_bounds()));
        assert!(workspace.arrange_windows(ArrangeMode::Align, arrange_bounds()));

        let claude = workspace.window("claude-1").expect("claude");
        assert!(!claude.maximized);
        assert_eq!(claude.pre_maximize_geometry, None);
        assert_eq!(claude.geometry.width, original.width);
        assert_eq!(claude.geometry.height, original.height);
        assert_eq!(claude.geometry.x, 612.0);
        assert_eq!(claude.geometry.y, 64.0);
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

    #[test]
    fn docking_window_tabs_groups_windows_and_activates_dragged_tab() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        let codex_geometry = workspace.window("codex-1").expect("codex").geometry.clone();

        assert!(workspace.dock_window_tab("codex-1", "claude-1"));

        let claude = workspace.window("claude-1").expect("claude");
        let codex = workspace.window("codex-1").expect("codex");
        assert!(claude.tab_group_id.is_some());
        assert_eq!(claude.tab_group_id, codex.tab_group_id);
        assert!(!claude.tab_group_active);
        assert!(codex.tab_group_active);
        assert_eq!(
            codex.geometry,
            workspace.window("claude-1").expect("claude").geometry,
            "docked tab should adopt the host group geometry"
        );
        assert_ne!(codex.geometry, codex_geometry);
    }

    #[test]
    fn activating_window_tab_switches_active_marker_within_group() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));

        assert!(workspace.activate_window_tab("claude-1"));

        assert!(
            workspace
                .window("claude-1")
                .expect("claude")
                .tab_group_active
        );
        assert!(!workspace.window("codex-1").expect("codex").tab_group_active);
    }

    #[test]
    fn detaching_window_tab_restores_independent_floating_window() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));
        let detached_geometry = WindowGeometry {
            x: 240.0,
            y: 180.0,
            width: 640.0,
            height: 360.0,
        };

        assert!(workspace.detach_window_tab("codex-1", detached_geometry.clone()));

        let claude = workspace.window("claude-1").expect("claude");
        let codex = workspace.window("codex-1").expect("codex");
        assert!(claude.tab_group_id.is_none());
        assert!(!claude.tab_group_active);
        assert!(codex.tab_group_id.is_none());
        assert!(!codex.tab_group_active);
        assert_eq!(codex.geometry, detached_geometry);
    }

    #[test]
    fn closing_active_group_tab_promotes_another_tab() {
        let mut workspace = WorkspaceState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));

        assert!(workspace.close_window("codex-1"));

        let claude = workspace.window("claude-1").expect("claude");
        assert!(claude.tab_group_id.is_none());
        assert!(!claude.tab_group_active);
    }
}
