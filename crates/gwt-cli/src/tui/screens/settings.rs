//! Settings Screen
//!
//! Includes custom agent management (SPEC-71f2742d US3)

#![allow(dead_code)] // Screen components for future use

use gwt_core::config::{AgentType, CustomCodingAgent, Settings, ToolsConfig};
use ratatui::{prelude::*, widgets::*};
use std::collections::HashMap;

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

/// Form field for custom agent (T310, T311)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentFormField {
    #[default]
    Id,
    DisplayName,
    Type,
    Command,
}

impl AgentFormField {
    pub fn all() -> &'static [AgentFormField] {
        &[
            AgentFormField::Id,
            AgentFormField::DisplayName,
            AgentFormField::Type,
            AgentFormField::Command,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            AgentFormField::Id => "ID",
            AgentFormField::DisplayName => "Display Name",
            AgentFormField::Type => "Type",
            AgentFormField::Command => "Command",
        }
    }
}

/// Custom agent form state (T310, T311)
#[derive(Debug, Clone, Default)]
pub struct AgentFormState {
    pub id: String,
    pub display_name: String,
    pub agent_type: AgentType,
    pub command: String,
    pub current_field: AgentFormField,
    pub cursor: usize,
}

impl AgentFormState {
    /// Create form for new agent
    pub fn new() -> Self {
        Self {
            agent_type: AgentType::Command,
            ..Default::default()
        }
    }

    /// Create form from existing agent
    pub fn from_agent(agent: &CustomCodingAgent) -> Self {
        Self {
            id: agent.id.clone(),
            display_name: agent.display_name.clone(),
            agent_type: agent.agent_type,
            command: agent.command.clone(),
            current_field: AgentFormField::Id,
            cursor: agent.id.len(),
        }
    }

    /// Build CustomCodingAgent from form
    pub fn to_agent(&self) -> CustomCodingAgent {
        CustomCodingAgent {
            id: self.id.clone(),
            display_name: self.display_name.clone(),
            agent_type: self.agent_type,
            command: self.command.clone(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        }
    }

    /// Get current field value
    fn current_value(&self) -> &str {
        match self.current_field {
            AgentFormField::Id => &self.id,
            AgentFormField::DisplayName => &self.display_name,
            AgentFormField::Type => "", // Type uses selection, not text
            AgentFormField::Command => &self.command,
        }
    }

    /// Get mutable reference to current field value
    fn current_value_mut(&mut self) -> Option<&mut String> {
        match self.current_field {
            AgentFormField::Id => Some(&mut self.id),
            AgentFormField::DisplayName => Some(&mut self.display_name),
            AgentFormField::Type => None, // Type uses selection
            AgentFormField::Command => Some(&mut self.command),
        }
    }

    /// Move to next field
    pub fn next_field(&mut self) {
        let fields = AgentFormField::all();
        let idx = fields
            .iter()
            .position(|f| *f == self.current_field)
            .unwrap_or(0);
        self.current_field = fields[(idx + 1) % fields.len()];
        self.cursor = self.current_value().len();
    }

    /// Move to previous field
    pub fn prev_field(&mut self) {
        let fields = AgentFormField::all();
        let idx = fields
            .iter()
            .position(|f| *f == self.current_field)
            .unwrap_or(0);
        self.current_field = fields[(idx + fields.len() - 1) % fields.len()];
        self.cursor = self.current_value().len();
    }

    /// Insert character at cursor
    pub fn insert_char(&mut self, c: char) {
        let cursor = self.cursor;
        if let Some(value) = self.current_value_mut() {
            value.insert(cursor, c);
        }
        self.cursor += 1;
    }

    /// Delete character before cursor
    pub fn delete_char(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let cursor = self.cursor;
            if let Some(value) = self.current_value_mut() {
                value.remove(cursor);
            }
        }
    }

    /// Cycle agent type (for Type field)
    pub fn cycle_type(&mut self) {
        self.agent_type = match self.agent_type {
            AgentType::Command => AgentType::Path,
            AgentType::Path => AgentType::Bunx,
            AgentType::Bunx => AgentType::Command,
        };
    }

