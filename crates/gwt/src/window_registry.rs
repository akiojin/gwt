use crate::{
    persistence::{
        AgentKanbanLane, PersistedWindowRegistry, PersistedWindowState, WindowPlacement,
        WindowProcessStatus, WindowSurfaceOwner, WindowWorktreeForm,
    },
    preset::WindowPreset,
    protocol::FocusCycleDirection,
};

/// Window identity and metadata, independent of surface layout and terminal size.
#[derive(Debug, Clone)]
pub struct WindowRegistry {
    persisted: PersistedWindowRegistry,
}

impl WindowRegistry {
    /// Merge presentation changes without admitting, replacing, or deleting a
    /// process identity. Only owner lifecycle commands change those rows.
    pub fn merge_owner_layout(&mut self, proposal: &PersistedWindowRegistry) -> bool {
        let before = self.persisted.clone();
        let process = |window: &PersistedWindowState| {
            window.preset.requires_process()
                || window.agent_id.is_some()
                || window.session_id.is_some()
        };
        // Missing process/husk rows are stale presentation, never close intent.
        // Ordinary UI-only rows still follow the GUI's latest surface layout.
        self.persisted.windows.retain(|window| {
            process(window)
                || proposal
                    .windows
                    .iter()
                    .any(|proposed| proposed.id == window.id && !process(proposed))
        });
        let mut seen = std::collections::HashSet::new();
        for proposed in &proposal.windows {
            if !seen.insert(proposed.id.as_str()) {
                continue;
            }
            match self
                .persisted
                .windows
                .iter_mut()
                .find(|window| window.id == proposed.id)
            {
                Some(current) if process(current) => {
                    current.placement = proposed.placement.clone();
                    current.surface_owner = proposed.surface_owner;
                    current.z_index = proposed.z_index;
                }
                Some(current) if !process(proposed) => {
                    *current = proposed.clone();
                }
                Some(_) => {}
                None if !process(proposed) => {
                    self.persisted.windows.push(proposed.clone());
                }
                None => {}
            }
        }
        self.persisted.next_z_index = self
            .persisted
            .windows
            .iter()
            .map(|window| window.z_index)
            .max()
            .unwrap_or_default()
            .saturating_add(1);
        self.persisted != before
    }

    pub fn from_persisted(persisted: PersistedWindowRegistry) -> Self {
        Self { persisted }
    }

    pub fn persisted(&self) -> &PersistedWindowRegistry {
        &self.persisted
    }

    pub fn persistable_state(&self) -> PersistedWindowRegistry {
        let mut persisted = self.persisted.clone();
        persisted.windows.retain(|window| window.persist);
        persisted.next_z_index = persisted.windows.len() as u32 + 1;
        persisted
    }

    pub fn window(&self, id: &str) -> Option<&PersistedWindowState> {
        self.persisted.windows.iter().find(|window| window.id == id)
    }

