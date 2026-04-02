//! Settings screen — category-based settings management
//!
//! Migrated from gwt-cli reference with Elm Architecture adaptation.
//! Includes custom agent management (SPEC-71f2742d US3),
//! profile management, environment variable editing.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gwt_core::config::{
    AgentType, CustomCodingAgent, Profile, ProfilesConfig, Settings, ToolsConfig,
};
use ratatui::prelude::*;
use ratatui::widgets::*;

// ---------------------------------------------------------------------------
// Settings categories
// ---------------------------------------------------------------------------

/// Settings categories displayed in the left/tab panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsCategory {
    General,
    Worktree,
    Agent,
    CustomAgents,
    Environment,
    AISettings,
}

impl SettingsCategory {
    pub const ALL: [SettingsCategory; 6] = [
        SettingsCategory::General,
        SettingsCategory::Worktree,
        SettingsCategory::Agent,
        SettingsCategory::CustomAgents,
        SettingsCategory::Environment,
        SettingsCategory::AISettings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsCategory::General => "General",
            SettingsCategory::Worktree => "Worktree",
            SettingsCategory::Agent => "Agent",
            SettingsCategory::CustomAgents => "Custom",
            SettingsCategory::Environment => "Env",
            SettingsCategory::AISettings => "AI",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Custom agent form types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CustomAgentMode {
    #[default]
    List,
    Add,
    Edit(String),
    ConfirmDelete(String),
}

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
    pub fn new() -> Self {
        Self {
            agent_type: AgentType::Command,
            ..Default::default()
        }
    }

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

    fn current_value(&self) -> &str {
        match self.current_field {
            AgentFormField::Id => &self.id,
            AgentFormField::DisplayName => &self.display_name,
            AgentFormField::Type => "",
            AgentFormField::Command => &self.command,
        }
    }

    fn current_value_mut(&mut self) -> Option<&mut String> {
        match self.current_field {
            AgentFormField::Id => Some(&mut self.id),
            AgentFormField::DisplayName => Some(&mut self.display_name),
            AgentFormField::Type => None,
            AgentFormField::Command => Some(&mut self.command),
        }
    }

    pub fn next_field(&mut self) {
        let fields = AgentFormField::all();
        let idx = fields
            .iter()
            .position(|f| *f == self.current_field)
            .unwrap_or(0);
        self.current_field = fields[(idx + 1) % fields.len()];
        self.cursor = self.current_value().len();
    }

    pub fn prev_field(&mut self) {
        let fields = AgentFormField::all();
        let idx = fields
            .iter()
            .position(|f| *f == self.current_field)
            .unwrap_or(0);
        self.current_field = fields[(idx + fields.len() - 1) % fields.len()];
        self.cursor = self.current_value().len();
    }

    pub fn insert_char(&mut self, c: char) {
        let cursor = self.cursor;
        if let Some(value) = self.current_value_mut() {
            value.insert(cursor, c);
        }
        self.cursor += 1;
    }

    pub fn delete_char(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let cursor = self.cursor;
            if let Some(value) = self.current_value_mut() {
                value.remove(cursor);
            }
        }
    }

    pub fn cycle_type(&mut self) {
        self.agent_type = match self.agent_type {
            AgentType::Command => AgentType::Path,
            AgentType::Path => AgentType::Bunx,
            AgentType::Bunx => AgentType::Command,
        };
    }

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

// ---------------------------------------------------------------------------
// Profile management types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProfileMode {
    #[default]
    List,
    Add,
    Edit(String),
    ConfirmDelete(String),
    EnvEdit(String),
}

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

#[derive(Debug, Clone, Default)]
pub struct ProfileFormState {
    pub name: String,
    pub description: String,
    pub current_field: ProfileFormField,
    pub cursor: usize,
}

impl ProfileFormState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_profile(profile: &Profile) -> Self {
        Self {
            name: profile.name.clone(),
            description: profile.description.clone(),
            current_field: ProfileFormField::Name,
            cursor: profile.name.len(),
        }
    }

    pub fn to_profile(&self, original: Option<&Profile>) -> Profile {
        Profile {
            name: self.name.clone(),
            description: self.description.clone(),
            env: original.map(|p| p.env.clone()).unwrap_or_default(),
            disabled_env: original.map(|p| p.disabled_env.clone()).unwrap_or_default(),
            ai: original.and_then(|p| p.ai.clone()),
            ai_enabled: original.and_then(|p| p.ai_enabled),
        }
    }

    fn current_value(&self) -> &str {
        match self.current_field {
            ProfileFormField::Name => &self.name,
            ProfileFormField::Description => &self.description,
        }
    }

    fn current_value_mut(&mut self) -> &mut String {
        match self.current_field {
            ProfileFormField::Name => &mut self.name,
            ProfileFormField::Description => &mut self.description,
        }
    }

    pub fn next_field(&mut self) {
        let fields = ProfileFormField::all();
        let idx = fields
            .iter()
            .position(|f| *f == self.current_field)
            .unwrap_or(0);
        self.current_field = fields[(idx + 1) % fields.len()];
        self.cursor = self.current_value().len();
    }

    pub fn prev_field(&mut self) {
        let fields = ProfileFormField::all();
        let idx = fields
            .iter()
            .position(|f| *f == self.current_field)
            .unwrap_or(0);
        self.current_field = fields[(idx + fields.len() - 1) % fields.len()];
        self.cursor = self.current_value().len();
    }

    pub fn insert_char(&mut self, c: char) {
        let cursor = self.cursor;
        let value = self.current_value_mut();
        value.insert(cursor, c);
        self.cursor += 1;
    }

    pub fn delete_char(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let cursor = self.cursor;
            let value = self.current_value_mut();
            value.remove(cursor);
        }
    }

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

// ---------------------------------------------------------------------------
// Environment variable edit state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvDisplayKind {
    OsOnly,
    OsDisabled,
    Added,
    Overridden,
}

