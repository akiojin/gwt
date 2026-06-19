use crate::{
    persistence::{
        AgentKanbanLane, CanvasViewport, PersistedWindowCanvasState, PersistedWindowState,
        WindowGeometry, WindowPlacement, WindowProcessStatus,
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
pub struct WindowCanvasState {
    persisted: PersistedWindowCanvasState,
}

impl WindowCanvasState {
    pub fn from_persisted(persisted: PersistedWindowCanvasState) -> Self {
        Self { persisted }
    }

    pub fn persisted(&self) -> &PersistedWindowCanvasState {
        &self.persisted
    }

    pub fn persistable_state(&self) -> PersistedWindowCanvasState {
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

    pub fn set_session_id(&mut self, id: &str, session_id: Option<String>) -> bool {
        let Some(window) = self
            .persisted
            .windows
            .iter_mut()
            .find(|window| window.id == id)
        else {
            return false;
        };
        window.session_id = session_id;
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

    /// Sets `dynamic_title` for the named window. Returns `true` only when
    /// the stored value actually changed (after normalization) so callers
    /// can decide whether to emit a broadcast. SPEC-2359 US-26: gating on
    /// real diff avoids forcing frontend re-renders for no-op projection
    /// reloads (e.g. repeated `workspace.update` with the same title).
    pub fn set_dynamic_title(&mut self, id: &str, title: Option<String>) -> bool {
        let Some(window) = self
            .persisted
            .windows
            .iter_mut()
            .find(|window| window.id == id)
        else {
            return false;
        };
        let new_title = title.and_then(normalize_title);
        if window.dynamic_title == new_title {
            return false;
        }
        window.dynamic_title = new_title;
        true
    }

    /// Sets `dynamic_title` and `dynamic_title_detail` for the named window.
    /// Returns `true` only when either field actually changed (after
    /// normalization), so callers can gate broadcasts on a real value diff.
    /// SPEC-2359 US-26.
    pub fn set_dynamic_title_with_detail(
        &mut self,
        id: &str,
        title: Option<String>,
        detail: Option<String>,
    ) -> bool {
        let Some(window) = self
            .persisted
            .windows
            .iter_mut()
            .find(|window| window.id == id)
        else {
            return false;
        };
        let new_title = title.and_then(normalize_title);
        let new_detail = detail.and_then(normalize_title);
        if window.dynamic_title == new_title && window.dynamic_title_detail == new_detail {
            return false;
        }
        window.dynamic_title = new_title;
        window.dynamic_title_detail = new_detail;
        true
    }

    pub fn update_viewport(&mut self, viewport: CanvasViewport) -> bool {
        if self.persisted.viewport == viewport {
            return false;
        }
        self.persisted.viewport = viewport;
        true
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
            .filter_map(|(index, window)| {
                (!window.minimized
                    && window.placement.is_canvas()
                    && (window.tab_group_id.is_none() || window.tab_group_active))
                    .then_some(index)
            })
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
        if !self.persisted.windows[index].placement.is_canvas() {
            return false;
        }
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
            .filter(|(_, w)| !w.minimized && w.placement.is_canvas())
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

        // Find the currently focused visible window (highest z_index) within
        // eligible. Tab groups share one z-index across member windows, so the
        // active tab is the visible focus owner for that group.
        let visible_eligible = eligible
            .iter()
            .copied()
            .filter(|&i| {
                let window = &self.persisted.windows[i];
                window.tab_group_id.is_none() || window.tab_group_active
            })
            .collect::<Vec<_>>();
        let focus_candidates = if visible_eligible.is_empty() {
            &eligible
        } else {
            &visible_eligible
        };
        let current_idx = focus_candidates
            .iter()
            .copied()
            .max_by_key(|&i| self.persisted.windows[i].z_index)?;
        let pos = eligible.iter().position(|&i| i == current_idx)?;

        let next_pos = match direction {
            FocusCycleDirection::Forward => (pos + 1) % eligible.len(),
            FocusCycleDirection::Backward => (pos + eligible.len() - 1) % eligible.len(),
        };
        let current_id = self.persisted.windows[current_idx].id.clone();
        let next_id = self.persisted.windows[eligible[next_pos]].id.clone();
        if self
            .persisted
            .windows
            .get(current_idx)
            .is_some_and(|window| window.maximized)
        {
            let _ = self.restore_window(&current_id);
        }
        let _ = self.activate_window_tab(&next_id);
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
            .filter(|w| !w.minimized && !w.maximized && w.placement.is_canvas())
            .count();
        let mut step = 0usize;
        while step <= visible_window_count {
            let candidate_x = center_x + (step as f64) * STACK_OFFSET_X;
            let candidate_y = center_y + (step as f64) * STACK_OFFSET_Y;
            let occupied = self.persisted.windows.iter().any(|w| {
                !w.minimized
                    && !w.maximized
                    && w.placement.is_canvas()
                    && (w.geometry.x - candidate_x).abs() < 1.0
                    && (w.geometry.y - candidate_y).abs() < 1.0
            });
            if !occupied {
                break;
            }
            step += 1;
        }
        let step = step as f64;

        self.push_window_with_geometry(
            preset,
            title,
            persist,
            WindowGeometry {
                x: center_x + step * STACK_OFFSET_X,
                y: center_y + step * STACK_OFFSET_Y,
                width,
                height,
            },
        )
    }

    pub fn add_window_at_geometry_with_title(
        &mut self,
        preset: WindowPreset,
        title: impl Into<String>,
        persist: bool,
        geometry: WindowGeometry,
    ) -> PersistedWindowState {
        self.push_window_with_geometry(preset, title, persist, geometry)
    }

    fn push_window_with_geometry(
        &mut self,
        preset: WindowPreset,
        title: impl Into<String>,
        persist: bool,
        geometry: WindowGeometry,
    ) -> PersistedWindowState {
        let window = PersistedWindowState {
            id: self.next_window_id(preset),
            title: title.into(),
            preset,
            geometry,
            geometry_revision: 0,
            z_index: self.persisted.next_z_index,
            status: WindowProcessStatus::Running,
            minimized: false,
            maximized: false,
            pre_maximize_geometry: None,
            placement: WindowPlacement::Canvas,
            persist,
            purpose_title: None,
            dynamic_title: None,
            dynamic_title_detail: None,
            agent_id: None,
            agent_color: None,
            tab_group_id: None,
            tab_group_active: false,
            session_id: None,
        };
        self.persisted.next_z_index += 1;
        self.persisted.windows.push(window.clone());
        window
    }

    /// Idempotent "ensure maximized at these bounds" command.
    ///
    /// Issue #2757 follow-up: this used to toggle between maximized and
    /// restored on every call, which created a WebSocket flood when the
    /// frontend's `syncMaximizedWindowsToViewport()` re-sent the same
    /// `maximize_window` event after each `workspace_state` broadcast
    /// (~3000+ frames/sec observed in the wild). The frontend always
    /// uses `restore_window` to unmaximize, so `maximize_window` is only
    /// ever meant to assert "set to maximized with this geometry".
    ///
    /// Returns `true` only when state actually changed, so the caller
    /// can suppress the redundant broadcast that fuels the loop.
    pub fn maximize_window(&mut self, id: &str, bounds: WindowGeometry) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if !self.persisted.windows[index].placement.is_canvas() {
            return false;
        }
        let group_id = self.persisted.windows[index].tab_group_id.clone();
        let was_maximized = self.persisted.windows[index].maximized;
        let pre_geometry = self.persisted.windows[index].geometry.clone();
        // The frontend sends the FINAL maximized geometry (with a zoom-corrected
        // screen-space inset already applied in `maximizedGeometry`), so store it
        // verbatim. The previous code added a constant `ARRANGE_PADDING` in WORLD
        // units here, which rendered as an `ARRANGE_PADDING * zoom` SCREEN inset
        // under the canvas-stage `scale(zoom)` transform and drifted the maximized
        // window off the visible viewport at any zoom != 1.
        let target_geometry = bounds;

        // No-op fast path: already maximized at the exact target geometry.
        // Without this, repeated sync events from the frontend turn into a
        // feedback loop with the backend broadcasting a fresh
        // `workspace_state` every iteration.
        if was_maximized && self.persisted.windows[index].geometry == target_geometry {
            return false;
        }

        let members = self.group_member_indices(group_id.as_deref(), index);
        let geometry_revision = self.next_geometry_revision(&members);
        for member in members {
            let window = &mut self.persisted.windows[member];
            if !was_maximized {
                // First-time maximize: remember the previous geometry so
                // `restore_window` can put it back.
                window.pre_maximize_geometry = Some(pre_geometry.clone());
            }
            window.geometry = target_geometry.clone();
            window.minimized = false;
            window.maximized = true;
            window.geometry_revision = geometry_revision;
        }
        self.bring_to_front(index);
        true
    }

    pub fn minimize_window(&mut self, id: &str) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if !self.persisted.windows[index].placement.is_canvas() {
            return false;
        }
        let group_id = self.persisted.windows[index].tab_group_id.clone();
        let was_minimized = self.persisted.windows[index].minimized;
        let was_maximized = self.persisted.windows[index].maximized;
        let restore_geometry = self.persisted.windows[index].pre_maximize_geometry.clone();

        let members = self.group_member_indices(group_id.as_deref(), index);
        let geometry_revision = was_maximized.then(|| self.next_geometry_revision(&members));
        for member in members {
            let window = &mut self.persisted.windows[member];
            if was_minimized {
                window.minimized = false;
            } else {
                if was_maximized {
                    if let Some(geometry) = restore_geometry.clone() {
                        window.geometry = geometry;
                    }
                    if let Some(geometry_revision) = geometry_revision {
                        window.geometry_revision = geometry_revision;
                    }
                    window.pre_maximize_geometry = None;
                    window.maximized = false;
                }
                window.minimized = true;
            }
        }
        self.bring_to_front(index);
        true
    }

    pub fn restore_window(&mut self, id: &str) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if !self.persisted.windows[index].placement.is_canvas() {
            return false;
        }
        let window = &self.persisted.windows[index];
        let was_maximized = window.maximized;
        let was_minimized = window.minimized;
        if !was_maximized && !was_minimized {
            return false;
        }
        let group_id = window.tab_group_id.clone();
        let restore_geometry = window.pre_maximize_geometry.clone();

        let members = self.group_member_indices(group_id.as_deref(), index);
        let geometry_revision = was_maximized.then(|| self.next_geometry_revision(&members));
        for member in members {
            let window = &mut self.persisted.windows[member];
            if was_maximized {
                if let Some(geometry) = restore_geometry.clone() {
                    window.geometry = geometry;
                }
                if let Some(geometry_revision) = geometry_revision {
                    window.geometry_revision = geometry_revision;
                }
                window.pre_maximize_geometry = None;
                window.maximized = false;
                window.minimized = false;
            } else {
                window.minimized = false;
            }
        }
        self.bring_to_front(index);
        true
    }

    pub fn update_geometry(&mut self, id: &str, geometry: WindowGeometry) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if !self.persisted.windows[index].placement.is_canvas() {
            return false;
        }
        let group_id = self.persisted.windows[index].tab_group_id.clone();
        let members = self.group_member_indices(group_id.as_deref(), index);
        self.set_geometry_for_indices(&members, &geometry);
        true
    }

    pub fn close_window(&mut self, id: &str) -> bool {
        let initial_len = self.persisted.windows.len();
        let group_id = self
            .window(id)
            .and_then(|window| window.tab_group_id.clone());
        if self
            .window(id)
            .is_some_and(|window| window.preset == WindowPreset::AgentKanban)
        {
            self.undock_agent_windows_for_board(id);
        }
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
        let Some(source_index) = self.window_index(id) else {
            return false;
        };
        let Some(target_index) = self.window_index(target_id) else {
            return false;
        };
        if !self.persisted.windows[source_index].placement.is_canvas()
            || !self.persisted.windows[target_index].placement.is_canvas()
        {
            return false;
        }
        let source_group_id = self.persisted.windows[source_index].tab_group_id.clone();
        let group_id = self.persisted.windows[target_index]
            .tab_group_id
            .clone()
            .unwrap_or_else(|| format!("group-{}", self.persisted.windows[target_index].id));
        let group_geometry = self.persisted.windows[target_index].geometry.clone();
        let next_z_index = self.persisted.next_z_index;
        self.persisted.next_z_index += 1;

        let affected_indices = self
            .persisted
            .windows
            .iter()
            .enumerate()
            .filter_map(|(index, window)| {
                (window.id == id
                    || window.id == target_id
                    || window.tab_group_id.as_deref() == Some(&group_id))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        let geometry_revision = self.next_geometry_revision(&affected_indices);
        for index in affected_indices {
            let window = &mut self.persisted.windows[index];
            window.tab_group_id = Some(group_id.clone());
            window.tab_group_active = window.id == id;
            window.geometry = group_geometry.clone();
            window.geometry_revision = geometry_revision;
            window.minimized = false;
            window.maximized = false;
            window.pre_maximize_geometry = None;
            window.z_index = next_z_index;
        }
        if let Some(source_group_id) = source_group_id {
            if source_group_id != group_id {
                self.normalize_group(&source_group_id);
            }
        }
        true
    }

    pub fn activate_window_tab(&mut self, id: &str) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if !self.persisted.windows[index].placement.is_canvas() {
            return false;
        }
        let Some(group_id) = self.persisted.windows[index].tab_group_id.clone() else {
            return self.focus_window(id, None);
        };
        // SPEC-2008 FR-043C: chrome 状態 (geometry / maximize / minimize /
        // pre_maximize_geometry) は group-aware mutator で常時同期されている
        // ため、activate ではアクティブマーカと z_index のみを更新する。
        let next_z_index = self.persisted.next_z_index;
        self.persisted.next_z_index += 1;
        for window in &mut self.persisted.windows {
            if window.tab_group_id.as_deref() == Some(&group_id) {
                window.tab_group_active = window.id == id;
                window.z_index = next_z_index;
            }
        }
        true
    }

    pub fn detach_window_tab(&mut self, id: &str, geometry: WindowGeometry) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if !self.persisted.windows[index].placement.is_canvas() {
            return false;
        }
        let Some(group_id) = self.persisted.windows[index].tab_group_id.clone() else {
            self.set_geometry_for_indices(&[index], &geometry);
            self.bring_to_front(index);
            return true;
        };
        self.persisted.windows[index].tab_group_id = None;
        self.persisted.windows[index].tab_group_active = false;
        self.set_geometry_for_indices(&[index], &geometry);
        self.persisted.windows[index].minimized = false;
        self.persisted.windows[index].maximized = false;
        self.persisted.windows[index].pre_maximize_geometry = None;
        self.bring_to_front(index);
        self.normalize_group(&group_id);
        true
    }

    pub fn place_agent_window_in_kanban(
        &mut self,
        id: &str,
        board_id: &str,
        lane_id: AgentKanbanLane,
        order: Option<u32>,
    ) -> bool {
        let Some(source_index) = self.window_index(id) else {
            return false;
        };
        let Some(board_index) = self.window_index(board_id) else {
            return false;
        };
        if source_index == board_index
            || !self.persisted.windows[source_index]
                .preset
                .is_agent_terminal()
            || self.persisted.windows[board_index].preset != WindowPreset::AgentKanban
            || self.persisted.windows[source_index].tab_group_id.is_some()
        {
            return false;
        }

        let order = order.unwrap_or_else(|| self.next_agent_kanban_order(board_id, lane_id));
        let window = &mut self.persisted.windows[source_index];
        window.placement = WindowPlacement::AgentKanban {
            board_id: board_id.to_string(),
            lane_id,
            order,
            collapsed: false,
        };
        window.minimized = false;
        window.maximized = false;
        window.pre_maximize_geometry = None;
        self.normalize_agent_kanban_orders(board_id);
        true
    }

    pub fn move_agent_kanban_card(
        &mut self,
        id: &str,
        board_id: &str,
        lane_id: AgentKanbanLane,
        order: u32,
    ) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if !self
            .window(board_id)
            .is_some_and(|window| window.preset == WindowPreset::AgentKanban)
        {
            return false;
        }
        let WindowPlacement::AgentKanban {
            board_id: previous_board_id,
            collapsed,
            ..
        } = self.persisted.windows[index].placement.clone()
        else {
            return false;
        };
        self.persisted.windows[index].placement = WindowPlacement::AgentKanban {
            board_id: board_id.to_string(),
            lane_id,
            order,
            collapsed,
        };
        if previous_board_id != board_id {
            self.normalize_agent_kanban_orders(&previous_board_id);
        }
        self.normalize_agent_kanban_orders(board_id);
        true
    }

    pub fn set_agent_kanban_card_collapsed(&mut self, id: &str, collapsed: bool) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        let WindowPlacement::AgentKanban {
            board_id,
            lane_id,
            order,
            ..
        } = self.persisted.windows[index].placement.clone()
        else {
            return false;
        };
        self.persisted.windows[index].placement = WindowPlacement::AgentKanban {
            board_id,
            lane_id,
            order,
            collapsed,
        };
        true
    }

    pub fn undock_agent_window(&mut self, id: &str, geometry: Option<WindowGeometry>) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if !matches!(
            self.persisted.windows[index].placement,
            WindowPlacement::AgentKanban { .. }
        ) {
            return false;
        }
        self.persisted.windows[index].placement = WindowPlacement::Canvas;
        if let Some(geometry) = geometry {
            self.set_geometry_for_indices(&[index], &geometry);
        }
        self.persisted.windows[index].minimized = false;
        self.persisted.windows[index].maximized = false;
        self.persisted.windows[index].pre_maximize_geometry = None;
        self.bring_to_front(index);
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
            let group_id = self.persisted.windows[*window_index].tab_group_id.clone();
            let members = self.group_member_indices(group_id.as_deref(), *window_index);
            let geometry_revision = self.next_geometry_revision(&members);
            let column = index % columns;
            let row = index / columns;
            let geometry = WindowGeometry {
                x: bounds.x + ARRANGE_PADDING + column as f64 * (width + ARRANGE_PADDING),
                y: bounds.y + ARRANGE_PADDING + row as f64 * (height + ARRANGE_PADDING),
                width,
                height,
            };
            for member in members {
                let window = &mut self.persisted.windows[member];
                window.geometry = geometry.clone();
                window.geometry_revision = geometry_revision;
                window.maximized = false;
                window.pre_maximize_geometry = None;
            }
        }
    }

    fn arrange_stack(&mut self, bounds: WindowGeometry, open_indices: &[usize]) {
        let available_width = (bounds.width - STACK_START_INSET * 2.0).max(MIN_WINDOW_WIDTH);
        let available_height = (bounds.height - STACK_START_INSET * 2.0).max(MIN_WINDOW_HEIGHT);

        for (index, window_index) in open_indices.iter().enumerate() {
            let window_geometry = self.persisted.windows[*window_index].geometry.clone();
            let group_id = self.persisted.windows[*window_index].tab_group_id.clone();
            let members = self.group_member_indices(group_id.as_deref(), *window_index);
            let geometry_revision = self.next_geometry_revision(&members);
            let geometry = WindowGeometry {
                x: bounds.x + STACK_START_INSET + index as f64 * STACK_OFFSET_X,
                y: bounds.y + STACK_START_INSET + index as f64 * STACK_OFFSET_Y,
                width: window_geometry
                    .width
                    .min(available_width)
                    .max(MIN_WINDOW_WIDTH),
                height: window_geometry
                    .height
                    .min(available_height)
                    .max(MIN_WINDOW_HEIGHT),
            };
            for member in members {
                let window = &mut self.persisted.windows[member];
                window.geometry = geometry.clone();
                window.geometry_revision = geometry_revision;
                window.maximized = false;
                window.pre_maximize_geometry = None;
            }
        }
    }

    fn arrange_align(&mut self, bounds: WindowGeometry, open_indices: &[usize]) {
        let count = open_indices.len();
        let columns = (count as f64).sqrt().ceil() as usize;
        let rows = count.div_ceil(columns);

        for &window_index in open_indices {
            let group_id = self.persisted.windows[window_index].tab_group_id.clone();
            let members = self.group_member_indices(group_id.as_deref(), window_index);
            if let Some(geometry) = self.persisted.windows[window_index]
                .pre_maximize_geometry
                .clone()
            {
                for member in members {
                    let window = &mut self.persisted.windows[member];
                    window.geometry.width = geometry.width;
                    window.geometry.height = geometry.height;
                    window.pre_maximize_geometry = None;
                }
            }
        }

        let mut column_widths = vec![0.0_f64; columns];
        let mut row_heights = vec![0.0_f64; rows];
        for (index, &window_index) in open_indices.iter().enumerate() {
            let column = index % columns;
            let row = index / columns;
            let window = &self.persisted.windows[window_index];
            column_widths[column] = column_widths[column].max(window.geometry.width);
            row_heights[row] = row_heights[row].max(window.geometry.height);
        }

        let mut column_offsets = vec![bounds.x + ARRANGE_PADDING; columns];
        for c in 1..columns {
            column_offsets[c] = column_offsets[c - 1] + column_widths[c - 1] + ARRANGE_PADDING;
        }
        let mut row_offsets = vec![bounds.y + ARRANGE_PADDING; rows];
        for r in 1..rows {
            row_offsets[r] = row_offsets[r - 1] + row_heights[r - 1] + ARRANGE_PADDING;
        }

        for (index, &window_index) in open_indices.iter().enumerate() {
            let group_id = self.persisted.windows[window_index].tab_group_id.clone();
            let members = self.group_member_indices(group_id.as_deref(), window_index);
            let geometry_revision = self.next_geometry_revision(&members);
            let column = index % columns;
            let row = index / columns;
            for member in members {
                let window = &mut self.persisted.windows[member];
                window.geometry.x = column_offsets[column];
                window.geometry.y = row_offsets[row];
                window.geometry_revision = geometry_revision;
                window.maximized = false;
                window.pre_maximize_geometry = None;
            }
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

    // SPEC-2008 FR-043C: group-aware mutator が触るべき index 集合を返す。
    // group_id が None なら fallback として primary_index 単独を返し、未グループ
    // ウィンドウでも同じコードパスを使い回せるようにする。
    fn group_member_indices(&self, group_id: Option<&str>, primary_index: usize) -> Vec<usize> {
        let Some(group_id) = group_id else {
            return vec![primary_index];
        };
        let members: Vec<usize> = self
            .persisted
            .windows
            .iter()
            .enumerate()
            .filter_map(|(index, window)| {
                (window.tab_group_id.as_deref() == Some(group_id)).then_some(index)
            })
            .collect();
        if members.is_empty() {
            vec![primary_index]
        } else {
            members
        }
    }

    fn next_geometry_revision(&self, indices: &[usize]) -> u64 {
        indices
            .iter()
            .filter_map(|&index| self.persisted.windows.get(index))
            .map(|window| window.geometry_revision)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    fn set_geometry_for_indices(&mut self, indices: &[usize], geometry: &WindowGeometry) {
        let geometry_revision = self.next_geometry_revision(indices);
        for &index in indices {
            let Some(window) = self.persisted.windows.get_mut(index) else {
                continue;
            };
            window.geometry = geometry.clone();
            window.geometry_revision = geometry_revision;
        }
    }

    fn next_agent_kanban_order(&self, board_id: &str, lane_id: AgentKanbanLane) -> u32 {
        self.persisted
            .windows
            .iter()
            .filter_map(|window| match &window.placement {
                WindowPlacement::AgentKanban {
                    board_id: current_board_id,
                    lane_id: current_lane_id,
                    order,
                    ..
                } if current_board_id == board_id && *current_lane_id == lane_id => Some(*order),
                _ => None,
            })
            .max()
            .map(|order| order.saturating_add(1))
            .unwrap_or(0)
    }

    fn normalize_agent_kanban_orders(&mut self, board_id: &str) {
        for lane_id in AgentKanbanLane::ALL {
            let mut indices = self
                .persisted
                .windows
                .iter()
                .enumerate()
                .filter_map(|(index, window)| match &window.placement {
                    WindowPlacement::AgentKanban {
                        board_id: current_board_id,
                        lane_id: current_lane_id,
                        order,
                        ..
                    } if current_board_id == board_id && *current_lane_id == lane_id => {
                        Some((index, *order))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            indices.sort_by_key(|(_, order)| *order);
            for (order, (index, _)) in indices.into_iter().enumerate() {
                if let WindowPlacement::AgentKanban {
                    order: stored_order,
                    ..
                } = &mut self.persisted.windows[index].placement
                {
                    *stored_order = order as u32;
                }
            }
        }
    }

    fn undock_agent_windows_for_board(&mut self, board_id: &str) {
        for window in &mut self.persisted.windows {
            if matches!(
                &window.placement,
                WindowPlacement::AgentKanban {
                    board_id: current_board_id,
                    ..
                } if current_board_id == board_id
            ) {
                window.placement = WindowPlacement::Canvas;
                window.minimized = false;
                window.maximized = false;
                window.pre_maximize_geometry = None;
            }
        }
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

    fn workspace_with_five_logical_windows_in_three_physical_slots() -> WindowCanvasState {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        workspace.add_window(WindowPreset::Shell, arrange_bounds());
        let file_tree = workspace.add_window(WindowPreset::FileTree, arrange_bounds());
        let branches = workspace.add_window(WindowPreset::Branches, arrange_bounds());

        assert!(workspace.dock_window_tab("codex-1", "claude-1"));
        assert!(workspace.dock_window_tab(&branches.id, &file_tree.id));

        workspace
    }

    #[test]
    fn focusing_window_brings_it_to_front() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::FileTree, arrange_bounds());
        assert_eq!(window.title, "File Tree");
        assert_eq!(window.preset, WindowPreset::FileTree);
        assert_eq!(window.status, WindowProcessStatus::Running);
    }

    #[test]
    fn adding_branches_window_marks_it_running_without_process() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::Branches, arrange_bounds());
        assert_eq!(window.title, "Branches");
        assert_eq!(window.preset, WindowPreset::Branches);
        assert_eq!(window.status, WindowProcessStatus::Running);
    }

    #[test]
    fn placing_agent_window_in_kanban_sets_containment_placement() {
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::AgentKanban, arrange_bounds());
        let agent = workspace.add_window(WindowPreset::Agent, arrange_bounds());

        assert!(workspace.place_agent_window_in_kanban(
            &agent.id,
            &board.id,
            AgentKanbanLane::Active,
            None,
        ));

        let agent = workspace.window(&agent.id).expect("agent window");
        assert_eq!(
            agent.placement,
            WindowPlacement::AgentKanban {
                board_id: board.id.clone(),
                lane_id: AgentKanbanLane::Active,
                order: 0,
                collapsed: false,
            }
        );
        assert!(!agent.minimized);
        assert!(!agent.maximized);
    }

    #[test]
    fn agent_kanban_rejects_non_agent_or_grouped_sources() {
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::AgentKanban, arrange_bounds());
        let shell = workspace.add_window(WindowPreset::Shell, arrange_bounds());
        let claude = workspace.add_window(WindowPreset::Claude, arrange_bounds());
        let codex = workspace.add_window(WindowPreset::Codex, arrange_bounds());
        let agent = workspace.add_window(WindowPreset::Agent, arrange_bounds());
        let target = workspace.add_window(WindowPreset::Agent, arrange_bounds());
        assert!(workspace.dock_window_tab(&agent.id, &target.id));

        assert!(workspace.place_agent_window_in_kanban(
            &claude.id,
            &board.id,
            AgentKanbanLane::Plan,
            None,
        ));
        assert!(workspace.place_agent_window_in_kanban(
            &codex.id,
            &board.id,
            AgentKanbanLane::Active,
            None,
        ));
        assert!(!workspace.place_agent_window_in_kanban(
            &shell.id,
            &board.id,
            AgentKanbanLane::Plan,
            None,
        ));
        assert!(!workspace.place_agent_window_in_kanban(
            &agent.id,
            &board.id,
            AgentKanbanLane::Plan,
            None,
        ));
    }

    #[test]
    fn focus_cycle_skips_agent_kanban_contained_agents() {
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::AgentKanban, arrange_bounds());
        let agent = workspace.add_window(WindowPreset::Agent, arrange_bounds());
        assert!(workspace.place_agent_window_in_kanban(
            &agent.id,
            &board.id,
            AgentKanbanLane::Active,
            None,
        ));

        let focused = workspace.cycle_focus(FocusCycleDirection::Forward, arrange_bounds());

        assert_eq!(focused.as_deref(), Some(board.id.as_str()));
    }

    #[test]
    fn closing_agent_kanban_window_undocks_contained_agents() {
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::AgentKanban, arrange_bounds());
        let agent = workspace.add_window(WindowPreset::Agent, arrange_bounds());
        assert!(workspace.place_agent_window_in_kanban(
            &agent.id,
            &board.id,
            AgentKanbanLane::Done,
            None,
        ));

        assert!(workspace.close_window(&board.id));

        assert!(workspace.window(&board.id).is_none());
        assert_eq!(
            workspace.window(&agent.id).expect("agent").placement,
            WindowPlacement::Canvas
        );
    }

    #[test]
    fn moving_agent_kanban_card_rejects_missing_board() {
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::AgentKanban, arrange_bounds());
        let agent = workspace.add_window(WindowPreset::Agent, arrange_bounds());
        assert!(workspace.place_agent_window_in_kanban(
            &agent.id,
            &board.id,
            AgentKanbanLane::Plan,
            None,
        ));

        assert!(!workspace.move_agent_kanban_card(
            &agent.id,
            "missing-board",
            AgentKanbanLane::Done,
            0,
        ));

        assert_eq!(
            workspace.window(&agent.id).expect("agent").placement,
            WindowPlacement::AgentKanban {
                board_id: board.id,
                lane_id: AgentKanbanLane::Plan,
                order: 0,
                collapsed: false,
            }
        );
    }

    #[test]
    fn adding_window_centers_in_bounds() {
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(PersistedWindowCanvasState {
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
                geometry_revision: 0,
                z_index: 1,
                status: WindowProcessStatus::Running,
                minimized: false,
                maximized: false,
                pre_maximize_geometry: None,
                placement: WindowPlacement::Canvas,
                persist: false,
                purpose_title: None,
                dynamic_title: None,
                dynamic_title_detail: None,
                agent_id: None,
                agent_color: None,
                tab_group_id: None,
                tab_group_active: false,
                session_id: None,
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        let viewport = CanvasViewport {
            x: 180.0,
            y: -90.0,
            zoom: 1.35,
        };
        assert!(workspace.update_viewport(viewport.clone()));
        assert_eq!(
            workspace.persisted().viewport,
            CanvasViewport {
                x: 180.0,
                y: -90.0,
                zoom: 1.35
            }
        );
        assert!(
            !workspace.update_viewport(viewport),
            "same viewport should report no mutation"
        );
    }

    #[test]
    fn tile_arrangement_places_windows_on_a_grid() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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

        // bounds: x=100, y=40, width=1000, height=760, padding=24
        // columns=2, rows=2; column 0 max width = max(claude=720, file-tree=420) = 720
        // column 1 max width = codex=720; row 0 max height = 420; row 1 max height = 520.
        assert_eq!(claude.geometry.x, 124.0);
        assert_eq!(claude.geometry.y, 64.0);
        assert_eq!(codex.geometry.x, 868.0);
        assert_eq!(codex.geometry.y, 64.0);
        assert_eq!(file_tree.geometry.x, 124.0);
        assert_eq!(file_tree.geometry.y, 508.0);
    }

    #[test]
    fn align_arrangement_does_not_overlap_windows_with_varying_sizes() {
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
        let preset_sizes = [
            (800.0, 360.0),
            (420.0, 540.0),
            (600.0, 380.0),
            (500.0, 470.0),
        ];
        let mut ids = Vec::new();
        for (width, height) in preset_sizes {
            let window = workspace.add_window(WindowPreset::Settings, arrange_bounds());
            ids.push(window.id.clone());
            assert!(workspace.update_geometry(
                &window.id,
                WindowGeometry {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height,
                },
            ));
        }

        assert!(workspace.arrange_windows(ArrangeMode::Align, arrange_bounds()));

        let snapshot: Vec<_> = ids
            .iter()
            .map(|id| {
                let window = workspace.window(id).expect("added window");
                (id.clone(), window.geometry.clone())
            })
            .collect();

        for (i, (id_a, a)) in snapshot.iter().enumerate() {
            for (id_b, b) in snapshot.iter().skip(i + 1) {
                let separated = a.x + a.width <= b.x
                    || b.x + b.width <= a.x
                    || a.y + a.height <= b.y
                    || b.y + b.height <= a.y;
                assert!(
                    separated,
                    "{id_a} and {id_b} overlap after Align: {:?} vs {:?}",
                    a, b
                );
            }
        }

        // Width/height of every window must remain untouched (FR-004 / SC-017).
        for ((id, geometry), (expected_width, expected_height)) in
            snapshot.iter().zip(preset_sizes.iter())
        {
            assert_eq!(geometry.width, *expected_width, "{id} width preserved");
            assert_eq!(geometry.height, *expected_height, "{id} height preserved");
        }
    }

    #[test]
    fn align_arrangement_restores_maximized_window_size_before_positioning() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        // After maximize, claude.z=3 sorts after codex.z=2 -> open_indices = [codex, claude].
        // column 0 width = codex.width = 720, column 1 width = claude.width = 720.
        assert_eq!(claude.geometry.x, 868.0);
        assert_eq!(claude.geometry.y, 64.0);
    }

    #[test]
    fn cycling_focus_forward_brings_next_window_to_front_and_centers_it() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
    fn cycling_focus_forward_activates_hidden_window_tab() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));
        workspace.add_window(WindowPreset::Shell, arrange_bounds());
        // Current focus: shell-1. Forward wraps to claude-1, which is hidden
        // behind the active codex-1 tab in the same group.

        let focused = workspace
            .cycle_focus(FocusCycleDirection::Forward, arrange_bounds())
            .expect("focused window");

        assert_eq!(focused, "claude-1");
        assert!(
            workspace
                .window("claude-1")
                .expect("claude")
                .tab_group_active
        );
        assert!(!workspace.window("codex-1").expect("codex").tab_group_active);
    }

    #[test]
    fn cycling_focus_backward_activates_hidden_window_tab() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));
        assert!(workspace.activate_window_tab("claude-1"));
        workspace.add_window(WindowPreset::Shell, arrange_bounds());
        // Current focus: shell-1. Backward goes to codex-1, which is hidden
        // behind the active claude-1 tab in the same group.

        let focused = workspace
            .cycle_focus(FocusCycleDirection::Backward, arrange_bounds())
            .expect("focused window");

        assert_eq!(focused, "codex-1");
        assert!(workspace.window("codex-1").expect("codex").tab_group_active);
        assert!(
            !workspace
                .window("claude-1")
                .expect("claude")
                .tab_group_active
        );
    }

    #[test]
    fn cycling_focus_uses_active_window_tab_as_current_focus_when_group_z_indexes_tie() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));
        // Docking assigns one shared z-index to the group. The active tab is
        // codex-1, so Forward should wrap to claude-1, not treat hidden
        // claude-1 as the current focus just because its z-index ties.

        let focused = workspace
            .cycle_focus(FocusCycleDirection::Forward, arrange_bounds())
            .expect("focused window");

        assert_eq!(focused, "claude-1");
        assert!(
            workspace
                .window("claude-1")
                .expect("claude")
                .tab_group_active
        );
    }

    #[test]
    fn cycling_focus_restores_maximized_source_before_activating_next_window() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        let bounds = WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 1200.0,
            height: 800.0,
        };
        let codex_original = workspace.window("codex-1").expect("codex").geometry.clone();
        let claude = workspace
            .window("claude-1")
            .expect("claude")
            .geometry
            .clone();

        assert!(workspace.maximize_window("codex-1", bounds.clone()));
        let focused = workspace
            .cycle_focus(FocusCycleDirection::Forward, bounds.clone())
            .expect("focused window");

        assert_eq!(focused, "claude-1");
        let codex = workspace.window("codex-1").expect("codex");
        assert_eq!(codex.geometry, codex_original);
        assert!(!codex.maximized);
        assert_eq!(codex.pre_maximize_geometry, None);
        assert!(
            workspace.window("claude-1").expect("claude").z_index > codex.z_index,
            "next window must end as topmost after source restore"
        );
        assert_eq!(
            workspace.persisted().viewport.x,
            bounds.width / 2.0 - (claude.x + claude.width / 2.0)
        );
        assert_eq!(
            workspace.persisted().viewport.y,
            bounds.height / 2.0 - (claude.y + claude.height / 2.0)
        );
    }

    #[test]
    fn cycling_focus_restores_maximized_group_before_activating_hidden_tab() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));
        let bounds = arrange_bounds();
        let original = workspace.window("codex-1").expect("codex").geometry.clone();

        assert!(workspace.maximize_window("codex-1", bounds.clone()));
        let focused = workspace
            .cycle_focus(FocusCycleDirection::Forward, bounds)
            .expect("focused window");

        assert_eq!(focused, "claude-1");
        let codex = workspace.window("codex-1").expect("codex");
        let claude = workspace.window("claude-1").expect("claude");
        assert_eq!(codex.geometry, original);
        assert_eq!(claude.geometry, original);
        assert!(!codex.maximized);
        assert!(!claude.maximized);
        assert!(claude.tab_group_active);
        assert!(!codex.tab_group_active);
    }

    #[test]
    fn cycling_focus_keeps_single_maximized_window_maximized() {
        let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
        let window = workspace.add_window(WindowPreset::Shell, arrange_bounds());
        assert!(workspace.maximize_window(&window.id, arrange_bounds()));

        let focused = workspace
            .cycle_focus(FocusCycleDirection::Forward, arrange_bounds())
            .expect("focused window");

        assert_eq!(focused, window.id);
        assert!(workspace.window(&window.id).expect("window").maximized);
    }

    #[test]
    fn maximizing_window_is_idempotent_and_uses_restore_to_revert() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        // The backend now stores the received geometry VERBATIM — the frontend
        // applies the zoom-corrected screen inset in `maximizedGeometry` before
        // sending. (Previously the backend added ARRANGE_PADDING here, which
        // became a zoom-scaled inset under the canvas-stage transform and drifted
        // the maximized window off the viewport at any zoom != 1.)
        assert_eq!(maximized.geometry, arrange_bounds());

        // Issue #2757 follow-up: repeated maximize_window with the same
        // bounds must be a no-op. The frontend's viewport-sync path used to
        // trigger this every workspace_state broadcast; the old toggle
        // behaviour produced a 400+ msg/sec WebSocket flood that starved
        // resume_workspace and other clicks.
        assert!(
            !workspace.maximize_window("claude-1", arrange_bounds()),
            "re-issuing maximize at identical bounds must be reported as no-op",
        );
        let still_maximized = workspace.window("claude-1").expect("claude");
        assert!(still_maximized.maximized);
        assert_eq!(
            still_maximized.pre_maximize_geometry,
            Some(original.clone())
        );

        assert!(workspace.restore_window("claude-1"));
        let restored = workspace.window("claude-1").expect("claude");
        assert!(!restored.maximized);
        assert!(!restored.minimized);
        assert_eq!(restored.pre_maximize_geometry, None);
        assert_eq!(restored.geometry, original);
    }

    #[test]
    fn maximize_window_stores_received_geometry_verbatim_without_world_padding() {
        // Regression: the zoom-correct maximize inset is applied on the frontend
        // (`maximizedGeometry` divides the 24px screen inset by zoom). The backend
        // must NOT re-pad in world units, otherwise the inset scales with zoom and
        // the maximized window drifts off the visible viewport.
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        let geometry = WindowGeometry {
            x: 312.5,
            y: 88.0,
            width: 640.0,
            height: 360.0,
        };
        assert!(workspace.maximize_window("claude-1", geometry.clone()));
        let maximized = workspace.window("claude-1").expect("claude");
        assert!(maximized.maximized);
        assert_eq!(
            maximized.geometry, geometry,
            "backend must store the frontend-computed maximize geometry unchanged",
        );
    }

    #[test]
    fn minimizing_window_toggles_and_preserves_normal_geometry() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
    fn tile_arrangement_counts_tabbed_groups_as_one_physical_window() {
        let mut workspace = workspace_with_five_logical_windows_in_three_physical_slots();

        assert!(workspace.arrange_windows(ArrangeMode::Tile, arrange_bounds()));

        let shell = workspace.window("shell-1").expect("shell");
        let codex = workspace.window("codex-1").expect("codex");
        let claude = workspace.window("claude-1").expect("claude");
        let branches = workspace.window("branches-1").expect("branches");
        let file_tree = workspace.window("file-tree-1").expect("file tree");

        assert_eq!((shell.geometry.x, shell.geometry.y), (124.0, 64.0));
        assert_eq!((codex.geometry.x, codex.geometry.y), (612.0, 64.0));
        assert_eq!((branches.geometry.x, branches.geometry.y), (124.0, 432.0));
        assert_eq!(codex.geometry, claude.geometry);
        assert_eq!(branches.geometry, file_tree.geometry);
    }

    #[test]
    fn stack_arrangement_counts_tabbed_groups_as_one_physical_window() {
        let mut workspace = workspace_with_five_logical_windows_in_three_physical_slots();

        assert!(workspace.arrange_windows(ArrangeMode::Stack, arrange_bounds()));

        let shell = workspace.window("shell-1").expect("shell");
        let codex = workspace.window("codex-1").expect("codex");
        let claude = workspace.window("claude-1").expect("claude");
        let branches = workspace.window("branches-1").expect("branches");
        let file_tree = workspace.window("file-tree-1").expect("file tree");

        assert_eq!((shell.geometry.x, shell.geometry.y), (148.0, 88.0));
        assert_eq!((codex.geometry.x, codex.geometry.y), (176.0, 112.0));
        assert_eq!((branches.geometry.x, branches.geometry.y), (204.0, 136.0));
        assert_eq!(codex.geometry, claude.geometry);
        assert_eq!(branches.geometry, file_tree.geometry);
    }

    #[test]
    fn align_arrangement_counts_tabbed_groups_as_one_physical_window() {
        let mut workspace = workspace_with_five_logical_windows_in_three_physical_slots();
        assert!(workspace.update_geometry(
            "shell-1",
            WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 500.0,
                height: 300.0,
            },
        ));
        assert!(workspace.update_geometry(
            "codex-1",
            WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 720.0,
                height: 420.0,
            },
        ));
        assert!(workspace.update_geometry(
            "branches-1",
            WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 420.0,
                height: 520.0,
            },
        ));

        assert!(workspace.arrange_windows(ArrangeMode::Align, arrange_bounds()));

        let shell = workspace.window("shell-1").expect("shell");
        let codex = workspace.window("codex-1").expect("codex");
        let claude = workspace.window("claude-1").expect("claude");
        let branches = workspace.window("branches-1").expect("branches");
        let file_tree = workspace.window("file-tree-1").expect("file tree");

        assert_eq!((shell.geometry.x, shell.geometry.y), (124.0, 64.0));
        assert_eq!(
            (shell.geometry.width, shell.geometry.height),
            (500.0, 300.0)
        );
        assert_eq!((codex.geometry.x, codex.geometry.y), (648.0, 64.0));
        assert_eq!(
            (codex.geometry.width, codex.geometry.height),
            (720.0, 420.0)
        );
        assert_eq!((branches.geometry.x, branches.geometry.y), (124.0, 508.0));
        assert_eq!(
            (branches.geometry.width, branches.geometry.height),
            (420.0, 520.0)
        );
        assert_eq!(codex.geometry, claude.geometry);
        assert_eq!(branches.geometry, file_tree.geometry);
    }

    #[test]
    fn docking_window_tabs_groups_windows_and_activates_dragged_tab() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
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
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));

        assert!(workspace.close_window("codex-1"));

        let claude = workspace.window("claude-1").expect("claude");
        assert!(claude.tab_group_id.is_none());
        assert!(!claude.tab_group_active);
    }

    #[test]
    fn docking_active_tab_to_another_group_normalizes_source_group() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        let shell = workspace.add_window(WindowPreset::Shell, arrange_bounds());
        let file_tree = workspace.add_window(WindowPreset::FileTree, arrange_bounds());

        assert!(workspace.dock_window_tab("codex-1", "claude-1"));
        assert!(workspace.dock_window_tab(&shell.id, "codex-1"));
        assert!(workspace.window(&shell.id).expect("shell").tab_group_active);

        assert!(workspace.dock_window_tab(&shell.id, &file_tree.id));

        let remaining_source_group = ["claude-1", "codex-1"]
            .iter()
            .filter_map(|id| workspace.window(id))
            .collect::<Vec<_>>();
        assert_eq!(remaining_source_group.len(), 2);
        assert!(
            remaining_source_group
                .iter()
                .any(|window| window.tab_group_active),
            "source group must keep one visible active tab after active tab moves out"
        );
    }

    // SPEC-2008 US-14 / FR-043C: tab グループに属するウィンドウの geometry /
    // maximize / minimize / pre_maximize_geometry を変更する mutator は、同じ
    // tab_group_id を持つ全メンバーへ同一の値を伝播しなければならない。
    // activate_window_tab はアクティブマーカと z_index 以外の chrome 状態を
    // 変更してはならない。
    #[test]
    fn geometry_update_propagates_across_grouped_tabs() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));

        let new_geometry = WindowGeometry {
            x: 432.0,
            y: 256.0,
            width: 880.0,
            height: 540.0,
        };
        assert!(workspace.update_geometry("codex-1", new_geometry.clone()));

        assert_eq!(
            workspace.window("codex-1").expect("codex").geometry,
            new_geometry,
            "active tab must record its updated geometry"
        );
        assert_eq!(
            workspace.window("claude-1").expect("claude").geometry,
            new_geometry,
            "grouped sibling tab must adopt the same geometry so tab switch does not reset chrome"
        );
        assert_eq!(
            workspace
                .window("codex-1")
                .expect("codex")
                .geometry_revision,
            workspace
                .window("claude-1")
                .expect("claude")
                .geometry_revision,
            "grouped tabs must share the same geometry revision after propagated updates"
        );
        assert!(
            workspace
                .window("codex-1")
                .expect("codex")
                .geometry_revision
                > 0,
            "propagated geometry updates must advance the revision"
        );

        assert!(workspace.activate_window_tab("claude-1"));
        assert_eq!(
            workspace.window("claude-1").expect("claude").geometry,
            new_geometry,
            "activate must not overwrite the propagated geometry"
        );
        assert_eq!(
            workspace.window("codex-1").expect("codex").geometry,
            new_geometry,
            "previously-active tab must keep the propagated geometry after switching back"
        );
    }

    #[test]
    fn maximize_propagates_across_grouped_tabs() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));
        let pre_geometry = workspace.window("codex-1").expect("codex").geometry.clone();

        let bounds = WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
        };
        assert!(workspace.maximize_window("codex-1", bounds.clone()));

        let codex = workspace.window("codex-1").expect("codex");
        let claude = workspace.window("claude-1").expect("claude");
        assert!(codex.maximized);
        assert!(
            claude.maximized,
            "grouped sibling must share maximize state"
        );
        assert_eq!(codex.geometry, claude.geometry);
        assert_eq!(
            codex.pre_maximize_geometry.as_ref(),
            Some(&pre_geometry),
            "pre_maximize_geometry must record the geometry shared before maximize"
        );
        assert_eq!(
            claude.pre_maximize_geometry.as_ref(),
            Some(&pre_geometry),
            "grouped sibling must share pre_maximize_geometry so restore is symmetric"
        );

        // Issue #2757 follow-up: re-issuing maximize at the same bounds is
        // a no-op. Restoring grouped siblings must go through
        // `restore_window` instead.
        assert!(!workspace.maximize_window("codex-1", bounds));
        assert!(workspace.restore_window("codex-1"));
        let codex = workspace.window("codex-1").expect("codex");
        let claude = workspace.window("claude-1").expect("claude");
        assert!(!codex.maximized, "restore must clear maximized");
        assert!(!claude.maximized, "grouped sibling must restore together");
        assert_eq!(codex.geometry, pre_geometry);
        assert_eq!(claude.geometry, pre_geometry);
        assert!(codex.pre_maximize_geometry.is_none());
        assert!(claude.pre_maximize_geometry.is_none());
    }

    #[test]
    fn minimize_and_restore_propagate_across_grouped_tabs() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));

        assert!(workspace.minimize_window("codex-1"));
        assert!(workspace.window("codex-1").expect("codex").minimized);
        assert!(
            workspace.window("claude-1").expect("claude").minimized,
            "grouped sibling must share minimize state"
        );

        assert!(workspace.restore_window("codex-1"));
        assert!(!workspace.window("codex-1").expect("codex").minimized);
        assert!(
            !workspace.window("claude-1").expect("claude").minimized,
            "grouped sibling must restore together"
        );
    }

    #[test]
    fn activate_window_tab_preserves_grouped_chrome_state() {
        let mut workspace = WindowCanvasState::from_persisted(default_workspace_state());
        assert!(workspace.dock_window_tab("codex-1", "claude-1"));

        let new_geometry = WindowGeometry {
            x: 320.0,
            y: 180.0,
            width: 960.0,
            height: 600.0,
        };
        assert!(workspace.update_geometry("codex-1", new_geometry.clone()));
        let snapshot = workspace.window("codex-1").expect("codex").clone();

        assert!(workspace.activate_window_tab("claude-1"));
        let claude = workspace.window("claude-1").expect("claude");
        let codex = workspace.window("codex-1").expect("codex");
        assert!(claude.tab_group_active);
        assert!(!codex.tab_group_active);
        assert_eq!(claude.geometry, new_geometry);
        assert_eq!(codex.geometry, new_geometry);
        assert_eq!(claude.maximized, snapshot.maximized);
        assert_eq!(codex.maximized, snapshot.maximized);
        assert_eq!(claude.minimized, snapshot.minimized);
        assert_eq!(codex.minimized, snapshot.minimized);
        assert_eq!(claude.pre_maximize_geometry, snapshot.pre_maximize_geometry);
        assert_eq!(codex.pre_maximize_geometry, snapshot.pre_maximize_geometry);
    }
}
