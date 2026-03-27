//! Management UI components

pub mod agent_list;
pub mod detail_panel;
pub mod issue_panel;
pub mod launch_dialog;
pub mod pr_dashboard;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::Color,
};

/// Status of an agent process.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Running,
    Idle,
    Completed(i32),
    Error(String),
}

impl AgentStatus {
    /// Map this status to a display color.
    pub fn color(&self) -> Color {
        match self {
            AgentStatus::Running => Color::Green,
            AgentStatus::Idle => Color::Yellow,
            AgentStatus::Completed(_) => Color::Cyan,
            AgentStatus::Error(_) => Color::Red,
        }
    }

    /// Short text label for list display.
    pub fn label(&self) -> &str {
        match self {
            AgentStatus::Running => "running",
            AgentStatus::Idle => "idle",
            AgentStatus::Completed(_) => "done",
            AgentStatus::Error(_) => "error",
        }
    }
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Running => write!(f, "Running"),
            AgentStatus::Idle => write!(f, "Idle"),
            AgentStatus::Completed(code) => write!(f, "Completed ({})", code),
            AgentStatus::Error(msg) => write!(f, "Error: {}", msg),
        }
    }
}

/// A single agent entry shown in the management panel.
#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub pane_id: String,
    pub agent_name: String,
    pub agent_type: String,
    pub branch: Option<String>,
    pub status: AgentStatus,
    pub uptime: Option<std::time::Duration>,
    pub pr_url: Option<String>,
    pub spec_id: Option<String>,
}

/// State for the management panel.
#[derive(Debug, Default)]
pub struct ManagementState {
    pub agents: Vec<AgentEntry>,
    pub selected_index: usize,
    pub show_launch_dialog: bool,
    pub launch_dialog: launch_dialog::LaunchDialogState,
}

impl ManagementState {
    /// Move selection to the next agent (wraps around).
    pub fn select_next(&mut self) {
        if self.agents.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.agents.len();
    }

    /// Move selection to the previous agent (wraps around).
    pub fn select_prev(&mut self) {
        if self.agents.is_empty() {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            self.agents.len() - 1
        } else {
            self.selected_index - 1
        };
    }

    /// Returns a reference to the currently selected agent, if any.
    pub fn selected_agent(&self) -> Option<&AgentEntry> {
        self.agents.get(self.selected_index)
    }
}

/// Render the management panel into the given area.
///
/// Splits horizontally: left 35% for agent list, right 65% for detail panel.
pub fn render(buf: &mut Buffer, area: Rect, state: &ManagementState) {
    let layout = Layout::horizontal([
        Constraint::Percentage(35),
        Constraint::Percentage(65),
    ])
    .split(area);

    agent_list::render(buf, layout[0], state);
    detail_panel::render(buf, layout[1], state);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(name: &str, status: AgentStatus) -> AgentEntry {
        AgentEntry {
            pane_id: format!("pane-{}", name),
            agent_name: name.to_string(),
            agent_type: "claude".to_string(),
            branch: Some("main".to_string()),
            status,
            uptime: None,
            pr_url: None,
            spec_id: None,
        }
    }

    #[test]
    fn test_management_state_select_next_wraps() {
        let mut state = ManagementState {
            agents: vec![
                make_entry("a", AgentStatus::Running),
                make_entry("b", AgentStatus::Idle),
                make_entry("c", AgentStatus::Running),
            ],
            selected_index: 0,
            ..Default::default()
        };

        state.select_next();
        assert_eq!(state.selected_index, 1);
        state.select_next();
        assert_eq!(state.selected_index, 2);
        // Wraps around
        state.select_next();
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_management_state_select_prev_wraps() {
        let mut state = ManagementState {
            agents: vec![
                make_entry("a", AgentStatus::Running),
                make_entry("b", AgentStatus::Idle),
            ],
            selected_index: 0,
            ..Default::default()
        };

        // Wraps to end
        state.select_prev();
        assert_eq!(state.selected_index, 1);
        state.select_prev();
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_selected_agent_empty_list() {
        let state = ManagementState::default();
        assert!(state.selected_agent().is_none());
    }

    #[test]
    fn test_select_next_on_empty_list() {
        let mut state = ManagementState::default();
        state.select_next(); // Should not panic
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_select_prev_on_empty_list() {
        let mut state = ManagementState::default();
        state.select_prev(); // Should not panic
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_management_render_split_layout() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 20));
        let state = ManagementState::default();
        render(&mut buf, Rect::new(0, 0, 100, 20), &state);
        // Verify render completes without panic; left block title "Agents"
        // and right block title "Agent Details" should be present.
        let content: String = (0..100)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(content.contains("Agents"));
        assert!(content.contains("Agent Details"));
    }
}