#[derive(Debug, Clone)]
struct DisplayEnvItem {
    key: String,
    value: String,
    kind: EnvDisplayKind,
    profile_index: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct EnvEditState {
    pub vars: Vec<(String, String)>,
    pub os_vars: Vec<(String, String)>,
    pub disabled_keys: Vec<String>,
    pub selected_index: usize,
    pub editing: Option<EnvEditMode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvEditMode {
    Key(usize),
    Value(usize),
}

impl EnvEditState {
    pub fn from_profile(profile: &Profile) -> Self {
        let mut vars: Vec<(String, String)> = profile
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        vars.sort_by(|a, b| a.0.cmp(&b.0));
        let mut os_vars: Vec<(String, String)> = std::env::vars().collect();
        os_vars.sort_by(|a, b| a.0.cmp(&b.0));
        Self {
            vars,
            os_vars,
            disabled_keys: profile.disabled_env.clone(),
            selected_index: 0,
            editing: None,
        }
    }

    pub fn to_env(&self) -> HashMap<String, String> {
        self.vars.iter().cloned().collect()
    }

    pub fn add_new_var(&mut self) {
        self.vars.push((String::new(), String::new()));
        self.selected_index = 0;
        self.editing = Some(EnvEditMode::Key(0));
    }

    pub fn delete_selected(&mut self) {
        if self.selected_is_overridden() {
            self.delete_selected_override();
            return;
        }
        if let Some(index) = self.selected_profile_index() {
            self.vars.remove(index);
            if self.selected_index > 0 && self.selected_index >= self.display_len() {
                self.selected_index = self.display_len().saturating_sub(1);
            }
        }
    }

    pub fn select_next(&mut self) {
        let total = self.display_len();
        if total > 0 && self.selected_index < total - 1 {
            self.selected_index += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

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

    fn display_len(&self) -> usize {
        self.display_items().len()
    }

    fn selected_display_item(&self) -> Option<DisplayEnvItem> {
        let items = self.display_items();
        items.get(self.selected_index).cloned()
    }

    fn display_items(&self) -> Vec<DisplayEnvItem> {
        let mut os_map: HashMap<String, String> = HashMap::new();
        for (key, value) in &self.os_vars {
            os_map.insert(key.clone(), value.clone());
        }
        let mut profile_map: HashMap<String, (usize, String)> = HashMap::new();
        for (index, (key, value)) in self.vars.iter().enumerate() {
            profile_map.insert(key.clone(), (index, value.clone()));
        }

        let mut keys: Vec<String> = os_map.keys().cloned().collect();
        for key in profile_map.keys() {
            if !os_map.contains_key(key) {
                keys.push(key.clone());
            }
        }
        keys.sort();

        keys.into_iter()
            .map(|key| match (profile_map.get(&key), os_map.get(&key)) {
                (Some((index, profile_value)), Some(_)) => DisplayEnvItem {
                    key,
                    value: profile_value.clone(),
                    kind: EnvDisplayKind::Overridden,
                    profile_index: Some(*index),
                },
                (Some((index, profile_value)), None) => DisplayEnvItem {
                    key,
                    value: profile_value.clone(),
                    kind: EnvDisplayKind::Added,
                    profile_index: Some(*index),
                },
                (None, Some(os_value)) => DisplayEnvItem {
                    key: key.clone(),
                    value: os_value.clone(),
                    kind: if self.disabled_keys.contains(&key) {
                        EnvDisplayKind::OsDisabled
                    } else {
                        EnvDisplayKind::OsOnly
                    },
                    profile_index: None,
                },
                (None, None) => DisplayEnvItem {
                    key,
                    value: String::new(),
                    kind: EnvDisplayKind::OsOnly,
                    profile_index: None,
                },
            })
            .collect()
    }

    pub fn selected_profile_index(&self) -> Option<usize> {
        self.selected_display_item()
            .and_then(|item| item.profile_index)
    }

    pub fn selected_is_overridden(&self) -> bool {
        matches!(
            self.selected_display_item().map(|item| item.kind),
            Some(EnvDisplayKind::Overridden)
        )
    }

    pub fn selected_is_added(&self) -> bool {
        matches!(
            self.selected_display_item().map(|item| item.kind),
            Some(EnvDisplayKind::Added)
        )
    }

    pub fn selected_is_os_entry(&self) -> bool {
        matches!(
            self.selected_display_item().map(|item| item.kind),
            Some(EnvDisplayKind::OsOnly | EnvDisplayKind::OsDisabled)
        )
    }

    pub fn selected_key(&self) -> Option<String> {
        self.selected_display_item().map(|item| item.key)
    }

    pub fn toggle_selected_disabled(&mut self) -> bool {
        let Some(item) = self.selected_display_item() else {
            return false;
        };
        if !matches!(
            item.kind,
            EnvDisplayKind::OsOnly | EnvDisplayKind::OsDisabled
        ) {
            return false;
        }
        if let Some(pos) = self.disabled_keys.iter().position(|key| key == &item.key) {
            self.disabled_keys.remove(pos);
            false
        } else {
            self.disabled_keys.push(item.key);
            self.disabled_keys.sort();
            true
        }
    }

    pub fn delete_selected_override(&mut self) {
        if let Some(item) = self.selected_display_item() {
            if item.kind == EnvDisplayKind::Overridden {
                if let Some(pos) = self.vars.iter().position(|var| var.0 == item.key) {
                    self.vars.remove(pos);
                }
            }
        }
    }

    pub fn start_edit_selected(&mut self) {
        let Some(item) = self.selected_display_item() else {
            return;
        };
        if let Some(index) = item.profile_index {
            let value_len = self.vars[index].1.len();
            self.selected_index = self
                .selected_index
                .min(self.display_len().saturating_sub(1));
            self.editing = Some(EnvEditMode::Value(value_len));
            return;
        }

        self.vars.push((item.key.clone(), item.value.clone()));
        self.vars.sort_by(|a, b| a.0.cmp(&b.0));
        let new_index = self
            .display_items()
            .iter()
            .position(|display| display.key == item.key && display.profile_index.is_some())
            .unwrap_or(0);
        self.selected_index = new_index;
        self.editing = Some(EnvEditMode::Value(item.value.len()));
    }
}

// ---------------------------------------------------------------------------
// SettingsState
// ---------------------------------------------------------------------------

/// Central state for the Settings screen.
#[derive(Debug)]
pub struct SettingsState {
    pub category: SettingsCategory,
    pub selected_item: usize,
    pub settings: Option<Settings>,
    pub error_message: Option<String>,
    // Custom agents
    pub tools_config: Option<ToolsConfig>,
    pub custom_agent_mode: CustomAgentMode,
    pub custom_agent_index: usize,
    pub agent_form: AgentFormState,
    pub delete_confirm: bool,
    // Profile management
    pub profiles_config: Option<ProfilesConfig>,
    pub profile_mode: ProfileMode,
    pub profile_index: usize,
    pub profile_form: ProfileFormState,
    pub profile_delete_confirm: bool,
    // Environment variable edit
    pub env_state: EnvEditState,
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
            profiles_config: None,
            profile_mode: ProfileMode::default(),
            profile_index: 0,
            profile_form: ProfileFormState::default(),
            profile_delete_confirm: false,
            env_state: EnvEditState::default(),
        }
    }
}

impl SettingsState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load settings from global config.
    pub fn load_settings(&mut self) {
        match Settings::load_global() {
            Ok(s) => {
                self.tools_config = Some(s.tools.clone());
                self.profiles_config = Some(s.profiles.clone());
                self.settings = Some(s);
                self.error_message = None;
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to load settings: {e}"));
            }
        }
    }

    /// Get display items for the current category.
    pub fn category_items(&self) -> Vec<(&'static str, String)> {
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
            ],
            SettingsCategory::CustomAgents
            | SettingsCategory::Environment
            | SettingsCategory::AISettings => vec![],
        }
    }

    pub fn custom_agents(&self) -> &[CustomCodingAgent] {
        self.tools_config
            .as_ref()
            .map(|c| c.custom_coding_agents.as_slice())
            .unwrap_or(&[])
    }

    // ---- Category navigation ----

    pub fn next_category(&mut self) {
        let idx = self.category.index();
        self.category = SettingsCategory::ALL[(idx + 1) % SettingsCategory::ALL.len()];
        self.reset_category_state();
    }

    pub fn prev_category(&mut self) {
        let idx = self.category.index();
        let len = SettingsCategory::ALL.len();
        self.category = SettingsCategory::ALL[(idx + len - 1) % len];
        self.reset_category_state();
    }

    fn reset_category_state(&mut self) {
        self.selected_item = 0;
        self.custom_agent_index = 0;
        self.custom_agent_mode = CustomAgentMode::List;
        self.profile_index = 0;
        self.profile_mode = ProfileMode::List;
    }

    // ---- Item navigation ----

    pub fn select_next(&mut self) {
        match self.category {
            SettingsCategory::CustomAgents => {
                let max = self.custom_agents().len();
                if self.custom_agent_index < max {
                    self.custom_agent_index += 1;
                }
            }
            SettingsCategory::Environment => {
                if matches!(self.profile_mode, ProfileMode::EnvEdit(_)) {
                    self.env_state.select_next();
                } else {
                    let max = self.profile_names().len();
                    if self.profile_index < max {
                        self.profile_index += 1;
                    }
                }
            }
            _ => {
                let items = self.category_items();
                if !items.is_empty() && self.selected_item < items.len() - 1 {
                    self.selected_item += 1;
                }
            }
        }
    }

    pub fn select_prev(&mut self) {
        match self.category {
            SettingsCategory::CustomAgents => {
                if self.custom_agent_index > 0 {
                    self.custom_agent_index -= 1;
                }
            }
            SettingsCategory::Environment => {
                if matches!(self.profile_mode, ProfileMode::EnvEdit(_)) {
                    self.env_state.select_prev();
                } else if self.profile_index > 0 {
                    self.profile_index -= 1;
                }
            }
            _ => {
                if self.selected_item > 0 {
                    self.selected_item -= 1;
                }
            }
        }
    }

    // ---- Custom agent helpers ----

    pub fn selected_custom_agent(&self) -> Option<&CustomCodingAgent> {
        self.custom_agents().get(self.custom_agent_index)
    }

    pub fn is_add_agent_selected(&self) -> bool {
        self.category == SettingsCategory::CustomAgents
            && self.custom_agent_index == self.custom_agents().len()
    }

    pub fn enter_add_mode(&mut self) {
        self.agent_form = AgentFormState::new();
        self.custom_agent_mode = CustomAgentMode::Add;
    }

    pub fn enter_edit_mode(&mut self) {
        if let Some(agent) = self.selected_custom_agent() {
            let id = agent.id.clone();
            self.agent_form = AgentFormState::from_agent(agent);
            self.custom_agent_mode = CustomAgentMode::Edit(id);
        }
    }

    pub fn enter_delete_mode(&mut self) {
        if let Some(agent) = self.selected_custom_agent() {
            self.custom_agent_mode = CustomAgentMode::ConfirmDelete(agent.id.clone());
            self.delete_confirm = false;
        }
    }

    pub fn cancel_mode(&mut self) {
        self.custom_agent_mode = CustomAgentMode::List;
        self.agent_form = AgentFormState::default();
        self.delete_confirm = false;
    }

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

    pub fn delete_agent(&mut self) -> bool {
        if let CustomAgentMode::ConfirmDelete(ref id) = self.custom_agent_mode {
            let id = id.clone();
            if let Some(ref mut config) = self.tools_config {
                if config.remove_agent(&id) {
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

    pub fn is_form_mode(&self) -> bool {
        matches!(
            self.custom_agent_mode,
            CustomAgentMode::Add | CustomAgentMode::Edit(_)
        ) || matches!(self.profile_mode, ProfileMode::Add | ProfileMode::Edit(_))
    }

    pub fn is_delete_mode(&self) -> bool {
        matches!(self.custom_agent_mode, CustomAgentMode::ConfirmDelete(_))
    }

    // ---- Profile helpers ----

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

    pub fn selected_profile(&self) -> Option<&Profile> {
        let names = self.profile_names();
        names.get(self.profile_index).and_then(|name| {
            self.profiles_config
                .as_ref()
                .and_then(|c| c.profiles.get(name))
        })
    }

    pub fn selected_profile_name(&self) -> Option<String> {
        self.profile_names().get(self.profile_index).cloned()
    }

    pub fn is_add_profile_selected(&self) -> bool {
        self.category == SettingsCategory::Environment
            && self.profile_index == self.profile_names().len()
    }

    pub fn is_profile_active(&self, name: &str) -> bool {
        self.profiles_config
            .as_ref()
            .and_then(|c| c.active.as_ref())
            .map(|active| active == name)
            .unwrap_or(false)
    }

    pub fn toggle_active_profile(&mut self) {
        if let Some(name) = self.selected_profile_name() {
            let is_active = self.is_profile_active(&name);
            if let Some(ref mut config) = self.profiles_config {
                config.set_active(if is_active { None } else { Some(name) });
            }
        }
    }

    pub fn enter_profile_add_mode(&mut self) {
        self.profile_form = ProfileFormState::new();
        self.profile_mode = ProfileMode::Add;
    }

    pub fn enter_profile_edit_mode(&mut self) {
        if let Some(profile) = self.selected_profile() {
            let name = profile.name.clone();
            self.profile_form = ProfileFormState::from_profile(profile);
            self.profile_mode = ProfileMode::Edit(name);
        }
    }

    pub fn enter_profile_delete_mode(&mut self) {
        if let Some(name) = self.selected_profile_name() {
            self.profile_mode = ProfileMode::ConfirmDelete(name);
            self.profile_delete_confirm = false;
        }
    }

    pub fn enter_env_edit_mode(&mut self) {
        if let Some(profile) = self.selected_profile() {
            let name = profile.name.clone();
            self.env_state = EnvEditState::from_profile(profile);
            self.profile_mode = ProfileMode::EnvEdit(name);
        }
    }

    pub fn cancel_profile_mode(&mut self) {
        self.profile_mode = ProfileMode::List;
        self.profile_form = ProfileFormState::default();
        self.profile_delete_confirm = false;
        self.env_state = EnvEditState::default();
    }

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
                    let mut config = ProfilesConfig {
                        version: 1,
                        active: None,
                        profiles: HashMap::new(),
                    };
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

                    if &new_name != original_name {
                        if config.profiles.contains_key(&new_name) {
                            return Err("Profile with this name already exists");
                        }
                        config.profiles.remove(original_name);
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

        profile.env = self.env_state.to_env();
        profile.disabled_env = self.env_state.disabled_keys.clone();
        Ok(())
    }

    pub fn delete_profile(&mut self) -> bool {
        if let ProfileMode::ConfirmDelete(ref name) = self.profile_mode {
            let name = name.clone();
            if let Some(ref mut config) = self.profiles_config {
                if config.profiles.remove(&name).is_some() {
                    if config.active.as_ref() == Some(&name) {
                        config.active = None;
                    }
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

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Messages specific to the Settings screen.
#[derive(Debug)]
pub enum SettingsMessage {
    Refresh,
    NextCategory,
    PrevCategory,
    SelectNext,
    SelectPrev,
    Activate,
    Edit,
    Delete,
    Save,
    Cancel,
    FormChar(char),
    FormBackspace,
    FormNextField,
    FormPrevField,
    FormCycleType,
    ToggleDeleteConfirm,
    ConfirmDelete,
    // Profile-specific
    ProfileAdd,
    ProfileEdit,
    ProfileDelete,
    ProfileToggleActive,
    ProfileEnvEdit,
    // Env edit
    EnvNew,
    EnvDelete,
    EnvToggleDisabled,
    EnvToggleKeyValue,
    EnvStartEdit,
    EnvConfirm,
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

fn panel_title_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn styled_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White))
        .title(title)
        .title_style(panel_title_style())
}

/// Render the settings screen into the given area.
pub fn render(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Category tabs
        Constraint::Min(0),    // Content
    ])
    .split(area);

    render_tabs(state, buf, chunks[0]);
    render_content(state, buf, chunks[1]);
}

fn render_tabs(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    let titles: Vec<Line> = SettingsCategory::ALL
        .iter()
        .map(|cat| {
            let style = if *cat == state.category {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::styled(cat.label().to_string(), style)
        })
        .collect();

    let tabs = Tabs::new(titles)
        .block(styled_block(" Categories "))
        .highlight_style(Style::default().fg(Color::Cyan))
        .select(state.category.index());

    Widget::render(tabs, area, buf);
}

fn render_content(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    match state.category {
        SettingsCategory::CustomAgents => render_custom_agents(state, buf, area),
        SettingsCategory::Environment => render_profiles(state, buf, area),
        SettingsCategory::AISettings => render_ai_settings(state, buf, area),
        _ => render_general_content(state, buf, area),
    }
}

fn render_general_content(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    let items = state.category_items();

    if items.is_empty() {
        let text = if state.settings.is_none() {
            "Settings not loaded. Loading..."
        } else {
            "No settings in this category"
        };
        let paragraph = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(styled_block(" Settings "));
        Widget::render(paragraph, area, buf);
        return;
    }

    let (list_area, desc_area) = if area.height >= 6 {
        let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(area);
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

    let category_name = state.category.label();
    let title = format!(" {} Settings ", category_name);
    let list = List::new(list_items).block(styled_block(&title));
    Widget::render(list, list_area, buf);

    if let Some(desc_area) = desc_area {
        let description = selected_description(state);
        let paragraph = Paragraph::new(description)
            .wrap(Wrap { trim: true })
            .block(styled_block(" Description "));
        Widget::render(paragraph, desc_area, buf);
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
        SettingsCategory::Agent => match state.selected_item {
            0 => "Default coding agent for quick start.",
            1 => "If false, dependency install is skipped before launch.",
            2 => "Override path to Claude executable.",
            _ => "",
        },
        SettingsCategory::CustomAgents => "Manage custom coding agents.",
        SettingsCategory::Environment => "Manage environment profiles.",
        SettingsCategory::AISettings => "Configure AI settings (endpoint, key, model).",
    }
}

fn render_custom_agents(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    match &state.custom_agent_mode {
        CustomAgentMode::List => render_custom_agents_list(state, buf, area),
        CustomAgentMode::Add | CustomAgentMode::Edit(_) => {
            render_agent_form(state, buf, area);
        }
        CustomAgentMode::ConfirmDelete(id) => {
            render_agent_delete_confirm(state, buf, area, id);
        }
    }
}

fn render_custom_agents_list(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    let agents = state.custom_agents();

    let mut list_items: Vec<ListItem> = agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let type_str = match agent.agent_type {
                AgentType::Command => "cmd",
                AgentType::Path => "path",
                AgentType::Bunx => "bunx",
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

    let add_style = if state.is_add_agent_selected() {
        Style::default()
            .add_modifier(Modifier::REVERSED)
            .fg(Color::Green)
    } else {
        Style::default().fg(Color::Green)
    };
    list_items.push(ListItem::new("  + Add new custom agent...").style(add_style));

    let list = List::new(list_items).block(styled_block(" Custom Coding Agents "));
    Widget::render(list, area, buf);
}

fn render_agent_form(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    let form = &state.agent_form;
    let is_edit = matches!(state.custom_agent_mode, CustomAgentMode::Edit(_));
    let title = if is_edit {
        " Edit Custom Agent "
    } else {
        " Add Custom Agent "
    };

    let block = styled_block(title);
    let inner = block.inner(area);
    Widget::render(block, area, buf);

    let field_height = 3u16;
    let fields = AgentFormField::all();
    let constraints: Vec<Constraint> = fields
        .iter()
        .map(|_| Constraint::Length(field_height))
        .chain(std::iter::once(Constraint::Min(0)))
        .collect();

    let chunks = Layout::vertical(constraints).margin(1).split(inner);

    for (i, field) in fields.iter().enumerate() {
        let is_selected = *field == form.current_field;

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

        let display_text = if show_cursor {
            let mut text = String::from(value);
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

        let hint = if is_selected && *field == AgentFormField::Type {
            " (Space to cycle)"
        } else {
            ""
        };

        let field_title = format!(" {} ", field.label());
        let field_block = styled_block(&field_title).border_style(field_style);
        let paragraph = Paragraph::new(format!("{}{}", display_text, hint)).block(field_block);
        Widget::render(paragraph, chunks[i], buf);
    }
}

fn render_agent_delete_confirm(
    state: &SettingsState,
    buf: &mut Buffer,
    area: Rect,
    agent_id: &str,
) {
    let display_name = state
        .custom_agents()
        .iter()
        .find(|a| a.id == agent_id)
        .map(|a| a.display_name.as_str())
        .unwrap_or(agent_id);

    let yes_style = if state.delete_confirm {
        Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    let no_style = if !state.delete_confirm {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };

    let text = vec![
        Line::from(format!("Delete agent \"{}\"?", display_name)),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Yes ", yes_style),
            Span::raw("  "),
            Span::styled(" No ", no_style),
        ]),
    ];

    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(styled_block(" Confirm Delete "));
    Widget::render(paragraph, area, buf);
}

fn render_profiles(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    match &state.profile_mode {
        ProfileMode::List => render_profile_list(state, buf, area),
        ProfileMode::Add | ProfileMode::Edit(_) => render_profile_form(state, buf, area),
        ProfileMode::ConfirmDelete(name) => {
            let name = name.clone();
            render_profile_delete_confirm(state, buf, area, &name);
        }
        ProfileMode::EnvEdit(_) => render_env_edit(state, buf, area),
    }
}

pub fn render_profiles_tab(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    render_profiles(state, buf, area);
}

fn render_profile_list(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    let names = state.profile_names();

    let mut list_items: Vec<ListItem> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let active_marker = if state.is_profile_active(name) {
                " [active]"
            } else {
                ""
            };
            let desc = state
                .profiles_config
                .as_ref()
                .and_then(|c| c.profiles.get(name))
                .map(|p| {
                    if p.description.is_empty() {
                        String::new()
                    } else {
                        format!(" - {}", p.description)
                    }
                })
                .unwrap_or_default();

            let content = format!("  {}{}{}", name, active_marker, desc);
            let style = if i == state.profile_index {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let add_style = if state.is_add_profile_selected() {
        Style::default()
            .add_modifier(Modifier::REVERSED)
            .fg(Color::Green)
    } else {
        Style::default().fg(Color::Green)
    };
    list_items.push(ListItem::new("  + Add new profile...").style(add_style));

    let list = List::new(list_items).block(styled_block(" Environment Profiles "));
    Widget::render(list, area, buf);
}

fn render_profile_form(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    let form = &state.profile_form;
    let is_edit = matches!(state.profile_mode, ProfileMode::Edit(_));
    let title = if is_edit {
        " Edit Profile "
    } else {
        " Add Profile "
    };

    let block = styled_block(title);
    let inner = block.inner(area);
    Widget::render(block, area, buf);

    let fields = ProfileFormField::all();
    let constraints: Vec<Constraint> = fields
        .iter()
        .map(|_| Constraint::Length(3))
        .chain(std::iter::once(Constraint::Min(0)))
        .collect();

    let chunks = Layout::vertical(constraints).margin(1).split(inner);

    for (i, field) in fields.iter().enumerate() {
        let is_selected = *field == form.current_field;
        let value = match field {
            ProfileFormField::Name => &form.name,
            ProfileFormField::Description => &form.description,
        };

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

        let field_title = format!(" {} ", field.label());
        let field_block = styled_block(&field_title).border_style(field_style);
        let paragraph = Paragraph::new(display_text).block(field_block);
        Widget::render(paragraph, chunks[i], buf);
    }
}

fn render_profile_delete_confirm(
    state: &SettingsState,
    buf: &mut Buffer,
    area: Rect,
    profile_name: &str,
) {
    let yes_style = if state.profile_delete_confirm {
        Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    let no_style = if !state.profile_delete_confirm {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };

    let text = vec![
        Line::from(format!("Delete profile \"{}\"?", profile_name)),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Yes ", yes_style),
            Span::raw("  "),
            Span::styled(" No ", no_style),
        ]),
    ];

    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(styled_block(" Confirm Delete "));
    Widget::render(paragraph, area, buf);
}

fn render_env_edit(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    let env = &state.env_state;
    let layout = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).split(area);

    let display_items = env.display_items();
    if display_items.is_empty() {
        let paragraph = Paragraph::new("No environment variables. Press 'n' to add.")
            .alignment(Alignment::Center)
            .block(styled_block(" Environment Variables "));
        Widget::render(paragraph, layout[0], buf);
        render_env_footer(env, buf, layout[1]);
        return;
    }

    let list_items: Vec<ListItem> = display_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == env.selected_index;
            let kind_marker = match item.kind {
                EnvDisplayKind::Overridden => "[OVR]",
                EnvDisplayKind::Added => "[ADD]",
                EnvDisplayKind::OsDisabled => "[OFF]",
                EnvDisplayKind::OsOnly => "[OS ]",
            };
            let display = if let Some(ref edit_mode) = env.editing {
                if is_selected {
                    match edit_mode {
                        EnvEditMode::Key(cursor) => {
                            let mut k = item.key.clone();
                            let pos = (*cursor).min(k.len());
                            k.insert(pos, '|');
                            format!(" {kind_marker} {} = {}", k, item.value)
                        }
                        EnvEditMode::Value(cursor) => {
                            let mut v = item.value.clone();
                            let pos = (*cursor).min(v.len());
                            v.insert(pos, '|');
                            format!(" {kind_marker} {} = {}", item.key, v)
                        }
                    }
                } else {
                    format!(" {kind_marker} {} = {}", item.key, item.value)
                }
            } else {
                format!(" {kind_marker} {} = {}", item.key, item.value)
            };

            let style = if is_selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                match item.kind {
                    EnvDisplayKind::Overridden => Style::default().fg(Color::Yellow),
                    EnvDisplayKind::Added => Style::default().fg(Color::Green),
                    EnvDisplayKind::OsDisabled => Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::CROSSED_OUT),
                    EnvDisplayKind::OsOnly => Style::default(),
                }
            };
            ListItem::new(display).style(style)
        })
        .collect();

    let list = List::new(list_items).block(styled_block(" Environment Variables "));
    Widget::render(list, layout[0], buf);
    render_env_footer(env, buf, layout[1]);
}

fn render_env_footer(env: &EnvEditState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }

    let hint = if env.editing.is_some() {
        "[Enter] Confirm  [Tab] Key/Value  [Esc] Save & Back"
    } else if env.selected_is_os_entry() {
        "[Enter] Override  [Space] Disable/Enable  [n] Add  [Esc] Save & Back"
    } else if env.selected_is_overridden() {
        "[Enter] Edit Override  [d] Delete Override  [n] Add  [Esc] Save & Back"
    } else if env.selected_is_added() {
        "[Enter] Edit  [d] Delete  [n] Add  [Esc] Save & Back"
    } else {
        "[Enter] Edit  [n] Add  [Esc] Save & Back"
    };

    let span = Span::styled(hint, Style::default().fg(Color::DarkGray));
    buf.set_span(area.x, area.y, &span, area.width);

    if area.height > 1 {
        let legend = Span::styled(
            "[OS] inherited  [OVR] override  [ADD] added  [OFF] disabled",
            Style::default().fg(Color::DarkGray),
        );
        buf.set_span(area.x, area.y + 1, &legend, area.width);
    }
}

fn render_ai_settings(state: &SettingsState, buf: &mut Buffer, area: Rect) {
    let ai_info = state
        .profiles_config
        .as_ref()
        .and_then(|c| c.active_profile())
        .and_then(|p| p.ai.as_ref());

    let text = if let Some(ai) = ai_info {
        vec![
            Line::from(vec![
                Span::styled("Endpoint: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(&ai.endpoint),
            ]),
            Line::from(vec![
                Span::styled("Model:    ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(if ai.model.is_empty() {
                    "Not set"
                } else {
                    &ai.model
                }),
            ]),
            Line::from(vec![
                Span::styled("API Key:  ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(if ai.api_key.is_empty() {
                    "Not set"
                } else {
                    "********"
                }),
            ]),
            Line::from(vec![
                Span::styled("Language: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(&ai.language),
            ]),
            Line::from(vec![
                Span::styled("Summary:  ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(if ai.summary_enabled {
                    "Enabled"
                } else {
                    "Disabled"
                }),
            ]),
        ]
    } else {
        vec![
            Line::from("No AI settings configured."),
            Line::from("Set up via profile AI settings or wizard."),
        ]
    };

    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: true })
        .block(styled_block(" AI Settings "));
    Widget::render(paragraph, area, buf);
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

