//! Settings Screen
//!
//! Includes custom agent management (SPEC-71f2742d US3)

#![allow(dead_code)] // Screen components for future use

use gwt_core::config::{CustomCodingAgent, Settings, ToolsConfig};
use ratatui::{prelude::*, widgets::*};

/// Settings categories
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsCategory {
    General,
    Worktree,
    Web,
    Agent,
    /// Custom coding agents management (SPEC-71f2742d US3)
    CustomAgents,
}

/// Custom agent edit mode (T310, T311, T312)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CustomAgentMode {
    /// Viewing list of agents
    #[default]
    List,
    /// Adding a new agent
    Add,
    /// Editing an existing agent
    Edit(String), // agent id
    /// Confirming deletion
    ConfirmDelete(String), // agent id
}

/// Settings state
#[derive(Debug)]
pub struct SettingsState {
    pub category: SettingsCategory,
    pub selected_item: usize,
    pub settings: Option<Settings>,
    pub error_message: Option<String>,
    /// Custom agents configuration (SPEC-71f2742d T308)
    pub tools_config: Option<ToolsConfig>,
    /// Custom agent mode (list/add/edit/delete)
    pub custom_agent_mode: CustomAgentMode,
    /// Selected custom agent index
    pub custom_agent_index: usize,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            category: SettingsCategory::General,
            selected_item: 0,
            settings: None,
            error_message: None,
            tools_config: None,
            custom_agent_mode: CustomAgentMode::default(),
            custom_agent_index: 0,
        }
    }
}

impl SettingsState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_settings(mut self, settings: Settings) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Set tools configuration (SPEC-71f2742d T308)
    pub fn with_tools_config(mut self, tools_config: ToolsConfig) -> Self {
        self.tools_config = Some(tools_config);
        self
    }

    /// Load tools configuration from global file
    /// Settings screen edits global tools.json (~/.gwt/tools.json)
    pub fn load_tools_config(&mut self) {
        self.tools_config = ToolsConfig::load_global();
    }

    /// Get items for current category
    fn category_items(&self) -> Vec<(&'static str, String)> {
        let settings = match &self.settings {
            Some(s) => s,
            None => return vec![],
        };

        match self.category {
            SettingsCategory::General => vec![
                ("Default Base Branch", settings.default_base_branch.clone()),
                ("Debug Mode", format!("{}", settings.debug)),
                (
                    "Log Retention Days",
                    format!("{}", settings.log_retention_days),
                ),
            ],
            SettingsCategory::Worktree => vec![
                ("Worktree Root", settings.worktree_root.clone()),
                ("Protected Branches", settings.protected_branches.join(", ")),
            ],
            SettingsCategory::Web => vec![
                ("Port", format!("{}", settings.web.port)),
                ("Address", settings.web.address.clone()),
                ("CORS Enabled", format!("{}", settings.web.cors)),
            ],
            SettingsCategory::Agent => vec![
                (
                    "Default Agent",
                    settings
                        .agent
                        .default_agent
                        .clone()
                        .unwrap_or_else(|| "None".to_string()),
                ),
                (
                    "Auto Install Deps",
                    format!("{}", settings.agent.auto_install_deps),
                ),
                (
                    "Claude Path",
                    settings
                        .agent
                        .claude_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "Not set".to_string()),
                ),
                (
                    "Codex Path",
                    settings
                        .agent
                        .codex_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "Not set".to_string()),
                ),
            ],
            // CustomAgents uses separate rendering (T309)
            SettingsCategory::CustomAgents => vec![],
        }
    }

    /// Get custom agents list (T309)
    pub fn custom_agents(&self) -> &[CustomCodingAgent] {
        self.tools_config
            .as_ref()
            .map(|c| c.custom_coding_agents.as_slice())
            .unwrap_or(&[])
    }

    /// Select next category
    pub fn next_category(&mut self) {
        self.category = match self.category {
            SettingsCategory::General => SettingsCategory::Worktree,
            SettingsCategory::Worktree => SettingsCategory::Web,
            SettingsCategory::Web => SettingsCategory::Agent,
            SettingsCategory::Agent => SettingsCategory::CustomAgents,
            SettingsCategory::CustomAgents => SettingsCategory::General,
        };
        self.selected_item = 0;
        self.custom_agent_index = 0;
        self.custom_agent_mode = CustomAgentMode::List;
    }

    /// Select previous category
    pub fn prev_category(&mut self) {
        self.category = match self.category {
            SettingsCategory::General => SettingsCategory::CustomAgents,
            SettingsCategory::Worktree => SettingsCategory::General,
            SettingsCategory::Web => SettingsCategory::Worktree,
            SettingsCategory::Agent => SettingsCategory::Web,
            SettingsCategory::CustomAgents => SettingsCategory::Agent,
        };
        self.selected_item = 0;
        self.custom_agent_index = 0;
        self.custom_agent_mode = CustomAgentMode::List;
    }

    /// Select next item
    pub fn select_next(&mut self) {
        if self.category == SettingsCategory::CustomAgents {
            let agents = self.custom_agents();
            // +1 for "Add new agent" option at the end
            let max = agents.len();
            if self.custom_agent_index < max {
                self.custom_agent_index += 1;
            }
        } else {
            let items = self.category_items();
            if !items.is_empty() && self.selected_item < items.len() - 1 {
                self.selected_item += 1;
            }
        }
    }

    /// Select previous item
    pub fn select_prev(&mut self) {
        if self.category == SettingsCategory::CustomAgents {
            if self.custom_agent_index > 0 {
                self.custom_agent_index -= 1;
            }
        } else if self.selected_item > 0 {
            self.selected_item -= 1;
        }
    }

    /// Get selected custom agent (T311)
    pub fn selected_custom_agent(&self) -> Option<&CustomCodingAgent> {
        self.custom_agents().get(self.custom_agent_index)
    }

    /// Check if "Add new agent" option is selected
    pub fn is_add_agent_selected(&self) -> bool {
        self.category == SettingsCategory::CustomAgents
            && self.custom_agent_index == self.custom_agents().len()
    }
}

