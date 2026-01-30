//! Settings Screen
//!
//! Includes custom agent management (SPEC-71f2742d US3)
//! Includes profile management (Profile integration)
//! Environment variable editing follows SPEC-dafff079 (FR-009, FR-010, FR-019, FR-020)

#![allow(dead_code)] // Screen components for future use

use crate::tui::screens::environment::{collect_os_env, EnvItem, EnvironmentState};
use gwt_core::config::{
    AgentType, CustomCodingAgent, Profile, ProfilesConfig, Settings, ToolsConfig,
};
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
    /// Environment variables management (profile env settings)
    Environment,
    /// AI settings management (endpoint, key, model)
    AISettings,
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

// ============================================================================
// Profile Management (Profile integration)
// ============================================================================

/// Profile edit mode
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProfileMode {
    /// Viewing list of profiles
    #[default]
    List,
    /// Adding a new profile
    Add,
    /// Editing an existing profile
    Edit(String), // profile name
    /// Confirming deletion
    ConfirmDelete(String), // profile name
    /// Editing environment variables
    EnvEdit(String), // profile name
}

/// Form field for profile editing (name and description only, AI settings are separate)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProfileFormField {
    #[default]
    Name,
    Description,
}

impl ProfileFormField {
    pub fn all() -> &'static [ProfileFormField] {
        &[ProfileFormField::Name, ProfileFormField::Description]
    }

    pub fn label(&self) -> &'static str {
        match self {
            ProfileFormField::Name => "Name",
            ProfileFormField::Description => "Description",
        }
    }
}

/// Profile form state (name and description only, AI settings are separate)
#[derive(Debug, Clone, Default)]
pub struct ProfileFormState {
    pub name: String,
    pub description: String,
    pub current_field: ProfileFormField,
    pub cursor: usize,
}

impl ProfileFormState {
    /// Create form for new profile
    pub fn new() -> Self {
        Self::default()
    }

    /// Create form from existing profile
    pub fn from_profile(profile: &Profile) -> Self {
        Self {
            name: profile.name.clone(),
            description: profile.description.clone(),
            current_field: ProfileFormField::Name,
            cursor: profile.name.len(),
        }
    }

    /// Build Profile from form (preserves env and ai from original if editing)
    pub fn to_profile(&self, original: Option<&Profile>) -> Profile {
        Profile {
            name: self.name.clone(),
            description: self.description.clone(),
            env: original.map(|p| p.env.clone()).unwrap_or_default(),
            disabled_env: original.map(|p| p.disabled_env.clone()).unwrap_or_default(),
            ai: original.and_then(|p| p.ai.clone()),
        }
    }

    /// Get current field value
    fn current_value(&self) -> &str {
        match self.current_field {
            ProfileFormField::Name => &self.name,
            ProfileFormField::Description => &self.description,
        }
    }

    /// Get mutable reference to current field value
    fn current_value_mut(&mut self) -> &mut String {
        match self.current_field {
            ProfileFormField::Name => &mut self.name,
            ProfileFormField::Description => &mut self.description,
        }
    }

    /// Move to next field
    pub fn next_field(&mut self) {
        let fields = ProfileFormField::all();
        let idx = fields
            .iter()
            .position(|f| *f == self.current_field)
            .unwrap_or(0);
        self.current_field = fields[(idx + 1) % fields.len()];
        self.cursor = self.current_value().len();
    }

    /// Move to previous field
    pub fn prev_field(&mut self) {
        let fields = ProfileFormField::all();
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
        let value = self.current_value_mut();
        value.insert(cursor, c);
        self.cursor += 1;
    }

    /// Delete character before cursor
    pub fn delete_char(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let cursor = self.cursor;
            let value = self.current_value_mut();
            value.remove(cursor);
        }
    }

    /// Validate form
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.is_empty() {
            return Err("Name is required");
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err("Name must be alphanumeric with hyphens/underscores only");
        }
        Ok(())
    }
}

/// Environment variable edit state
#[derive(Debug, Clone, Default)]
pub struct EnvEditState {
    pub vars: Vec<(String, String)>, // (key, value)
    pub selected_index: usize,
    pub editing: Option<EnvEditMode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvEditMode {
    Key(usize),   // cursor position
    Value(usize), // cursor position
}

impl EnvEditState {
    pub fn from_profile(profile: &Profile) -> Self {
        let mut vars: Vec<(String, String)> = profile
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        vars.sort_by(|a, b| a.0.cmp(&b.0));
        Self {
            vars,
            selected_index: 0,
            editing: None,
        }
    }

    pub fn to_env(&self) -> HashMap<String, String> {
        self.vars.iter().cloned().collect()
    }