/// Handle a key event in the settings screen.
pub fn handle_key(state: &SettingsState, key: &KeyEvent) -> Option<SettingsMessage> {
    // Form mode (custom agent or profile)
    if state.is_form_mode() {
        return handle_form_key(key);
    }

    // Delete confirmation mode (agent)
    if state.is_delete_mode() {
        return handle_delete_confirm_key(key);
    }

    // Profile delete confirmation
    if matches!(state.profile_mode, ProfileMode::ConfirmDelete(_)) {
        return handle_profile_delete_confirm_key(key);
    }

    // Env edit mode
    if matches!(state.profile_mode, ProfileMode::EnvEdit(_)) {
        return handle_env_edit_key(state, key);
    }

    match key.code {
        KeyCode::Left | KeyCode::Char('h') => Some(SettingsMessage::PrevCategory),
        KeyCode::Right | KeyCode::Char('l') => Some(SettingsMessage::NextCategory),
        KeyCode::Up | KeyCode::Char('k') => Some(SettingsMessage::SelectPrev),
        KeyCode::Down | KeyCode::Char('j') => Some(SettingsMessage::SelectNext),
        KeyCode::Enter => match state.category {
            SettingsCategory::CustomAgents => Some(SettingsMessage::Edit),
            SettingsCategory::Environment => {
                if state.is_add_profile_selected() {
                    Some(SettingsMessage::ProfileAdd)
                } else {
                    Some(SettingsMessage::ProfileEnvEdit)
                }
            }
            _ => None,
        },
        KeyCode::Char('d') | KeyCode::Char('D') => match state.category {
            SettingsCategory::CustomAgents if !state.is_add_agent_selected() => {
                Some(SettingsMessage::Delete)
            }
            SettingsCategory::Environment if !state.is_add_profile_selected() => {
                Some(SettingsMessage::ProfileDelete)
            }
            _ => None,
        },
        KeyCode::Char('e') | KeyCode::Char('E') => {
            if state.category == SettingsCategory::Environment && !state.is_add_profile_selected() {
                Some(SettingsMessage::ProfileEdit)
            } else {
                None
            }
        }
        KeyCode::Char(' ') => {
            if state.category == SettingsCategory::Environment && !state.is_add_profile_selected() {
                Some(SettingsMessage::ProfileToggleActive)
            } else {
                None
            }
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(SettingsMessage::Refresh)
        }
        _ => None,
    }
}