fn selected_description(state: &SettingsState) -> &'static str {
    match state.category {
        SettingsCategory::General => match state.selected_item {
            0 => "Base branch used for diff checks and cleanup safety.",
            1 => "Enable verbose logging output.",
            2 => "Days to keep logs before pruning.",
            _ => "",
        },
        SettingsCategory::Worktree => match state.selected_item {
            0 => "Relative root directory for worktree creation.",
            1 => "Branches that cannot be deleted.",
            _ => "",
        },
        SettingsCategory::Web => match state.selected_item {
            0 => "HTTP port for the Web UI server.",
            1 => "Bind address for the Web UI server.",
            2 => "Enable CORS for Web UI requests.",
            _ => "",
        },
        SettingsCategory::Agent => match state.selected_item {
            0 => "Default coding agent for quick start.",
            1 => "If false, dependency install is skipped before launch.",
            2 => "Override path to Claude executable.",
            3 => "Override path to Codex executable.",
            _ => "",
        },
        SettingsCategory::CustomAgents => {
            if state.is_add_agent_selected() {
                "Add a new custom coding agent to tools.json."
            } else if let Some(agent) = state.selected_custom_agent() {
                match agent.agent_type {
                    gwt_core::config::AgentType::Command => {
                        "Execute via PATH search. Press Enter to edit, D to delete."
                    }
                    gwt_core::config::AgentType::Path => {
                        "Execute via absolute path. Press Enter to edit, D to delete."
                    }
                    gwt_core::config::AgentType::Bunx => {
                        "Execute via bunx. Press Enter to edit, D to delete."
                    }
                }
            } else {
                "Manage custom coding agents defined in ~/.gwt/tools.json."
            }
        }
    }
}

/// Render settings screen
pub fn render_settings(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tabs
            Constraint::Min(0),    // Content
            Constraint::Length(3), // Instructions
        ])
        .split(area);

    // Category tabs
    render_tabs(state, frame, chunks[0]);

    // Settings content
    render_settings_content(state, frame, chunks[1]);

    // Instructions
    render_instructions(frame, chunks[2]);
}