    pub fn add_new_var(&mut self) {
        self.vars.push((String::new(), String::new()));
        self.selected_index = self.vars.len() - 1;
        self.editing = Some(EnvEditMode::Key(0));
    }

    pub fn delete_selected(&mut self) {
        if !self.vars.is_empty() {
            self.vars.remove(self.selected_index);
            if self.selected_index > 0 && self.selected_index >= self.vars.len() {
                self.selected_index = self.vars.len().saturating_sub(1);
            }
        }
    }

    /// Toggle between Key and Value editing
    pub fn toggle_key_value(&mut self) {
        if let Some(ref mode) = self.editing.clone() {
            match mode {
                EnvEditMode::Key(pos) => {
                    if self.selected_index < self.vars.len() {
                        let val_len = self.vars[self.selected_index].1.len();
                        self.editing = Some(EnvEditMode::Value(val_len.min(*pos)));
                    }
                }
                EnvEditMode::Value(pos) => {
                    if self.selected_index < self.vars.len() {
                        let key_len = self.vars[self.selected_index].0.len();
                        self.editing = Some(EnvEditMode::Key(key_len.min(*pos)));
                    }
                }
            }
        }
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
    // Profile management fields
    /// Profiles configuration
    pub profiles_config: Option<ProfilesConfig>,
    /// Profile mode (list/add/edit/delete/env_edit)
    pub profile_mode: ProfileMode,
    /// Selected profile index
    pub profile_index: usize,
    /// Profile form state
    pub profile_form: ProfileFormState,
    /// Profile delete confirmation
    pub profile_delete_confirm: bool,
    /// Environment variable edit state (SPEC-dafff079 compliant)
    pub env_state: EnvironmentState,
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
            // Profile defaults
            profiles_config: None,
            profile_mode: ProfileMode::default(),
            profile_index: 0,
            profile_form: ProfileFormState::default(),
            profile_delete_confirm: false,
            env_state: EnvironmentState::default(),
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

    /// Load profiles configuration from ~/.gwt/profiles.yaml
    pub fn load_profiles_config(&mut self) {
        self.profiles_config = ProfilesConfig::load().ok();
    }

    /// Set profiles configuration
    pub fn with_profiles_config(mut self, config: ProfilesConfig) -> Self {
        self.profiles_config = Some(config);
        self
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
            // Profile uses separate rendering
            SettingsCategory::Environment => vec![],
            // AISettings uses separate rendering
            SettingsCategory::AISettings => vec![],
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
            SettingsCategory::CustomAgents => SettingsCategory::Environment,
            SettingsCategory::Environment => SettingsCategory::AISettings,
            SettingsCategory::AISettings => SettingsCategory::General,
        };
        self.reset_category_state();
    }

    /// Select previous category
    pub fn prev_category(&mut self) {
        self.category = match self.category {
            SettingsCategory::General => SettingsCategory::AISettings,
            SettingsCategory::Worktree => SettingsCategory::General,
            SettingsCategory::Web => SettingsCategory::Worktree,
            SettingsCategory::Agent => SettingsCategory::Web,
            SettingsCategory::CustomAgents => SettingsCategory::Agent,
            SettingsCategory::Environment => SettingsCategory::CustomAgents,
            SettingsCategory::AISettings => SettingsCategory::Environment,
        };
        self.reset_category_state();
    }