fn handle_form_key(key: &KeyEvent) -> Option<SettingsMessage> {
    match key.code {
        KeyCode::Esc => Some(SettingsMessage::Cancel),
        KeyCode::Enter => Some(SettingsMessage::Save),
        KeyCode::Tab | KeyCode::Down => Some(SettingsMessage::FormNextField),
        KeyCode::BackTab | KeyCode::Up => Some(SettingsMessage::FormPrevField),
        KeyCode::Char(' ') => Some(SettingsMessage::FormCycleType),
        KeyCode::Backspace => Some(SettingsMessage::FormBackspace),
        KeyCode::Char(c) => Some(SettingsMessage::FormChar(c)),
        _ => None,
    }
}

fn handle_delete_confirm_key(key: &KeyEvent) -> Option<SettingsMessage> {
    match key.code {
        KeyCode::Esc => Some(SettingsMessage::Cancel),
        KeyCode::Left | KeyCode::Right => Some(SettingsMessage::ToggleDeleteConfirm),
        KeyCode::Enter => Some(SettingsMessage::ConfirmDelete),
        _ => None,
    }
}

fn handle_profile_delete_confirm_key(key: &KeyEvent) -> Option<SettingsMessage> {
    match key.code {
        KeyCode::Esc => Some(SettingsMessage::Cancel),
        KeyCode::Left | KeyCode::Right => Some(SettingsMessage::ToggleDeleteConfirm),
        KeyCode::Enter => Some(SettingsMessage::ConfirmDelete),
        _ => None,
    }
}