    /// Validate form
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.is_empty() {
            return Err("ID is required");
        }
        if !self.id.chars().all(|c| c.is_alphanumeric() || c == '-') {
            return Err("ID must be alphanumeric with hyphens only");
        }
        if self.display_name.is_empty() {
            return Err("Display Name is required");
        }
        if self.command.is_empty() {
            return Err("Command is required");
        }
        Ok(())
    }
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
    /// Form state for add/edit (T310, T311)
    pub agent_form: AgentFormState,
    /// Delete confirmation selection (true = Yes, false = No) (T312)
    pub delete_confirm: bool,
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
            agent_form: AgentFormState::default(),
            delete_confirm: false,
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

    /// Enter add mode (T310)
    pub fn enter_add_mode(&mut self) {
        self.agent_form = AgentFormState::new();
        self.custom_agent_mode = CustomAgentMode::Add;
    }

    /// Enter edit mode for selected agent (T311)
    pub fn enter_edit_mode(&mut self) {
        if let Some(agent) = self.selected_custom_agent() {
            let id = agent.id.clone();
            self.agent_form = AgentFormState::from_agent(agent);
            self.custom_agent_mode = CustomAgentMode::Edit(id);
        }
    }

    /// Enter delete confirmation mode (T312)
    pub fn enter_delete_mode(&mut self) {
        if let Some(agent) = self.selected_custom_agent() {
            self.custom_agent_mode = CustomAgentMode::ConfirmDelete(agent.id.clone());
            self.delete_confirm = false;
        }
    }

    /// Cancel current mode and return to list
    pub fn cancel_mode(&mut self) {
        self.custom_agent_mode = CustomAgentMode::List;
        self.agent_form = AgentFormState::default();
        self.delete_confirm = false;
    }

    /// Save agent from form (returns true if successful)
    pub fn save_agent(&mut self) -> Result<(), &'static str> {
        self.agent_form.validate()?;
        let agent = self.agent_form.to_agent();

        match &self.custom_agent_mode {
            CustomAgentMode::Add => {
                if let Some(ref mut config) = self.tools_config {
                    if !config.add_agent(agent) {
                        return Err("Agent with this ID already exists");
                    }
                } else {
                    let mut config = ToolsConfig::empty();
                    config.add_agent(agent);
                    self.tools_config = Some(config);
                }
            }
            CustomAgentMode::Edit(_) => {
                if let Some(ref mut config) = self.tools_config {
                    if !config.update_agent(agent) {
                        return Err("Agent not found");
                    }
                }
            }
            _ => return Err("Invalid mode for save"),
        }

        self.cancel_mode();
        Ok(())
    }

    /// Delete selected agent (returns true if successful)
    pub fn delete_agent(&mut self) -> bool {
        if let CustomAgentMode::ConfirmDelete(ref id) = self.custom_agent_mode {
            let id = id.clone();
            if let Some(ref mut config) = self.tools_config {
                if config.remove_agent(&id) {
                    // Adjust index if needed
                    if self.custom_agent_index > 0
                        && self.custom_agent_index >= config.custom_coding_agents.len()
                    {
                        self.custom_agent_index =
                            config.custom_coding_agents.len().saturating_sub(1);
                    }
                    self.cancel_mode();
                    return true;
                }
            }
        }
        false
    }

    /// Check if in form mode
    pub fn is_form_mode(&self) -> bool {
        matches!(
            self.custom_agent_mode,
            CustomAgentMode::Add | CustomAgentMode::Edit(_)
        )
    }

    /// Check if in delete confirmation mode
    pub fn is_delete_mode(&self) -> bool {
        matches!(self.custom_agent_mode, CustomAgentMode::ConfirmDelete(_))
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
    render_instructions(state, frame, chunks[2]);
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

/// Render custom agents content based on mode (T309, T310, T311, T312)
fn render_custom_agents_content(state: &SettingsState, frame: &mut Frame, area: Rect) {
    match &state.custom_agent_mode {
        CustomAgentMode::List => render_custom_agents_list(state, frame, area),
        CustomAgentMode::Add | CustomAgentMode::Edit(_) => {
            render_agent_form(state, frame, area);
        }
        CustomAgentMode::ConfirmDelete(_) => {
            render_delete_confirmation(state, frame, area);
        }
    }
}

/// Render custom agents list (T309)
fn render_custom_agents_list(state: &SettingsState, frame: &mut Frame, area: Rect) {
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

/// Render agent form for add/edit (T310, T311)
fn render_agent_form(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let form = &state.agent_form;
    let is_edit = matches!(state.custom_agent_mode, CustomAgentMode::Edit(_));
    let title = if is_edit {
        " Edit Custom Agent "
    } else {
        " Add Custom Agent "
    };

    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Form fields layout
    let field_height = 3;
    let fields = AgentFormField::all();
    let constraints: Vec<Constraint> = fields
        .iter()
        .map(|_| Constraint::Length(field_height))
        .chain(std::iter::once(Constraint::Min(0)))
        .collect();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(inner);

    for (i, field) in fields.iter().enumerate() {
        let is_selected = *field == form.current_field;
        let label = field.label();

        let (value, show_cursor) = match field {
            AgentFormField::Id => (form.id.as_str(), is_selected),
            AgentFormField::DisplayName => (form.display_name.as_str(), is_selected),
            AgentFormField::Type => {
                let type_str = match form.agent_type {
                    AgentType::Command => "command (PATH search)",
                    AgentType::Path => "path (absolute path)",
                    AgentType::Bunx => "bunx (bunx execution)",
                };
                (type_str, false)
            }
            AgentFormField::Command => (form.command.as_str(), is_selected),
        };

        // Build display text with cursor
        let display_text = if show_cursor {
            let mut text = String::from(value);
            // Insert cursor indicator
            let cursor_pos = form.cursor.min(text.len());
            text.insert(cursor_pos, '|');
            text
        } else {
            value.to_string()
        };

        let field_style = if is_selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        let field_block = Block::default()
            .borders(Borders::ALL)
            .border_style(field_style)
            .title(format!(" {} ", label));

        let hint = if is_selected && *field == AgentFormField::Type {
            " (Space/Enter to cycle)"
        } else {
            ""
        };

        let paragraph = Paragraph::new(format!("{}{}", display_text, hint)).block(field_block);
        frame.render_widget(paragraph, chunks[i]);
    }
}

/// Render delete confirmation dialog (T312)
fn render_delete_confirmation(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let agent_id = match &state.custom_agent_mode {
        CustomAgentMode::ConfirmDelete(id) => id.as_str(),
        _ => return,
    };

    // Find agent display name
    let display_name = state
        .custom_agents()
        .iter()
        .find(|a| a.id == agent_id)
        .map(|a| a.display_name.as_str())
        .unwrap_or(agent_id);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Delete Custom Agent ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2), // Question
            Constraint::Length(3), // Buttons
            Constraint::Min(0),    // Padding
        ])
        .split(inner);

    // Question
    let question = Paragraph::new(format!(
        "Are you sure you want to delete '{}'?",
        display_name
    ))
    .alignment(Alignment::Center);
    frame.render_widget(question, chunks[0]);

    // Buttons
    let button_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(30),
        ])
        .split(chunks[1]);

    let yes_style = if state.delete_confirm {
        Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(Color::Red)
    };

    let no_style = if !state.delete_confirm {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(Color::Green)
    };

    let yes_btn = Paragraph::new(" Yes ")
        .alignment(Alignment::Center)
        .style(yes_style)
        .block(Block::default().borders(Borders::ALL));
    let no_btn = Paragraph::new(" No ")
        .alignment(Alignment::Center)
        .style(no_style)
        .block(Block::default().borders(Borders::ALL));

    frame.render_widget(yes_btn, button_chunks[1]);
    frame.render_widget(no_btn, button_chunks[2]);
}

fn render_instructions(state: &SettingsState, frame: &mut Frame, area: Rect) {
    // FR-020: Tab cycles screens, Left/Right cycles categories
    let instructions = if state.category == SettingsCategory::CustomAgents {
        match &state.custom_agent_mode {
            CustomAgentMode::List => {
                if state.is_add_agent_selected() {
                    "[Enter] Add | [L/R] Category | [U/D] Select | [Tab] Screen | [Esc] Back"
                } else {
                    "[Enter] Edit | [D] Delete | [L/R] Cat | [U/D] Sel | [Tab] Scr | [Esc] Back"
                }
            }
            CustomAgentMode::Add | CustomAgentMode::Edit(_) => {
                "[Tab/Up/Down] Field | [Space] Type | [Enter] Save | [Esc] Cancel"
            }
            CustomAgentMode::ConfirmDelete(_) => {
                "[Left/Right] Select | [Enter] Confirm | [Esc] Cancel"
            }
        }
    } else {
        "[Left/Right] Category | [Up/Down] Select | [Tab] Screen | [Esc] Back"
    };
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