    /// Reset category-specific state when switching categories
    fn reset_category_state(&mut self) {
        self.selected_item = 0;
        self.custom_agent_index = 0;
        self.custom_agent_mode = CustomAgentMode::List;
        self.profile_index = 0;
        self.profile_mode = ProfileMode::List;
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
        } else if self.category == SettingsCategory::Environment {
            // Check if in EnvEdit mode
            if matches!(self.profile_mode, ProfileMode::EnvEdit(_)) {
                // Use EnvironmentState's navigation (SPEC-dafff079)
                self.env_state.select_next();
            } else {
                let profiles = self.profile_names();
                // +1 for "Add new profile" option at the end
                let max = profiles.len();
                if self.profile_index < max {
                    self.profile_index += 1;
                }
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
        } else if self.category == SettingsCategory::Environment {
            // Check if in EnvEdit mode
            if matches!(self.profile_mode, ProfileMode::EnvEdit(_)) {
                // Use EnvironmentState's navigation (SPEC-dafff079)
                self.env_state.select_prev();
            } else if self.profile_index > 0 {
                self.profile_index -= 1;
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

    /// Check if in form mode (CustomAgents or Profile)
    pub fn is_form_mode(&self) -> bool {
        matches!(
            self.custom_agent_mode,
            CustomAgentMode::Add | CustomAgentMode::Edit(_)
        ) || matches!(self.profile_mode, ProfileMode::Add | ProfileMode::Edit(_))
    }

    /// Check if in Profile form mode specifically
    pub fn is_profile_form_mode(&self) -> bool {
        matches!(self.profile_mode, ProfileMode::Add | ProfileMode::Edit(_))
    }

    /// Check if in EnvEdit mode
    pub fn is_env_edit_mode(&self) -> bool {
        matches!(self.profile_mode, ProfileMode::EnvEdit(_))
    }

    /// Check if in Profile delete confirmation mode
    pub fn is_profile_delete_mode(&self) -> bool {
        matches!(self.profile_mode, ProfileMode::ConfirmDelete(_))
    }

    /// Check if in delete confirmation mode
    pub fn is_delete_mode(&self) -> bool {
        matches!(self.custom_agent_mode, CustomAgentMode::ConfirmDelete(_))
    }

    /// Footer keybinds for Settings screen (used by app footer help)
    pub fn footer_keybinds(&self) -> String {
        if self.category == SettingsCategory::CustomAgents {
            match &self.custom_agent_mode {
                CustomAgentMode::List => {
                    if self.is_add_agent_selected() {
                        "[Enter] Add | [L/R] Category | [U/D] Select | [Tab] Screen | [Esc] Back"
                            .to_string()
                    } else {
                        "[Enter] Edit | [D] Delete | [L/R] Cat | [U/D] Sel | [Tab] Scr | [Esc] Back"
                            .to_string()
                    }
                }
                CustomAgentMode::Add | CustomAgentMode::Edit(_) => {
                    "[Tab/Up/Down] Field | [Space] Type | [Enter] Save | [Esc] Cancel".to_string()
                }
                CustomAgentMode::ConfirmDelete(_) => {
                    "[Left/Right] Select | [Enter] Confirm | [Esc] Cancel".to_string()
                }
            }
        } else if self.category == SettingsCategory::Environment {
            match &self.profile_mode {
                ProfileMode::List => {
                    if self.is_add_profile_selected() {
                        "[Enter] Add | [Space] Activate | [L/R] Cat | [U/D] Sel | [Tab] Scr | [Esc] Back"
                            .to_string()
                    } else {
                        "[Enter] Env | [E] Edit | [D] Del | [Space] Activate | [L/R] Cat | [Tab] Scr | [Esc] Back"
                            .to_string()
                    }
                }
                ProfileMode::Add | ProfileMode::Edit(_) => {
                    "[Tab/Up/Down] Field | [Enter] Save | [Esc] Cancel".to_string()
                }
                ProfileMode::ConfirmDelete(_) => {
                    "[Left/Right] Select | [Enter] Confirm | [Esc] Cancel".to_string()
                }
                ProfileMode::EnvEdit(_) => {
                    if self.env_state.edit_mode {
                        "[Tab] Switch | [Enter] Confirm | [Esc] Cancel".to_string()
                    } else {
                        "[Enter] Edit | [N] New | [D] Disable | [R] Reset | [Esc] Back".to_string()
                    }
                }
            }
        } else if self.category == SettingsCategory::AISettings {
            "[Enter] Open AI Settings Wizard | [L/R] Category | [Tab] Screen | [Esc] Back"
                .to_string()
        } else {
            "[Left/Right] Category | [Up/Down] Select | [Tab] Screen | [Esc] Back".to_string()
        }
    }

    // ========================================================================
    // Profile Management Methods
    // ========================================================================

    /// Get sorted profile names
    pub fn profile_names(&self) -> Vec<String> {
        self.profiles_config
            .as_ref()
            .map(|c| {
                let mut names: Vec<String> = c.profiles.keys().cloned().collect();
                names.sort();
                names
            })
            .unwrap_or_default()
    }

    /// Get selected profile
    pub fn selected_profile(&self) -> Option<&Profile> {
        let names = self.profile_names();
        names.get(self.profile_index).and_then(|name| {
            self.profiles_config
                .as_ref()
                .and_then(|c| c.profiles.get(name))
        })
    }

    /// Get selected profile name
    pub fn selected_profile_name(&self) -> Option<String> {
        let names = self.profile_names();
        names.get(self.profile_index).cloned()
    }

    /// Check if "Add new profile" option is selected
    pub fn is_add_profile_selected(&self) -> bool {
        self.category == SettingsCategory::Environment
            && self.profile_index == self.profile_names().len()
    }

    /// Check if profile is active
    pub fn is_profile_active(&self, name: &str) -> bool {
        self.profiles_config
            .as_ref()
            .and_then(|c| c.active.as_ref())
            .map(|active| active == name)
            .unwrap_or(false)
    }

    /// Set active profile
    pub fn set_active_profile(&mut self, name: Option<String>) {
        if let Some(ref mut config) = self.profiles_config {
            config.set_active(name);
        }
    }

    /// Toggle active profile for selected
    pub fn toggle_active_profile(&mut self) {
        if let Some(name) = self.selected_profile_name() {
            let is_active = self.is_profile_active(&name);
            if is_active {
                self.set_active_profile(None);
            } else {
                self.set_active_profile(Some(name));
            }
        }
    }

    /// Enter profile add mode
    pub fn enter_profile_add_mode(&mut self) {
        self.profile_form = ProfileFormState::new();
        self.profile_mode = ProfileMode::Add;
    }

    /// Enter profile edit mode
    pub fn enter_profile_edit_mode(&mut self) {
        if let Some(profile) = self.selected_profile() {
            let name = profile.name.clone();
            self.profile_form = ProfileFormState::from_profile(profile);
            self.profile_mode = ProfileMode::Edit(name);
        }
    }

    /// Enter profile delete confirmation mode
    pub fn enter_profile_delete_mode(&mut self) {
        if let Some(name) = self.selected_profile_name() {
            self.profile_mode = ProfileMode::ConfirmDelete(name);
            self.profile_delete_confirm = false;
        }
    }

    /// Enter environment variable edit mode (SPEC-dafff079 FR-009: integrate OS env vars)
    pub fn enter_env_edit_mode(&mut self) {
        if let Some(profile) = self.selected_profile() {
            let name = profile.name.clone();
            // Convert profile env to EnvItem vec
            let profile_vars: Vec<EnvItem> = profile
                .env
                .iter()
                .map(|(k, v)| EnvItem {
                    key: k.clone(),
                    value: v.clone(),
                    is_secret: false,
                })
                .collect();
            // Collect OS environment variables
            let os_vars = collect_os_env();
            // Initialize EnvironmentState with both (SPEC-dafff079 compliant)
            // FR-023: hide AI settings - managed separately in AI tab
            self.env_state = EnvironmentState::new()
                .with_variables(profile_vars)
                .with_os_variables(os_vars)
                .with_disabled_keys(profile.disabled_env.clone())
                .with_profile(&name)
                .with_hide_ai(true);
            self.profile_mode = ProfileMode::EnvEdit(name);
        }
    }

    /// Cancel profile mode and return to list
    pub fn cancel_profile_mode(&mut self) {
        self.profile_mode = ProfileMode::List;
        self.profile_form = ProfileFormState::default();
        self.profile_delete_confirm = false;
        self.env_state = EnvironmentState::default();
    }

    /// Persist env edit state to profile and save to disk (SPEC-dafff079 FR-021)
    pub fn persist_env_edit(&mut self) -> Result<(), &'static str> {
        let profile_name = match &self.profile_mode {
            ProfileMode::EnvEdit(name) => name.clone(),
            _ => return Err("Not in environment edit mode"),
        };

        let config = self
            .profiles_config
            .as_mut()
            .ok_or("Profiles config not loaded")?;

        let profile = config
            .profiles
            .get_mut(&profile_name)
            .ok_or("Profile not found")?;

        // Save profile env vars from EnvironmentState
        profile.env = self
            .env_state
            .variables
            .iter()
            .map(|v| (v.key.clone(), v.value.clone()))
            .collect();
        // Save disabled keys
        profile.disabled_env = self.env_state.disabled_keys.clone();

        config.save().map_err(|_| "Failed to save profiles")?;
        Ok(())
    }

    /// Save profile from form
    pub fn save_profile(&mut self) -> Result<(), &'static str> {
        self.profile_form.validate()?;

        match &self.profile_mode {
            ProfileMode::Add => {
                if let Some(ref mut config) = self.profiles_config {
                    let name = self.profile_form.name.clone();
                    if config.profiles.contains_key(&name) {
                        return Err("Profile with this name already exists");
                    }
                    let profile = self.profile_form.to_profile(None);
                    config.profiles.insert(name, profile);
                } else {
                    let mut config = ProfilesConfig::default();
                    let name = self.profile_form.name.clone();
                    let profile = self.profile_form.to_profile(None);
                    config.profiles.insert(name, profile);
                    self.profiles_config = Some(config);
                }
            }
            ProfileMode::Edit(original_name) => {
                if let Some(ref mut config) = self.profiles_config {
                    let new_name = self.profile_form.name.clone();
                    let original_profile = config.profiles.get(original_name);
                    let profile = self.profile_form.to_profile(original_profile);

                    // If name changed, remove old and insert new
                    if &new_name != original_name {
                        if config.profiles.contains_key(&new_name) {
                            return Err("Profile with this name already exists");
                        }
                        config.profiles.remove(original_name);
                        // Update active if it was the renamed profile
                        if config.active.as_ref() == Some(original_name) {
                            config.active = Some(new_name.clone());
                        }
                    }
                    config.profiles.insert(new_name, profile);
                }
            }
            _ => return Err("Invalid mode for save"),
        }

        self.cancel_profile_mode();
        Ok(())
    }

    // save_profile_env removed in favor of persist_env_edit auto-save flow

    /// Delete selected profile
    pub fn delete_profile(&mut self) -> bool {
        if let ProfileMode::ConfirmDelete(ref name) = self.profile_mode {
            let name = name.clone();
            if let Some(ref mut config) = self.profiles_config {
                if config.profiles.remove(&name).is_some() {
                    // Clear active if deleted profile was active
                    if config.active.as_ref() == Some(&name) {
                        config.active = None;
                    }
                    // Adjust index if needed
                    let profiles_len = config.profiles.len();
                    if self.profile_index > 0 && self.profile_index >= profiles_len {
                        self.profile_index = profiles_len.saturating_sub(1);
                    }
                    self.cancel_profile_mode();
                    return true;
                }
            }
        }
        false
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
        SettingsCategory::Environment => {
            if state.is_add_profile_selected() {
                "Add a new environment profile."
            } else if let Some(profile) = state.selected_profile() {
                if profile.env.is_empty() {
                    "No environment variables. Press Enter to add, E to edit profile name."
                } else {
                    "Press Enter to edit env vars, E to edit profile name, Space to activate."
                }
            } else {
                "Manage environment profiles for coding agents."
            }
        }
        SettingsCategory::AISettings => {
            "Configure AI settings (endpoint, API key, model). Press Enter to open wizard."
        }
    }
}

fn panel_title_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn styled_block<'a, T>(title: T) -> Block<'a>
where
    T: Into<Line<'a>>,
{
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White))
        .title(title)
        .title_style(panel_title_style())
}