fn handle_env_edit_key(state: &SettingsState, key: &KeyEvent) -> Option<SettingsMessage> {
    if state.env_state.editing.is_some() {
        // In edit mode
        match key.code {
            KeyCode::Esc => Some(SettingsMessage::Cancel),
            KeyCode::Enter => Some(SettingsMessage::EnvConfirm),
            KeyCode::Tab => Some(SettingsMessage::EnvToggleKeyValue),
            KeyCode::Backspace => Some(SettingsMessage::FormBackspace),
            KeyCode::Char(c) => Some(SettingsMessage::FormChar(c)),
            _ => None,
        }
    } else {
        // Navigation mode
        match key.code {
            KeyCode::Esc => Some(SettingsMessage::Cancel),
            KeyCode::Up | KeyCode::Char('k') => Some(SettingsMessage::SelectPrev),
            KeyCode::Down | KeyCode::Char('j') => Some(SettingsMessage::SelectNext),
            KeyCode::Enter => Some(SettingsMessage::EnvStartEdit),
            KeyCode::Char('n') | KeyCode::Char('N') => Some(SettingsMessage::EnvNew),
            KeyCode::Char('d') | KeyCode::Char('D') => Some(SettingsMessage::EnvDelete),
            KeyCode::Char(' ') => Some(SettingsMessage::EnvToggleDisabled),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_state_default_starts_at_general() {
        let state = SettingsState::new();
        assert_eq!(state.category, SettingsCategory::General);
        assert_eq!(state.selected_item, 0);
        assert!(state.settings.is_none());
    }

    #[test]
    fn category_navigation_wraps() {
        let mut state = SettingsState::new();
        assert_eq!(state.category, SettingsCategory::General);

        state.next_category();
        assert_eq!(state.category, SettingsCategory::Worktree);

        state.next_category();
        assert_eq!(state.category, SettingsCategory::Agent);

        state.next_category();
        assert_eq!(state.category, SettingsCategory::CustomAgents);

        state.next_category();
        assert_eq!(state.category, SettingsCategory::Environment);

        state.next_category();
        assert_eq!(state.category, SettingsCategory::AISettings);

        state.next_category();
        assert_eq!(state.category, SettingsCategory::General); // wraps

        state.prev_category();
        assert_eq!(state.category, SettingsCategory::AISettings); // wraps back
    }

    #[test]
    fn category_items_empty_when_no_settings() {
        let state = SettingsState::new();
        assert!(state.category_items().is_empty());
    }

    #[test]
    fn category_items_general_with_settings() {
        let mut state = SettingsState::new();
        state.settings = Some(Settings::default());
        let items = state.category_items();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, "Default Base Branch");
    }

    #[test]
    fn custom_agent_form_validation() {
        let form = AgentFormState::new();
        assert!(form.validate().is_err());

        let mut form = AgentFormState::new();
        form.id = "test-agent".to_string();
        form.display_name = "Test Agent".to_string();
        form.command = "/usr/bin/test".to_string();
        assert!(form.validate().is_ok());
    }

    #[test]
    fn custom_agent_form_char_insert_delete() {
        let mut form = AgentFormState::new();
        form.current_field = AgentFormField::Id;
        form.cursor = 0;

        form.insert_char('a');
        form.insert_char('b');
        assert_eq!(form.id, "ab");
        assert_eq!(form.cursor, 2);

        form.delete_char();
        assert_eq!(form.id, "a");
        assert_eq!(form.cursor, 1);
    }

    #[test]
    fn custom_agent_form_field_navigation() {
        let mut form = AgentFormState::new();
        assert_eq!(form.current_field, AgentFormField::Id);

        form.next_field();
        assert_eq!(form.current_field, AgentFormField::DisplayName);

        form.next_field();
        assert_eq!(form.current_field, AgentFormField::Type);

        form.prev_field();
        assert_eq!(form.current_field, AgentFormField::DisplayName);
    }

    #[test]
    fn custom_agent_type_cycle() {
        let mut form = AgentFormState::new();
        assert_eq!(form.agent_type, AgentType::Command);

        form.cycle_type();
        assert_eq!(form.agent_type, AgentType::Path);

        form.cycle_type();
        assert_eq!(form.agent_type, AgentType::Bunx);

        form.cycle_type();
        assert_eq!(form.agent_type, AgentType::Command);
    }

    #[test]
    fn profile_form_validation() {
        let form = ProfileFormState::new();
        assert!(form.validate().is_err());

        let mut form = ProfileFormState::new();
        form.name = "test-profile".to_string();
        assert!(form.validate().is_ok());

        form.name = "invalid name!".to_string();
        assert!(form.validate().is_err());
    }

    #[test]
    fn profile_crud_operations() {
        let mut state = SettingsState::new();
        state.profiles_config = Some(ProfilesConfig {
            version: 1,
            active: None,
            profiles: HashMap::new(),
        });

        // Add profile
        state.enter_profile_add_mode();
        assert!(matches!(state.profile_mode, ProfileMode::Add));
        state.profile_form.name = "test".to_string();
        state.profile_form.description = "Test profile".to_string();
        assert!(state.save_profile().is_ok());
        assert_eq!(state.profile_names().len(), 1);

        // Toggle active
        state.profile_index = 0;
        state.toggle_active_profile();
        assert!(state.is_profile_active("test"));

        // Delete
        state.enter_profile_delete_mode();
        assert!(matches!(state.profile_mode, ProfileMode::ConfirmDelete(_)));
        assert!(state.delete_profile());
        assert!(state.profile_names().is_empty());
    }

    #[test]
    fn env_edit_state_operations() {
        let mut profile = Profile::new("test");
        profile.env.insert("FOO".to_string(), "bar".to_string());
        profile.env.insert("BAZ".to_string(), "qux".to_string());

        let mut env = EnvEditState::from_profile(&profile);
        assert_eq!(env.vars.len(), 2);
        // sorted by key
        assert_eq!(env.vars[0].0, "BAZ");
        assert_eq!(env.vars[1].0, "FOO");

        env.add_new_var();
        assert_eq!(env.vars.len(), 3);
        assert_eq!(env.selected_index, 0);

        env.delete_selected();
        assert_eq!(env.vars.len(), 2);

        let env_map = env.to_env();
        assert_eq!(env_map.len(), 2);
    }

    #[test]
    fn env_edit_state_classifies_os_and_profile_entries() {
        let mut profile = Profile::new("test");
        profile
            .env
            .insert("TOKEN".to_string(), "override".to_string());
        profile.env.insert("NEW".to_string(), "added".to_string());
        profile.disabled_env.push("HOME".to_string());

        let mut env = EnvEditState::from_profile(&profile);
        env.os_vars = vec![
            ("HOME".to_string(), "/tmp".to_string()),
            ("PATH".to_string(), "/bin".to_string()),
            ("TOKEN".to_string(), "os-token".to_string()),
        ];

        let items = env.display_items();
        let home = items.iter().find(|item| item.key == "HOME").unwrap();
        assert_eq!(home.kind, EnvDisplayKind::OsDisabled);
        let path = items.iter().find(|item| item.key == "PATH").unwrap();
        assert_eq!(path.kind, EnvDisplayKind::OsOnly);
        let token = items.iter().find(|item| item.key == "TOKEN").unwrap();
        assert_eq!(token.kind, EnvDisplayKind::Overridden);
        let added = items.iter().find(|item| item.key == "NEW").unwrap();
        assert_eq!(added.kind, EnvDisplayKind::Added);
    }

    #[test]
    fn env_edit_state_toggle_selected_disabled_marks_os_entry() {
        let profile = Profile::new("test");
        let mut env = EnvEditState::from_profile(&profile);
        env.os_vars = vec![("PATH".to_string(), "/bin".to_string())];

        assert!(env.toggle_selected_disabled());
        assert_eq!(env.disabled_keys, vec!["PATH".to_string()]);
        assert!(!env.toggle_selected_disabled());
        assert!(env.disabled_keys.is_empty());
    }

    #[test]
    fn env_edit_state_start_edit_selected_on_os_entry_creates_override() {
        let profile = Profile::new("test");
        let mut env = EnvEditState::from_profile(&profile);
        env.os_vars = vec![("PATH".to_string(), "/bin".to_string())];

        env.start_edit_selected();

        assert_eq!(env.vars.len(), 1);
        assert_eq!(env.vars[0].0, "PATH");
        assert_eq!(env.vars[0].1, "/bin");
        assert!(env.selected_is_overridden() || env.selected_is_added());
    }

    #[test]
    fn persist_env_edit_saves_disabled_env() {
        let mut state = SettingsState::new();
        let profile = Profile::new("dev");
        let mut config = ProfilesConfig::default();
        config.profiles.insert("dev".to_string(), profile);
        state.profiles_config = Some(config);
        state.profile_mode = ProfileMode::EnvEdit("dev".to_string());
        state.env_state.os_vars = vec![("HOME".to_string(), "/tmp".to_string())];
        state.env_state.toggle_selected_disabled();

        state.persist_env_edit().unwrap();

        let saved = state
            .profiles_config
            .as_ref()
            .unwrap()
            .profiles
            .get("dev")
            .unwrap();
        assert_eq!(saved.disabled_env, vec!["HOME".to_string()]);
    }

    #[test]
    fn select_next_prev_general() {
        let mut state = SettingsState::new();
        state.settings = Some(Settings::default());

        state.select_next();
        assert_eq!(state.selected_item, 1);
        state.select_next();
        assert_eq!(state.selected_item, 2);
        state.select_next();
        assert_eq!(state.selected_item, 2); // clamped

        state.select_prev();
        assert_eq!(state.selected_item, 1);
    }

    #[test]
    fn render_smoke_test() {
        let state = SettingsState::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(&state, &mut buf, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn render_with_settings_smoke_test() {
        let mut state = SettingsState::new();
        state.settings = Some(Settings::default());
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(&state, &mut buf, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn render_custom_agents_smoke_test() {
        let mut state = SettingsState::new();
        state.category = SettingsCategory::CustomAgents;
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(&state, &mut buf, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn render_profiles_smoke_test() {
        let mut state = SettingsState::new();
        state.category = SettingsCategory::Environment;
        state.profiles_config = Some(ProfilesConfig {
            version: 1,
            active: None,
            profiles: HashMap::new(),
        });
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(&state, &mut buf, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn render_env_footer_shows_os_entry_actions() {
        let profile = Profile::new("test");
        let mut env = EnvEditState::from_profile(&profile);
        env.os_vars = vec![("PATH".to_string(), "/bin".to_string())];

        let area = Rect::new(0, 0, 80, 2);
        let mut buf = Buffer::empty(area);
        render_env_footer(&env, &mut buf, area);

        let text: String = (0..80)
            .map(|x| {
                buf.cell((x, 0))
                    .map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))
            })
            .collect();
        assert!(text.contains("Override"), "expected override hint, got: {text:?}");
    }

    #[test]
    fn render_env_footer_shows_override_actions() {
        let mut profile = Profile::new("test");
        profile.env.insert("PATH".to_string(), "/custom".to_string());
        let mut env = EnvEditState::from_profile(&profile);
        env.os_vars = vec![("PATH".to_string(), "/bin".to_string())];

        let area = Rect::new(0, 0, 80, 2);
        let mut buf = Buffer::empty(area);
        render_env_footer(&env, &mut buf, area);

        let text: String = (0..80)
            .map(|x| {
                buf.cell((x, 0))
                    .map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))
            })
            .collect();
        assert!(
            text.contains("Delete Override"),
            "expected delete override hint, got: {text:?}"
        );
    }

    #[test]
    fn render_ai_settings_smoke_test() {
        let mut state = SettingsState::new();
        state.category = SettingsCategory::AISettings;
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(&state, &mut buf, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn handle_key_category_navigation() {
        let state = SettingsState::new();
        let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        let msg = handle_key(&state, &key);
        assert!(matches!(msg, Some(SettingsMessage::NextCategory)));

        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        let msg = handle_key(&state, &key);
        assert!(matches!(msg, Some(SettingsMessage::PrevCategory)));
    }

    #[test]
    fn handle_key_item_navigation() {
        let state = SettingsState::new();
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let msg = handle_key(&state, &key);
        assert!(matches!(msg, Some(SettingsMessage::SelectNext)));

        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let msg = handle_key(&state, &key);
        assert!(matches!(msg, Some(SettingsMessage::SelectPrev)));
    }

    #[test]
    fn settings_message_is_debug() {
        let msg = SettingsMessage::Refresh;
        assert!(format!("{msg:?}").contains("Refresh"));
    }
}