    pub fn set_surface_owner(&mut self, id: &str, owner: Option<WindowSurfaceOwner>) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        let window = &mut self.persisted.windows[index];
        if window.surface_owner == owner {
            return false;
        }
        window.surface_owner = owner;
        true
    }

    /// Changes snapshot eligibility without replacing the in-memory window.
    /// Returns true only when the persist flag changed.
    pub fn set_persist(&mut self, id: &str, persist: bool) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        let window = &mut self.persisted.windows[index];
        if window.persist == persist {
            return false;
        }
        window.persist = persist;
        true
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

    pub fn set_agent_name(&mut self, id: &str, name: Option<String>) -> bool {
        let Some(window) = self
            .persisted
            .windows
            .iter_mut()
            .find(|window| window.id == id)
        else {
            return false;
        };
        window.agent_name = name;
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

    pub fn focus_window(&mut self, id: &str) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if !self.persisted.windows[index].placement.is_surface() {
            return false;
        }
        self.bring_to_front(index);
        true
    }

    pub fn cycle_focus(&mut self, direction: FocusCycleDirection) -> Option<String> {
        let eligible = self
            .persisted
            .windows
            .iter()
            .enumerate()
            .filter_map(|(index, window)| window.placement.is_surface().then_some(index))
            .collect::<Vec<_>>();
        if eligible.len() == 1 {
            return Some(self.persisted.windows[eligible[0]].id.clone());
        }
        let current = eligible
            .iter()
            .copied()
            .max_by_key(|&index| self.persisted.windows[index].z_index)?;
        let position = eligible.iter().position(|&index| index == current)?;
        let next = match direction {
            FocusCycleDirection::Forward => (position + 1) % eligible.len(),
            FocusCycleDirection::Backward => (position + eligible.len() - 1) % eligible.len(),
        };
        let id = self.persisted.windows[eligible[next]].id.clone();
        self.focus_window(&id);
        Some(id)
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
        let window = PersistedWindowState {
            id: self.next_window_id(preset),
            title: title.into(),
            preset,
            z_index: self.persisted.next_z_index,
            status: WindowProcessStatus::Running,
            placement: WindowPlacement::Surface,
            persist,
            purpose_title: None,
            dynamic_title: None,
            dynamic_title_detail: None,
            agent_id: None,
            agent_name: None,
            agent_color: None,
            worktree_form: WindowWorktreeForm::Unknown,
            session_id: None,
            linked_issue_number: None,
            runtime_started_at_ms: None,
            surface_owner: None,
            is_pm: false,
        };
        self.persisted.next_z_index += 1;
        self.persisted.windows.push(window.clone());
        window
    }

    pub fn next_window_id_preview(&self, preset: WindowPreset) -> String {
        self.next_window_id(preset)
    }

    pub fn close_window(&mut self, id: &str) -> bool {
        let initial_len = self.persisted.windows.len();
        if self
            .window(id)
            .is_some_and(|window| window.preset == WindowPreset::AgentKanban)
        {
            self.undock_agent_windows_for_board(id);
        }
        // SPEC-3671: a preview without its Issue window has nowhere to render, so
        // release its containment instead of stranding an invisible pane.
        if self
            .window(id)
            .is_some_and(|window| window.preset.hosts_issue_preview())
        {
            self.release_issue_previews_for_issue_window(id);
        }
        self.persisted.windows.retain(|window| window.id != id);
        self.persisted.windows.len() != initial_len
    }

    pub fn activate_window_tab(&mut self, id: &str) -> bool {
        self.focus_window(id)
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
        self.normalize_agent_kanban_orders(board_id);
        true
    }

    pub fn place_agent_window_in_issue_preview(
        &mut self,
        id: &str,
        issue_window_id: &str,
        issue_number: u64,
    ) -> bool {
        let Some(source_index) = self.window_index(id) else {
            return false;
        };
        let Some(issue_index) = self.window_index(issue_window_id) else {
            return false;
        };
        if source_index == issue_index
            || !self.persisted.windows[source_index]
                .preset
                .is_agent_terminal()
            || !self.persisted.windows[issue_index]
                .preset
                .hosts_issue_preview()
        {
            return false;
        }

        self.persisted.windows[source_index].placement = WindowPlacement::IssuePreview {
            issue_window_id: issue_window_id.to_string(),
            issue_number,
        };
        // SPEC-3885 FR-011: remember the Issue itself, not just where the window is
        // drawn, so Windowize can keep showing it.
        self.persisted.windows[source_index].linked_issue_number = Some(issue_number);
        true
    }

    /// Retain the owning Issue independently of which surface presents the window.
    pub fn set_linked_issue_number(&mut self, id: &str, issue_number: Option<u64>) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        self.persisted.windows[index].linked_issue_number = issue_number;
        true
    }

    /// Prefer an independent Issue host and break ties in stable creation order.
    pub fn issue_preview_host_window_id(&self) -> Option<String> {
        let windows = &self.persisted.windows;
        windows
            .iter()
            .find(|window| window.preset.hosts_issue_preview() && window.placement.is_surface())
            .or_else(|| {
                windows
                    .iter()
                    .find(|window| window.preset.hosts_issue_preview())
            })
            .map(|window| window.id.clone())
    }

    pub fn dock_agent_window_to_issue(&mut self, id: &str) -> Result<(), &'static str> {
        let index = self.window_index(id).ok_or("window no longer exists")?;
        let window = &self.persisted.windows[index];
        let issue_number = window.linked_issue_number.ok_or("no linked Issue")?;
        if !window.preset.is_agent_terminal() || !window.placement.is_surface() {
            return Err("window is not an agent on a surface");
        }
        let issue_window_id = self
            .issue_preview_host_window_id()
            .ok_or("no Issue or Issue Monitor window is open")?;

        self.place_agent_window_in_issue_preview(id, &issue_window_id, issue_number);
        Ok(())
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

    /// Release semantic containment and select the same existing window.
    pub fn undock_agent_window(&mut self, id: &str) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if !self.persisted.windows[index].placement.is_contained() {
            return false;
        }
        self.persisted.windows[index].placement = WindowPlacement::Surface;
        self.bring_to_front(index);
        true
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

    fn window_index(&self, id: &str) -> Option<usize> {
        self.persisted
            .windows
            .iter()
            .position(|window| window.id == id)
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
                window.placement = WindowPlacement::Surface;
            }
        }
    }

    fn release_issue_previews_for_issue_window(&mut self, issue_window_id: &str) {
        for window in &mut self.persisted.windows {
            if matches!(
                &window.placement,
                WindowPlacement::IssuePreview {
                    issue_window_id: host_id,
                    ..
                } if host_id == issue_window_id
            ) {
                window.placement = WindowPlacement::Surface;
            }
        }
    }

    fn bring_to_front(&mut self, index: usize) {
        if let Some(window) = self.persisted.windows.get_mut(index) {
            window.z_index = self.persisted.next_z_index;
            self.persisted.next_z_index += 1;
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
    use crate::persistence::{default_workspace_state, empty_workspace_state};

    #[test]
    fn persist_mutation_retains_window_and_excludes_only_it_from_snapshot() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let agent = workspace.add_window(WindowPreset::Agent);
        let shell = workspace.add_window(WindowPreset::Shell);
        let original = workspace.persisted().clone();

        assert!(workspace.set_persist(&agent.id, false));
        let mut retained = agent.clone();
        retained.persist = false;
        assert_eq!(workspace.window(&agent.id), Some(&retained));
        assert_eq!(workspace.window(&shell.id), Some(&shell));
        assert_eq!(workspace.persistable_state().windows, vec![shell]);
        assert!(!workspace.set_persist(&agent.id, false));

        assert!(workspace.set_persist(&agent.id, true));
        assert!(!workspace.set_persist(&agent.id, true));
        assert_eq!(workspace.persisted(), &original);
        assert_eq!(workspace.persistable_state(), original);
    }

    #[test]
    fn persist_mutation_missing_window_leaves_registry_unchanged() {
        let mut workspace = WindowRegistry::from_persisted(default_workspace_state());
        let original = workspace.persisted().clone();

        assert!(!workspace.set_persist("missing", false));
        assert_eq!(workspace.persisted(), &original);
    }

    #[test]
    fn focusing_window_brings_it_to_front() {
        let mut workspace = WindowRegistry::from_persisted(default_workspace_state());
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
        let mut workspace = WindowRegistry::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::Shell);
        assert_eq!(window.title, "Shell");
        assert_eq!(window.preset, WindowPreset::Shell);
        assert_eq!(window.z_index, 3);
        assert_eq!(workspace.persisted().windows.len(), 3);
        assert_eq!(workspace.persisted().next_z_index, 4);
        assert_eq!(window.status, WindowProcessStatus::Running);
    }

    #[test]
    fn surface_owner_is_retained_by_registry_snapshot() {
        let mut value = serde_json::to_value(default_workspace_state()).expect("workspace");
        value["windows"][0]["surface_owner"] = serde_json::json!("pm");
        let state = serde_json::from_value(value).expect("owned window");
        let registry = WindowRegistry::from_persisted(state);
        let snapshot = serde_json::to_value(registry.persistable_state()).expect("snapshot");
        assert_eq!(snapshot["windows"][0]["surface_owner"], "pm");
        assert!(!registry.persisted().windows[0].is_pm);
    }

    #[test]
    fn adding_board_window_marks_it_running_without_process() {
        let mut workspace = WindowRegistry::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::Board);
        assert_eq!(window.title, "Board");
        assert_eq!(window.preset, WindowPreset::Board);
        assert_eq!(window.status, WindowProcessStatus::Running);
    }

    #[test]
    fn adding_branches_window_marks_it_running_without_process() {
        let mut workspace = WindowRegistry::from_persisted(default_workspace_state());
        let window = workspace.add_window(WindowPreset::Branches);
        assert_eq!(window.title, "Branches");
        assert_eq!(window.preset, WindowPreset::Branches);
        assert_eq!(window.status, WindowProcessStatus::Running);
    }

    #[test]
    fn placing_agent_window_in_kanban_sets_containment_placement() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::AgentKanban);
        let agent = workspace.add_window(WindowPreset::Agent);

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
    }

    #[test]
    fn agent_kanban_rejects_non_agent_sources() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::AgentKanban);
        let shell = workspace.add_window(WindowPreset::Shell);
        let claude = workspace.add_window(WindowPreset::Claude);
        let codex = workspace.add_window(WindowPreset::Codex);

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
    }

    #[test]
    fn focus_cycle_skips_agent_kanban_contained_agents() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::AgentKanban);
        let agent = workspace.add_window(WindowPreset::Agent);
        assert!(workspace.place_agent_window_in_kanban(
            &agent.id,
            &board.id,
            AgentKanbanLane::Active,
            None,
        ));

        let focused = workspace.cycle_focus(FocusCycleDirection::Forward);

        assert_eq!(focused.as_deref(), Some(board.id.as_str()));
    }

    #[test]
    fn closing_agent_kanban_window_undocks_contained_agents() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::AgentKanban);
        let agent = workspace.add_window(WindowPreset::Agent);
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
            WindowPlacement::Surface
        );
    }

    #[test]
    fn placing_agent_window_in_issue_preview_sets_containment_placement() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let issue = workspace.add_window(WindowPreset::Settings);
        let agent = workspace.add_window(WindowPreset::Agent);

        assert!(workspace.place_agent_window_in_issue_preview(&agent.id, &issue.id, 3671));

        let agent = workspace.window(&agent.id).expect("agent window");
        assert_eq!(
            agent.placement,
            WindowPlacement::IssuePreview {
                issue_window_id: issue.id.clone(),
                issue_number: 3671,
            }
        );
        assert!(agent.placement.is_contained());
    }

    #[test]
    fn issue_preview_rejects_non_agent_sources_and_non_issue_hosts() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let issue = workspace.add_window(WindowPreset::Issue);
        let board = workspace.add_window(WindowPreset::Board);
        let shell = workspace.add_window(WindowPreset::Shell);
        let agent = workspace.add_window(WindowPreset::Agent);

        assert!(!workspace.place_agent_window_in_issue_preview(&shell.id, &issue.id, 3671));
        assert!(!workspace.place_agent_window_in_issue_preview(&agent.id, &board.id, 3671));
        assert!(!workspace.place_agent_window_in_issue_preview(&agent.id, "missing-window", 3671));
        assert_eq!(
            workspace.window(&agent.id).expect("agent").placement,
            WindowPlacement::Surface
        );
    }

    #[test]
    fn undocking_issue_preview_window_returns_it_to_a_surface() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let issue = workspace.add_window(WindowPreset::Issue);
        let agent = workspace.add_window(WindowPreset::Agent);
        assert!(workspace.place_agent_window_in_issue_preview(&agent.id, &issue.id, 3671));

        assert!(workspace.undock_agent_window(&agent.id));

        let undocked = workspace.window(&agent.id).expect("agent");
        assert_eq!(undocked.placement, WindowPlacement::Surface);
        assert_eq!(
            undocked.z_index,
            workspace.persisted().next_z_index - 1,
            "Windowize must focus the window it puts back on a surface"
        );
    }

    #[test]
    fn windowized_agent_window_keeps_its_linked_issue() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let issue = workspace.add_window(WindowPreset::Issue);
        let agent = workspace.add_window(WindowPreset::Agent);
        assert!(workspace.place_agent_window_in_issue_preview(&agent.id, &issue.id, 3885));
        assert_eq!(
            workspace
                .window(&agent.id)
                .expect("agent")
                .linked_issue_number,
            Some(3885)
        );

        assert!(workspace.undock_agent_window(&agent.id));

        let undocked = workspace.window(&agent.id).expect("agent");
        assert_eq!(undocked.placement, WindowPlacement::Surface);
        assert_eq!(undocked.linked_issue_number, Some(3885));
    }

    #[test]
    fn docking_agent_window_back_to_issue_returns_it_to_the_preview() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let issue = workspace.add_window(WindowPreset::Settings);
        let agent = workspace.add_window(WindowPreset::Agent);
        assert!(workspace.place_agent_window_in_issue_preview(&agent.id, &issue.id, 3885));
        assert!(workspace.undock_agent_window(&agent.id));

        assert!(workspace.dock_agent_window_to_issue(&agent.id).is_ok());

        assert_eq!(
            workspace.window(&agent.id).expect("agent").placement,
            WindowPlacement::IssuePreview {
                issue_window_id: issue.id.clone(),
                issue_number: 3885,
            }
        );
    }

    #[test]
    fn docking_agent_window_without_a_linked_issue_is_refused() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let _issue = workspace.add_window(WindowPreset::Issue);
        let agent = workspace.add_window(WindowPreset::Agent);

        assert!(workspace.dock_agent_window_to_issue(&agent.id).is_err());
        assert_eq!(
            workspace.window(&agent.id).expect("agent").placement,
            WindowPlacement::Surface
        );
        assert_eq!(
            workspace
                .window(&agent.id)
                .expect("agent")
                .linked_issue_number,
            None
        );
    }

    #[test]
    fn docking_agent_window_without_an_issue_host_is_refused() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let issue = workspace.add_window(WindowPreset::Issue);
        let agent = workspace.add_window(WindowPreset::Agent);
        assert!(workspace.place_agent_window_in_issue_preview(&agent.id, &issue.id, 3885));
        assert!(workspace.undock_agent_window(&agent.id));
        assert!(workspace.close_window(&issue.id));

        assert!(workspace.dock_agent_window_to_issue(&agent.id).is_err());
        assert_eq!(
            workspace.window(&agent.id).expect("agent").placement,
            WindowPlacement::Surface
        );
    }

    #[test]
    fn issue_preview_host_prefers_a_surface_issue_window() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::Board);
        let issue = workspace.add_window(WindowPreset::Issue);

        assert_eq!(
            workspace.issue_preview_host_window_id(),
            Some(issue.id.clone())
        );
        assert!(workspace.close_window(&board.id));
        assert!(workspace.close_window(&issue.id));
        assert_eq!(workspace.issue_preview_host_window_id(), None);
    }

    #[test]
    fn closing_issue_window_releases_its_previews_to_a_surface() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let issue = workspace.add_window(WindowPreset::Settings);
        let other_issue = workspace.add_window(WindowPreset::Settings);
        let agent = workspace.add_window(WindowPreset::Agent);
        let kept = workspace.add_window(WindowPreset::Agent);
        assert!(workspace.place_agent_window_in_issue_preview(&agent.id, &issue.id, 3671));
        assert!(workspace.place_agent_window_in_issue_preview(&kept.id, &other_issue.id, 3672));

        assert!(workspace.close_window(&issue.id));

        assert_eq!(
            workspace.window(&agent.id).expect("agent").placement,
            WindowPlacement::Surface
        );
        assert_eq!(
            workspace.window(&kept.id).expect("kept agent").placement,
            WindowPlacement::IssuePreview {
                issue_window_id: other_issue.id,
                issue_number: 3672,
            }
        );
    }

    #[test]
    fn focus_cycle_skips_issue_preview_contained_agents() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let issue = workspace.add_window(WindowPreset::Issue);
        let agent = workspace.add_window(WindowPreset::Agent);
        assert!(workspace.place_agent_window_in_issue_preview(&agent.id, &issue.id, 3671));

        let focused = workspace.cycle_focus(FocusCycleDirection::Forward);

        assert_eq!(focused.as_deref(), Some(issue.id.as_str()));
    }

    #[test]
    fn moving_agent_kanban_card_rejects_missing_board() {
        let mut workspace = WindowRegistry::from_persisted(empty_workspace_state());
        let board = workspace.add_window(WindowPreset::AgentKanban);
        let agent = workspace.add_window(WindowPreset::Agent);
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
    fn adding_agent_window_uses_new_id_when_lower_suffix_was_closed() {
        let mut workspace = WindowRegistry::from_persisted(PersistedWindowRegistry {
            windows: vec![PersistedWindowState {
                id: "agent-2".to_string(),
                title: "Agent".to_string(),
                preset: WindowPreset::Agent,
                z_index: 1,
                status: WindowProcessStatus::Running,
                placement: WindowPlacement::Surface,
                persist: false,
                purpose_title: None,
                dynamic_title: None,
                dynamic_title_detail: None,
                agent_id: None,
                agent_name: None,
                agent_color: None,
                worktree_form: WindowWorktreeForm::Unknown,
                session_id: None,
                linked_issue_number: None,
                runtime_started_at_ms: None,
                surface_owner: None,
                is_pm: false,
            }],
            next_z_index: 2,
        });

        let window = workspace.add_window(WindowPreset::Agent);

        assert_eq!(window.id, "agent-3");
        assert_eq!(workspace.persisted().windows.len(), 2);
        assert!(workspace.window("agent-2").is_some());
        assert!(workspace.window("agent-3").is_some());
    }

    #[test]
    fn closing_window_removes_it_from_workspace() {
        let mut workspace = WindowRegistry::from_persisted(default_workspace_state());
        assert!(workspace.close_window("codex-1"));
        assert_eq!(workspace.persisted().windows.len(), 1);
        assert!(workspace
            .persisted()
            .windows
            .iter()
            .all(|window| window.id != "codex-1"));
    }

    #[test]
    fn cycling_focus_forward_updates_selection_order() {
        let mut workspace = WindowRegistry::from_persisted(default_workspace_state());
        workspace.add_window(WindowPreset::Shell);
        let focused = workspace
            .cycle_focus(FocusCycleDirection::Forward)
            .expect("focused window");
        assert_eq!(focused, "claude-1");
        assert_eq!(
            workspace
                .window("claude-1")
                .expect("selected window")
                .z_index,
            4
        );
        assert_eq!(workspace.persisted().next_z_index, 5);
    }

    #[test]
    fn cycling_focus_backward_updates_selection_order() {
        let mut workspace = WindowRegistry::from_persisted(default_workspace_state());
        workspace.add_window(WindowPreset::Shell);
        let focused = workspace
            .cycle_focus(FocusCycleDirection::Backward)
            .expect("focused window");
        assert_eq!(focused, "codex-1");
        assert_eq!(
            workspace
                .window("codex-1")
                .expect("selected window")
                .z_index,
            4
        );
        assert_eq!(workspace.persisted().next_z_index, 5);
    }
}