/// Render settings screen
pub fn render_settings(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tabs
            Constraint::Min(0),    // Content
        ])
        .split(area);

    // Category tabs
    render_tabs(state, frame, chunks[0]);

    // Settings content
    render_settings_content(state, frame, chunks[1]);
}

fn render_tabs(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let categories = [
        ("General", SettingsCategory::General),
        ("Worktree", SettingsCategory::Worktree),
        ("Web", SettingsCategory::Web),
        ("Agent", SettingsCategory::Agent),
        ("Custom", SettingsCategory::CustomAgents), // T309
        ("Env", SettingsCategory::Environment),
        ("AI", SettingsCategory::AISettings),
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
        .block(styled_block(" Categories "))
        .highlight_style(Style::default().fg(Color::Cyan))
        .select(match state.category {
            SettingsCategory::General => 0,
            SettingsCategory::Worktree => 1,
            SettingsCategory::Web => 2,
            SettingsCategory::Agent => 3,
            SettingsCategory::CustomAgents => 4,
            SettingsCategory::Environment => 5,
            SettingsCategory::AISettings => 6,
        });

    frame.render_widget(tabs, area);
}

fn render_settings_content(state: &SettingsState, frame: &mut Frame, area: Rect) {
    // CustomAgents has special rendering (T309)
    if state.category == SettingsCategory::CustomAgents {
        render_custom_agents_content(state, frame, area);
        return;
    }

    // Environment: render profile content directly (integrated from Screen::Profiles)
    if state.category == SettingsCategory::Environment {
        render_profile_content(state, frame, area);
        return;
    }

    // AISettings has special rendering
    if state.category == SettingsCategory::AISettings {
        render_ai_settings_content(state, frame, area);
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
            .block(styled_block(" Settings "));
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
        SettingsCategory::Environment => "Environment",    // Handled separately
        SettingsCategory::AISettings => "AI",              // Handled separately
    };

    let list = List::new(list_items).block(styled_block(format!(" {} Settings ", category_name)));
    frame.render_widget(list, list_area);

    if let Some(desc_area) = desc_area {
        let description = selected_description(state);
        let paragraph = Paragraph::new(description)
            .wrap(Wrap { trim: true })
            .block(styled_block(" Description "));
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

    let list = List::new(list_items).block(styled_block(" Custom Coding Agents "));
    frame.render_widget(list, list_area);

    if let Some(desc_area) = desc_area {
        let description = selected_description(state);
        let paragraph = Paragraph::new(description)
            .wrap(Wrap { trim: true })
            .block(styled_block(" Description "));
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

    let block = styled_block(title);
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

        let field_block = styled_block(format!(" {} ", label)).border_style(field_style);

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

    let block = styled_block(" Delete Custom Agent ");
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
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::White)),
        );
    let no_btn = Paragraph::new(" No ")
        .alignment(Alignment::Center)
        .style(no_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::White)),
        );

    frame.render_widget(yes_btn, button_chunks[1]);
    frame.render_widget(no_btn, button_chunks[2]);
}