fn render_tabs(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let categories = [
        ("General", SettingsCategory::General),
        ("Worktree", SettingsCategory::Worktree),
        ("Web", SettingsCategory::Web),
        ("Agent", SettingsCategory::Agent),
        ("Custom", SettingsCategory::CustomAgents), // T309
    ];

    let titles: Vec<Line> = categories
        .iter()
        .map(|(name, cat)| {
            let style = if *cat == state.category {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::styled(name.to_string(), style)
        })
        .collect();

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" Settings "))
        .highlight_style(Style::default().fg(Color::Cyan))
        .select(match state.category {
            SettingsCategory::General => 0,
            SettingsCategory::Worktree => 1,
            SettingsCategory::Web => 2,
            SettingsCategory::Agent => 3,
            SettingsCategory::CustomAgents => 4,
        });

    frame.render_widget(tabs, area);
}

fn render_settings_content(state: &SettingsState, frame: &mut Frame, area: Rect) {
    // CustomAgents has special rendering (T309)
    if state.category == SettingsCategory::CustomAgents {
        render_custom_agents_content(state, frame, area);
        return;
    }

    let items = state.category_items();

    if items.is_empty() {
        let text = if state.settings.is_none() {
            "Settings not loaded"
        } else {
            "No settings in this category"
        };
        let paragraph = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(paragraph, area);
        return;
    }

    let (list_area, desc_area) = if area.height >= 6 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, (name, value))| {
            let content = format!("  {}: {}", name, value);
            let style = if i == state.selected_item {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let category_name = match state.category {
        SettingsCategory::General => "General",
        SettingsCategory::Worktree => "Worktree",
        SettingsCategory::Web => "Web UI",
        SettingsCategory::Agent => "Agent",
        SettingsCategory::CustomAgents => "Custom Agents", // Handled separately
    };

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} Settings ", category_name)),
    );
    frame.render_widget(list, list_area);

    if let Some(desc_area) = desc_area {
        let description = selected_description(state);
        let paragraph = Paragraph::new(description).wrap(Wrap { trim: true }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Description "),
        );
        frame.render_widget(paragraph, desc_area);
    }
}

/// Render custom agents list (T309)
fn render_custom_agents_content(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let agents = state.custom_agents();

    let (list_area, desc_area) = if area.height >= 6 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let mut list_items: Vec<ListItem> = agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let type_str = match agent.agent_type {
                gwt_core::config::AgentType::Command => "cmd",
                gwt_core::config::AgentType::Path => "path",
                gwt_core::config::AgentType::Bunx => "bunx",
            };
            let content = format!(
                "  {} [{}] - {}",
                agent.display_name, type_str, agent.command
            );
            let style = if i == state.custom_agent_index {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    // Add "Add new agent" option at the end
    let add_selected = state.is_add_agent_selected();
    let add_style = if add_selected {
        Style::default()
            .add_modifier(Modifier::REVERSED)
            .fg(Color::Green)
    } else {
        Style::default().fg(Color::Green)
    };
    list_items.push(ListItem::new("  + Add new custom agent...").style(add_style));

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Custom Coding Agents "),
    );
    frame.render_widget(list, list_area);

    if let Some(desc_area) = desc_area {
        let description = selected_description(state);
        let paragraph = Paragraph::new(description).wrap(Wrap { trim: true }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Description "),
        );
        frame.render_widget(paragraph, desc_area);
    }
}

fn render_instructions(frame: &mut Frame, area: Rect) {
    let instructions = "[Tab] Category | [Up/Down] Select | [Enter] Edit | [Esc] Back";
    let paragraph =
        Paragraph::new(format!(" {} ", instructions)).block(Block::default().borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_category_navigation() {
        let mut state = SettingsState::new();
        assert_eq!(state.category, SettingsCategory::General);

        state.next_category();
        assert_eq!(state.category, SettingsCategory::Worktree);

        state.next_category();
        assert_eq!(state.category, SettingsCategory::Web);

        state.prev_category();
        assert_eq!(state.category, SettingsCategory::Worktree);
    }

    #[test]
    fn test_selected_description_auto_install_deps() {
        let mut state = SettingsState::new();
        state.category = SettingsCategory::Agent;
        state.selected_item = 1;
        assert_eq!(
            selected_description(&state),
            "If false, dependency install is skipped before launch."
        );
    }
}