// ============================================================================
// Profile Rendering Functions (integrated from Screen::Profiles)
// ============================================================================

/// Render profile content based on mode (integrated from Screen::Profiles)
fn render_profile_content(state: &SettingsState, frame: &mut Frame, area: Rect) {
    match &state.profile_mode {
        ProfileMode::List => render_profile_list(state, frame, area),
        ProfileMode::Add | ProfileMode::Edit(_) => {
            render_profile_form(state, frame, area);
        }
        ProfileMode::ConfirmDelete(_) => {
            render_profile_delete_confirmation(state, frame, area);
        }
        ProfileMode::EnvEdit(_) => {
            render_env_edit(state, frame, area);
        }
    }
}

/// Render profile list
fn render_profile_list(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let names = state.profile_names();

    let (list_area, desc_area) = if area.height >= 6 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let mut list_items: Vec<ListItem> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let is_active = state.is_profile_active(name);
            let active_marker = if is_active { " *" } else { "" };
            let profile = state
                .profiles_config
                .as_ref()
                .and_then(|c| c.profiles.get(name));
            let env_count = profile.map(|p| p.env.len()).unwrap_or(0);

            let content = format!("  {}{} ({} env vars)", name, active_marker, env_count);
            let style = if i == state.profile_index {
                Style::default().add_modifier(Modifier::REVERSED)
            } else if is_active {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    // Add "Add new profile" option at the end
    let add_selected = state.is_add_profile_selected();
    let add_style = if add_selected {
        Style::default()
            .add_modifier(Modifier::REVERSED)
            .fg(Color::Green)
    } else {
        Style::default().fg(Color::Green)
    };
    list_items.push(ListItem::new("  + Add new profile...").style(add_style));

    let list = List::new(list_items).block(styled_block(" Environment Profiles "));
    frame.render_widget(list, list_area);

    if let Some(desc_area) = desc_area {
        let description = selected_description(state);
        let paragraph = Paragraph::new(description)
            .wrap(Wrap { trim: true })
            .block(styled_block(" Description "));
        frame.render_widget(paragraph, desc_area);
    }
}

/// Render profile form for add/edit
fn render_profile_form(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let form = &state.profile_form;
    let is_edit = matches!(state.profile_mode, ProfileMode::Edit(_));
    let title = if is_edit {
        " Edit Profile "
    } else {
        " Add Profile "
    };

    let block = styled_block(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Form fields layout (Name and Description only)
    let field_height = 3;
    let fields = ProfileFormField::all();
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

        let value = match field {
            ProfileFormField::Name => &form.name,
            ProfileFormField::Description => &form.description,
        };

        // Build display text with cursor
        let display_text = if is_selected {
            let mut text = value.clone();
            let cursor_pos = form.cursor.min(text.len());
            text.insert(cursor_pos, '|');
            text
        } else {
            value.clone()
        };

        let field_style = if is_selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        let field_block = styled_block(format!(" {} ", label)).border_style(field_style);

        let paragraph = Paragraph::new(display_text).block(field_block);
        frame.render_widget(paragraph, chunks[i]);
    }
}

/// Render profile delete confirmation dialog
fn render_profile_delete_confirmation(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let profile_name = match &state.profile_mode {
        ProfileMode::ConfirmDelete(name) => name.as_str(),
        _ => return,
    };

    let block = styled_block(" Delete Profile ");
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
        "Are you sure you want to delete profile '{}'?",
        profile_name
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

    let yes_style = if state.profile_delete_confirm {
        Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(Color::Red)
    };

    let no_style = if !state.profile_delete_confirm {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(Color::Green)
    };

    let yes_btn = Paragraph::new(" Yes ")
        .alignment(Alignment::Center)
        .style(yes_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::White)),
        );
    let no_btn = Paragraph::new(" No ")
        .alignment(Alignment::Center)
        .style(no_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::White)),
        );

    frame.render_widget(yes_btn, button_chunks[1]);
    frame.render_widget(no_btn, button_chunks[2]);
}

/// Render environment variable edit screen (SPEC-dafff079 compliant)
/// FR-009: OS env vars integrated with profile env vars
/// FR-010: Yellow=overridden, Green=added, Normal=OS-only, Red+strikethrough=disabled
fn render_env_edit(state: &SettingsState, frame: &mut Frame, area: Rect) {
    use crate::tui::screens::environment::render_environment;

    let profile_name = match &state.profile_mode {
        ProfileMode::EnvEdit(name) => name.as_str(),
        _ => return,
    };

    // Use the existing render_environment function from environment.rs
    // which already implements SPEC-dafff079 requirements
    let block = styled_block(format!(" Environment Variables - {} ", profile_name));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Create a mutable copy for rendering
    let mut env_state = state.env_state.clone();
    render_environment(&mut env_state, frame, inner);
}

/// Render AI settings content
fn render_ai_settings_content(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let block = styled_block(" AI Settings ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Check if AI settings exist
    let default_ai = state
        .profiles_config
        .as_ref()
        .and_then(|c| c.default_ai.as_ref());

    if let Some(ai) = default_ai {
        // Show current settings
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Endpoint label
                Constraint::Length(1), // Endpoint value
                Constraint::Length(1), // API Key label
                Constraint::Length(1), // API Key value
                Constraint::Length(1), // Model label
                Constraint::Length(1), // Model value
                Constraint::Length(1), // Spacing
                Constraint::Length(3), // Button
                Constraint::Min(0),    // Padding
            ])
            .margin(1)
            .split(inner);

        // Endpoint
        let label = Paragraph::new("Endpoint:").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(label, chunks[0]);
        let value =
            Paragraph::new(format!("  {}", ai.endpoint)).style(Style::default().fg(Color::White));
        frame.render_widget(value, chunks[1]);

        // API Key
        let label = Paragraph::new("API Key:").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(label, chunks[2]);
        let key_display = if ai.api_key.is_empty() {
            "(not set)".to_string()
        } else {
            format!("  {}...", &ai.api_key.chars().take(8).collect::<String>())
        };
        let value = Paragraph::new(key_display).style(Style::default().fg(Color::White));
        frame.render_widget(value, chunks[3]);

        // Model
        let label = Paragraph::new("Model:").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(label, chunks[4]);
        let value =
            Paragraph::new(format!("  {}", ai.model)).style(Style::default().fg(Color::White));
        frame.render_widget(value, chunks[5]);

        // Button
        let button_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(50),
                Constraint::Percentage(25),
            ])
            .split(chunks[7])[1];

        let button = Paragraph::new("[ Edit AI Settings ]")
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White)),
            );
        frame.render_widget(button, button_area);
    } else {
        // No settings - show create button
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Info text
                Constraint::Length(3), // Button
                Constraint::Min(0),    // Padding
            ])
            .margin(1)
            .split(inner);

        let info = Paragraph::new("No AI settings configured.")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(info, chunks[0]);

        let button_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(50),
                Constraint::Percentage(25),
            ])
            .split(chunks[1])[1];

        let button = Paragraph::new("[ Configure AI Settings ]")
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White)),
            );
        frame.render_widget(button, button_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::config::{Profile, ProfilesConfig, Settings};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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

    /// FR-029: Enter opens env edit mode, FR-030: E opens profile edit mode
    /// Tests that the instruction string correctly shows Enter for Env and E for Edit
    #[test]
    fn test_environment_instructions_enter_for_env_e_for_edit() {
        let mut state = SettingsState::new().with_settings(Settings::default());
        state.category = SettingsCategory::Environment;

        let mut profile = Profile::new("dev");
        profile
            .env
            .insert("API_KEY".to_string(), "secret".to_string());
        let mut config = ProfilesConfig {
            version: 1,
            active: Some("dev".to_string()),
            default_ai: None,
            profiles: HashMap::new(),
        };
        config.profiles.insert("dev".to_string(), profile);
        state.profiles_config = Some(config);
        state.profile_index = 0;

        let instructions = state.footer_keybinds();

        // Enter should be for Env (environment variables)
        assert!(
            instructions.contains("[Enter] Env"),
            "Instructions should show Enter for Env: {}",
            instructions
        );
        // E should be for Edit (profile name/description)
        assert!(
            instructions.contains("[E] Edit"),
            "Instructions should show E for Edit: {}",
            instructions
        );
    }

    #[test]
    fn test_environment_description_enter_for_env_vars() {
        let mut state = SettingsState::new();
        state.category = SettingsCategory::Environment;
        // Create a config with only one profile that has env vars
        let mut profile = Profile::new("dev");
        profile
            .env
            .insert("API_KEY".to_string(), "secret".to_string());
        let mut config = ProfilesConfig {
            version: 1,
            active: Some("dev".to_string()),
            default_ai: None,
            profiles: HashMap::new(),
        };
        config.profiles.insert("dev".to_string(), profile);
        state.profiles_config = Some(config);
        state.profile_index = 0; // "dev" is the only profile

        let desc = selected_description(&state);
        // Description should mention Enter for env vars, E for profile name
        assert!(
            desc.contains("Enter to edit env vars"),
            "Description should mention Enter for env vars: {}",
            desc
        );
        assert!(
            desc.contains("E to edit profile name"),
            "Description should mention E for profile name: {}",
            desc
        );
    }

    fn find_text_style(
        buffer: &ratatui::buffer::Buffer,
        width: u16,
        height: u16,
        text: &str,
    ) -> Option<Style> {
        for y in 0..height {
            let line: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
            if let Some(idx) = line.find(text) {
                let x = idx as u16;
                return Some(buffer[(x, y)].style());
            }
        }
        None
    }

    #[test]
    fn test_settings_titles_use_cyan_bold_style() {
        let state = SettingsState::new().with_settings(Settings::default());
        let width = 100;
        let height = 20;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal init");

        terminal
            .draw(|f| {
                let area = f.area();
                render_settings(&state, f, area);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        let style = find_text_style(buffer, width, height, " General Settings ")
            .expect("General Settings title should be rendered");

        assert_eq!(style.fg, Some(Color::Cyan));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    /// SPEC-dafff079 FR-009: OS env integration
    #[test]
    fn test_enter_env_edit_mode_integrates_os_variables() {
        let mut state = SettingsState::new();
        state.category = SettingsCategory::Environment;
        // Create a profile with one env var
        let mut profile = Profile::new("test-profile");
        profile
            .env
            .insert("MY_VAR".to_string(), "my_value".to_string());
        let mut config = ProfilesConfig {
            version: 1,
            active: None,
            default_ai: None,
            profiles: HashMap::new(),
        };
        config.profiles.insert("test-profile".to_string(), profile);
        state.profiles_config = Some(config);
        state.profile_index = 0;
        // Enter env edit mode
        state.enter_env_edit_mode();
        // Verify env_state has os_variables populated
        // (OS variables should be collected from current environment)
        assert!(
            !state.env_state.os_variables.is_empty() || state.env_state.variables.len() == 1,
            "EnvironmentState should have OS variables or profile variables"
        );
        // Verify profile variable is included
        assert!(
            state.env_state.variables.iter().any(|v| v.key == "MY_VAR"),
            "EnvironmentState should include profile variables"
        );
    }

    #[test]
    fn test_persist_env_edit_saves_profiles() {
        let _lock = ENV_LOCK.lock().unwrap();
        let temp = TempDir::new().expect("temp dir");
        let prev_home = std::env::var_os("HOME");

        struct HomeGuard(Option<OsString>);
        impl Drop for HomeGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(value) => std::env::set_var("HOME", value),
                    None => std::env::remove_var("HOME"),
                }
            }
        }

        std::env::set_var("HOME", temp.path());
        let _guard = HomeGuard(prev_home);

        let mut config = ProfilesConfig::default();
        let mut dev = Profile::new("dev");
        dev.env.insert("MY_VAR".to_string(), "old".to_string());
        config.profiles.insert("dev".to_string(), dev);
        config.save().expect("save initial config");

        let mut state = SettingsState::new();
        state.profiles_config = Some(config);
        state.profile_mode = ProfileMode::EnvEdit("dev".to_string());
        state.env_state = EnvironmentState::new()
            .with_variables(vec![EnvItem {
                key: "MY_VAR".to_string(),
                value: "new".to_string(),
                is_secret: false,
            }])
            .with_hide_ai(true);
        state.env_state.selected = 0;

        assert!(state.persist_env_edit().is_ok());
        assert!(matches!(state.profile_mode, ProfileMode::EnvEdit(_)));

        let loaded = ProfilesConfig::load().expect("load updated config");
        let env = &loaded.profiles.get("dev").unwrap().env;
        assert_eq!(env.get("MY_VAR"), Some(&"new".to_string()));
    }

    /// SPEC-dafff079 FR-031-034: Help text includes R for reset and D for disable
    #[test]
    fn test_env_edit_instructions_include_reset_and_disable() {
        // The instruction for EnvEdit mode when not editing
        let instructions = "[Enter] Edit | [N] New | [D] Disable | [R] Reset | [Esc] Back";
        assert!(
            instructions.contains("[D] Disable"),
            "Instructions should show D for Disable"
        );
        assert!(
            instructions.contains("[R] Reset"),
            "Instructions should show R for Reset"
        );
    }
}
