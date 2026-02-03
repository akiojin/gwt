//! AI Settings Wizard Screen
//!
//! FR-100: AI settings wizard with step-based flow
//! FR-101: URL -> API Key -> Model selection
//! FR-102: Connection check via GET /models API
//! FR-103: Block saving if connection fails
//! FR-104: Same UI for global and profile AI settings

#![allow(dead_code)]

use gwt_core::ai::{format_error_for_display, AIClient, AIError, ModelInfo};
use ratatui::{prelude::*, widgets::*};

/// AI Settings Wizard step (FR-101)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AIWizardStep {
    /// Step 1: Enter API endpoint URL
    #[default]
    Endpoint,
    /// Step 2: Enter API key (optional for local LLMs)
    ApiKey,
    /// Step 3: Fetching models (loading state)
    FetchingModels,
    /// Step 4: Select model from list
    ModelSelect,
}

/// AI Settings Wizard state
#[derive(Debug, Default)]
pub struct AIWizardState {
    /// Whether wizard is visible
    pub visible: bool,
    /// Current step
    pub step: AIWizardStep,
    /// Whether editing existing settings (vs creating new)
    pub is_edit: bool,
    /// Whether this is for default AI settings (vs profile-specific)
    pub is_default_ai: bool,
    /// Profile name (if not default)
    pub profile_name: Option<String>,

    // Input fields
    /// API endpoint URL
    pub endpoint: String,
    /// Cursor position for endpoint input
    pub endpoint_cursor: usize,
    /// API key
    pub api_key: String,
    /// Cursor position for API key input
    pub api_key_cursor: usize,

    // Model selection
    /// Available models from API
    pub models: Vec<ModelInfo>,
    /// Selected model index
    pub model_index: usize,
    /// Selected model ID
    pub selected_model: String,
    /// Scroll offset for model list
    pub model_scroll_offset: usize,

    // Status
    /// Error message (if any)
    pub error: Option<String>,
    /// Loading status message
    pub loading_message: Option<String>,
    /// Whether delete confirmation is shown
    pub show_delete_confirm: bool,

    // Mouse click support
    /// Popup area for mouse click detection
    pub popup_area: Option<Rect>,
    /// Model list inner area for click detection
    pub list_inner_area: Option<Rect>,
}

impl AIWizardState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open wizard for new AI settings
    pub fn open_new(&mut self, is_default_ai: bool, profile_name: Option<String>) {
        self.reset();
        self.visible = true;
        self.is_edit = false;
        self.is_default_ai = is_default_ai;
        self.profile_name = profile_name;
        // FR-105: Default endpoint
        self.endpoint = "https://api.openai.com/v1".to_string();
        self.endpoint_cursor = self.endpoint.len();
        self.step = AIWizardStep::Endpoint;
    }

    /// Open wizard for editing existing AI settings
    pub fn open_edit(
        &mut self,
        is_default_ai: bool,
        profile_name: Option<String>,
        endpoint: &str,
        api_key: &str,
        model: &str,
    ) {
        self.reset();
        self.visible = true;
        self.is_edit = true;
        self.is_default_ai = is_default_ai;
        self.profile_name = profile_name;
        self.endpoint = endpoint.to_string();
        self.endpoint_cursor = self.endpoint.len();
        self.api_key = api_key.to_string();
        self.api_key_cursor = self.api_key.len();
        self.selected_model = model.to_string();
        self.step = AIWizardStep::Endpoint;
    }

    /// Close wizard
    pub fn close(&mut self) {
        self.visible = false;
        self.reset();
    }

    /// Reset all state
    fn reset(&mut self) {
        self.step = AIWizardStep::Endpoint;
        self.is_edit = false;
        self.is_default_ai = false;
        self.profile_name = None;
        self.endpoint.clear();
        self.endpoint_cursor = 0;
        self.api_key.clear();
        self.api_key_cursor = 0;
        self.models.clear();
        self.model_index = 0;
        self.selected_model.clear();
        self.model_scroll_offset = 0;
        self.error = None;
        self.loading_message = None;
        self.show_delete_confirm = false;
    }

    /// Advance to next step
    pub fn next_step(&mut self) {
        match self.step {
            AIWizardStep::Endpoint => {
                if self.endpoint.trim().is_empty() {
                    self.error = Some("Endpoint URL is required".to_string());
                } else {
                    self.error = None;
                    self.step = AIWizardStep::ApiKey;
                }
            }
            AIWizardStep::ApiKey => {
                self.error = None;
                self.step = AIWizardStep::FetchingModels;
            }
            AIWizardStep::FetchingModels => {
                // This is handled externally after fetch completes
            }
            AIWizardStep::ModelSelect => {
                // Confirm selection - handled externally
            }
        }
    }

    /// Go back to previous step
    pub fn prev_step(&mut self) {
        match self.step {
            AIWizardStep::Endpoint => {
                // Close wizard
                self.close();
            }
            AIWizardStep::ApiKey => {
                self.error = None;
                self.step = AIWizardStep::Endpoint;
            }
            AIWizardStep::FetchingModels => {
                // Cannot go back while fetching
            }
            AIWizardStep::ModelSelect => {
                self.error = None;
                self.step = AIWizardStep::ApiKey;
            }
        }
    }

    /// Fetch models from API (blocking)
    pub fn fetch_models(&mut self) -> Result<(), AIError> {
        let client = AIClient::new_for_list_models(self.endpoint.trim(), self.api_key.trim())?;
        let models = client.list_models()?;
        self.apply_models(models)
    }

    /// Apply fetched model list to state
    pub fn apply_models(&mut self, mut models: Vec<ModelInfo>) -> Result<(), AIError> {
        if models.is_empty() {
            return Err(AIError::ConfigError("No models available".to_string()));
        }

        // Sort models by ID for consistent display
        models.sort_by(|a, b| a.id.cmp(&b.id));
        self.models = models;

        // If we have a previously selected model, find it in the list
        if !self.selected_model.is_empty() {
            if let Some(idx) = self.models.iter().position(|m| m.id == self.selected_model) {
                self.model_index = idx;
            } else {
                self.model_index = 0;
            }
        } else {
            self.model_index = 0;
        }
        self.model_scroll_offset = 0;

        Ok(())
    }

    /// Mark fetch as complete and move to model selection
    pub fn fetch_complete(&mut self) {
        self.loading_message = None;
        self.step = AIWizardStep::ModelSelect;
    }

    /// Mark fetch as failed
    pub fn fetch_failed(&mut self, error: &AIError) {
        self.loading_message = None;
        self.error = Some(format_error_for_display(error));
        self.step = AIWizardStep::ApiKey; // Go back to API key step
    }

    /// Get currently selected model
    pub fn current_model(&self) -> Option<&ModelInfo> {
        self.models.get(self.model_index)
    }

    /// Select next model in list
    pub fn select_next_model(&mut self) {
        if self.model_index < self.models.len().saturating_sub(1) {
            self.model_index += 1;
        }
        self.ensure_model_visible(self.model_visible_height());
    }

    /// Select previous model in list
    pub fn select_prev_model(&mut self) {
        if self.model_index > 0 {
            self.model_index -= 1;
        }
        self.ensure_model_visible(self.model_visible_height());
    }

    /// Ensure selected model is visible within viewport
    fn ensure_model_visible(&mut self, visible_height: usize) {
        if self.models.is_empty() {
            self.model_scroll_offset = 0;
            return;
        }
        if visible_height == 0 {
            return;
        }

        if self.model_index < self.model_scroll_offset {
            self.model_scroll_offset = self.model_index;
        } else if self.model_index >= self.model_scroll_offset + visible_height {
            self.model_scroll_offset = self.model_index + 1 - visible_height;
        }

        let max_offset = self.models.len().saturating_sub(visible_height);
        if self.model_scroll_offset > max_offset {
            self.model_scroll_offset = max_offset;
        }
    }

    fn model_visible_height(&self) -> usize {
        self.list_inner_area
            .map(|area| area.height as usize)
            .unwrap_or(0)
    }

    // Input handling methods
    pub fn insert_char(&mut self, c: char) {
        match self.step {
            AIWizardStep::Endpoint => {
                self.endpoint.insert(self.endpoint_cursor, c);
                self.endpoint_cursor += 1;
            }
            AIWizardStep::ApiKey => {
                self.api_key.insert(self.api_key_cursor, c);
                self.api_key_cursor += 1;
            }
            _ => {}
        }
        self.error = None;
    }

    pub fn delete_char(&mut self) {
        match self.step {
            AIWizardStep::Endpoint => {
                if self.endpoint_cursor > 0 {
                    self.endpoint_cursor -= 1;
                    self.endpoint.remove(self.endpoint_cursor);
                }
            }
            AIWizardStep::ApiKey => {
                if self.api_key_cursor > 0 {
                    self.api_key_cursor -= 1;
                    self.api_key.remove(self.api_key_cursor);
                }
            }
            _ => {}
        }
        self.error = None;
    }

    pub fn cursor_left(&mut self) {
        match self.step {
            AIWizardStep::Endpoint => {
                if self.endpoint_cursor > 0 {
                    self.endpoint_cursor -= 1;
                }
            }
            AIWizardStep::ApiKey => {
                if self.api_key_cursor > 0 {
                    self.api_key_cursor -= 1;
                }
            }
            _ => {}
        }
    }

    pub fn cursor_right(&mut self) {
        match self.step {
            AIWizardStep::Endpoint => {
                if self.endpoint_cursor < self.endpoint.len() {
                    self.endpoint_cursor += 1;
                }
            }
            AIWizardStep::ApiKey => {
                if self.api_key_cursor < self.api_key.len() {
                    self.api_key_cursor += 1;
                }
            }
            _ => {}
        }
    }

    /// Check if currently in text input mode
    pub fn is_text_input(&self) -> bool {
        matches!(self.step, AIWizardStep::Endpoint | AIWizardStep::ApiKey)
    }

    /// Show delete confirmation dialog
    pub fn show_delete(&mut self) {
        self.show_delete_confirm = true;
    }

    /// Hide delete confirmation dialog
    pub fn cancel_delete(&mut self) {
        self.show_delete_confirm = false;
    }

    /// Get wizard title
    pub fn title(&self) -> &'static str {
        if self.is_edit {
            "Edit AI Settings"
        } else {
            "Configure AI Settings"
        }
    }

    /// Get step title
    pub fn step_title(&self) -> &'static str {
        match self.step {
            AIWizardStep::Endpoint => "Step 1/3: API Endpoint",
            AIWizardStep::ApiKey => "Step 2/3: API Key",
            AIWizardStep::FetchingModels => "Connecting...",
            AIWizardStep::ModelSelect => "Step 3/3: Select Model",
        }
    }

    /// Get settings target description
    pub fn target_description(&self) -> String {
        if self.is_default_ai {
            "Default AI Settings".to_string()
        } else if let Some(name) = &self.profile_name {
            format!("Profile: {}", name)
        } else {
            "AI Settings".to_string()
        }
    }

    // Mouse click support methods

    /// Check if point is within popup area
    pub fn is_point_in_popup(&self, x: u16, y: u16) -> bool {
        self.popup_area.is_some_and(|area| {
            x >= area.x
                && x < area.x.saturating_add(area.width)
                && y >= area.y
                && y < area.y.saturating_add(area.height)
        })
    }

    /// Get model selection index from screen coordinates
    pub fn selection_index_from_point(&self, x: u16, y: u16) -> Option<usize> {
        let area = self.list_inner_area?;
        if x < area.x || x >= area.x.saturating_add(area.width) {
            return None;
        }
        if y < area.y || y >= area.y.saturating_add(area.height) {
            return None;
        }
        let relative_y = y.saturating_sub(area.y) as usize;
        let index = self.model_scroll_offset.saturating_add(relative_y);
        if index < self.models.len() {
            Some(index)
        } else {
            None
        }
    }

    /// Select model by index
    pub fn select_model_index(&mut self, index: usize) -> bool {
        if index < self.models.len() {
            self.model_index = index;
            self.ensure_model_visible(self.model_visible_height());
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;

    fn model(id: &str) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            created: 0,
            owned_by: "test".to_string(),
        }
    }

    #[test]
    fn test_apply_models_selects_existing_model() {
        let mut state = AIWizardState::new();
        state.selected_model = "gpt-5".to_string();
        let models = vec![model("gpt-4"), model("gpt-5"), model("gpt-3.5")];

        state.apply_models(models).unwrap();

        assert_eq!(state.models.len(), 3);
        assert_eq!(state.models[state.model_index].id, "gpt-5");
    }

    #[test]
    fn test_apply_models_empty_fails() {
        let mut state = AIWizardState::new();
        let result = state.apply_models(Vec::new());
        assert!(matches!(result, Err(AIError::ConfigError(_))));
    }

    #[test]
    fn test_fetch_complete_sets_step() {
        let mut state = AIWizardState::new();
        state.loading_message = Some("Fetching".to_string());
        state.step = AIWizardStep::FetchingModels;

        state.fetch_complete();

        assert!(state.loading_message.is_none());
        assert_eq!(state.step, AIWizardStep::ModelSelect);
    }

    #[test]
    fn test_fetch_failed_sets_error_and_step() {
        let mut state = AIWizardState::new();
        state.loading_message = Some("Fetching".to_string());
        state.step = AIWizardStep::FetchingModels;

        state.fetch_failed(&AIError::ConfigError("bad".to_string()));

        assert!(state.loading_message.is_none());
        assert_eq!(state.step, AIWizardStep::ApiKey);
        assert!(state.error.is_some());
    }
}

/// Render AI settings wizard
pub fn render_ai_wizard(state: &mut AIWizardState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }

    // Calculate popup size (60% width, 50% height)
    let popup_width = (area.width * 60 / 100).clamp(50, 80);
    let popup_height = (area.height * 50 / 100).clamp(15, 25);

    let popup_area = Rect {
        x: area.x + (area.width - popup_width) / 2,
        y: area.y + (area.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    // Record popup area for mouse click detection
    state.popup_area = Some(popup_area);

    // Clear background
    frame.render_widget(Clear, popup_area);

    // Popup block
    let title = format!(" {} ", state.title());
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(Color::Yellow).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Target + step header
            Constraint::Length(1), // Separator
            Constraint::Min(5),    // Content
            Constraint::Length(2), // Footer/actions
        ])
        .split(inner);

    // Header: target and step
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Target: ", Style::default().fg(Color::DarkGray)),
            Span::styled(state.target_description(), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![Span::styled(
            state.step_title(),
            Style::default().fg(Color::Yellow),
        )]),
    ]);
    frame.render_widget(header, chunks[0]);

    // Separator
    let separator = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(separator, chunks[1]);

    // Content based on step
    match state.step {
        AIWizardStep::Endpoint => render_endpoint_step(state, frame, chunks[2]),
        AIWizardStep::ApiKey => render_api_key_step(state, frame, chunks[2]),
        AIWizardStep::FetchingModels => render_fetching_step(state, frame, chunks[2]),
        AIWizardStep::ModelSelect => render_model_select_step(state, frame, chunks[2]),
    }

    // Footer/actions
    let actions = match state.step {
        AIWizardStep::Endpoint => "[Enter] Next | [Esc] Cancel",
        AIWizardStep::ApiKey => {
            if state.is_edit {
                "[Enter] Connect | [Esc] Back | [d] Delete"
            } else {
                "[Enter] Connect | [Esc] Back"
            }
        }
        AIWizardStep::FetchingModels => "Connecting to API...",
        AIWizardStep::ModelSelect => "[Enter] Save | [Esc] Back | [Up/Down] Select",
    };
    let footer = Paragraph::new(actions)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(footer, chunks[3]);

    // Delete confirmation overlay
    if state.show_delete_confirm {
        render_delete_confirm(frame, area);
    }
}

fn render_endpoint_step(state: &AIWizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Label
            Constraint::Length(3), // Input
            Constraint::Min(0),    // Error or help
        ])
        .split(area);

    // Label
    let label = Paragraph::new("API Endpoint URL:").style(Style::default().fg(Color::White));
    frame.render_widget(label, chunks[0]);

    // Input field
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let input_inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);

    let display_text = if state.endpoint.is_empty() {
        "https://api.openai.com/v1".to_string()
    } else {
        state.endpoint.clone()
    };
    let text_style = if state.endpoint.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    let input = Paragraph::new(display_text).style(text_style);
    frame.render_widget(input, input_inner);

    // Cursor
    if !state.endpoint.is_empty() {
        frame.set_cursor_position((input_inner.x + state.endpoint_cursor as u16, input_inner.y));
    }

    // Error or help text
    if let Some(error) = &state.error {
        let error_text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        frame.render_widget(error_text, chunks[2]);
    } else {
        let help = Paragraph::new("Examples: https://api.openai.com/v1, http://localhost:11434/v1")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(help, chunks[2]);
    }
}

fn render_api_key_step(state: &AIWizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Label
            Constraint::Length(3), // Input
            Constraint::Min(0),    // Error or help
        ])
        .split(area);

    // Label
    let label = Paragraph::new("API Key (optional for local LLMs):")
        .style(Style::default().fg(Color::White));
    frame.render_widget(label, chunks[0]);

    // Input field (masked)
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let input_inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);

    let display_text = if state.api_key.is_empty() {
        "(press Enter to skip)".to_string()
    } else {
        "*".repeat(state.api_key.len())
    };
    let text_style = if state.api_key.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    let input = Paragraph::new(display_text).style(text_style);
    frame.render_widget(input, input_inner);

    // Cursor
    if !state.api_key.is_empty() {
        frame.set_cursor_position((input_inner.x + state.api_key_cursor as u16, input_inner.y));
    }

    // Error or help text
    if let Some(error) = &state.error {
        let error_text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        frame.render_widget(error_text, chunks[2]);
    } else {
        let help = Paragraph::new(
            "For OpenAI, enter your API key. For local LLMs like Ollama, leave empty.",
        )
        .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(help, chunks[2]);
    }
}

fn render_fetching_step(state: &AIWizardState, frame: &mut Frame, area: Rect) {
    let message = state
        .loading_message
        .as_deref()
        .unwrap_or("Fetching available models...");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    let loading = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            format!("  {}  ", message),
            Style::default().fg(Color::Yellow),
        )]),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(loading, chunks[1]);

    // Show endpoint being connected to
    let endpoint_info = Paragraph::new(format!("Endpoint: {}", state.endpoint.trim()))
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(endpoint_info, chunks[2]);
}

fn render_model_select_step(state: &mut AIWizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Label
            Constraint::Min(3),    // Model list
            Constraint::Length(1), // Count/info
        ])
        .split(area);

    // Label
    let label = Paragraph::new(format!("Select Model ({} available):", state.models.len()))
        .style(Style::default().fg(Color::White));
    frame.render_widget(label, chunks[0]);

    // Model list
    if state.models.is_empty() {
        let empty = Paragraph::new("No models available")
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
        state.list_inner_area = None;
    } else {
        let list_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        // Record list inner area for mouse click detection
        let list_inner = list_block.inner(chunks[1]);
        state.list_inner_area = Some(list_inner);
        let visible_height = list_inner.height as usize;
        state.ensure_model_visible(visible_height);

        let items: Vec<ListItem> = state
            .models
            .iter()
            .enumerate()
            .skip(state.model_scroll_offset)
            .take(visible_height)
            .map(|(i, model)| {
                let style = if i == state.model_index {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default()
                };
                let prefix = if i == state.model_index { "> " } else { "  " };
                ListItem::new(format!("{}{}", prefix, model.id)).style(style)
            })
            .collect();

        let list = List::new(items).block(list_block);
        frame.render_widget(list, chunks[1]);

        if state.models.len() > visible_height && visible_height > 0 {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("^"))
                .end_symbol(Some("v"));
            let mut scrollbar_state =
                ScrollbarState::new(state.models.len()).position(state.model_index);
            frame.render_stateful_widget(
                scrollbar,
                chunks[1].inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }
    }

    // Info
    if let Some(model) = state.current_model() {
        let info = if !model.owned_by.is_empty() {
            format!("Owner: {}", model.owned_by)
        } else {
            String::new()
        };
        let info_text = Paragraph::new(info).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(info_text, chunks[2]);
    }
}

fn render_delete_confirm(frame: &mut Frame, area: Rect) {
    // Small confirmation dialog
    let popup_width = 40;
    let popup_height = 7;
    let popup_area = Rect {
        x: area.x + (area.width - popup_width) / 2,
        y: area.y + (area.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Delete AI Settings ")
        .title_style(Style::default().fg(Color::Red).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let content = Paragraph::new(vec![
        Line::from(""),
        Line::from("Are you sure you want to delete"),
        Line::from("AI settings?"),
        Line::from(""),
        Line::from(vec![
            Span::styled("[y] ", Style::default().fg(Color::Red)),
            Span::raw("Yes  "),
            Span::styled("[n] ", Style::default().fg(Color::Green)),
            Span::raw("No"),
        ]),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(content, inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_step_navigation() {
        let mut state = AIWizardState::new();
        state.open_new(true, None);
        assert_eq!(state.step, AIWizardStep::Endpoint);
        assert!(!state.endpoint.is_empty()); // Default endpoint

        // Cannot advance with empty endpoint
        state.endpoint.clear();
        state.next_step();
        assert!(state.error.is_some());
        assert_eq!(state.step, AIWizardStep::Endpoint);

        // Can advance with endpoint
        state.endpoint = "http://localhost:11434/v1".to_string();
        state.next_step();
        assert!(state.error.is_none());
        assert_eq!(state.step, AIWizardStep::ApiKey);

        // Can go back
        state.prev_step();
        assert_eq!(state.step, AIWizardStep::Endpoint);
    }

    #[test]
    fn test_wizard_text_input() {
        let mut state = AIWizardState::new();
        state.open_new(false, Some("dev".to_string()));

        // Clear default endpoint
        state.endpoint.clear();
        state.endpoint_cursor = 0;

        // Type endpoint
        state.insert_char('h');
        state.insert_char('t');
        state.insert_char('t');
        state.insert_char('p');
        assert_eq!(state.endpoint, "http");
        assert_eq!(state.endpoint_cursor, 4);

        // Delete character
        state.delete_char();
        assert_eq!(state.endpoint, "htt");
        assert_eq!(state.endpoint_cursor, 3);

        // Cursor movement
        state.cursor_left();
        assert_eq!(state.endpoint_cursor, 2);
        state.cursor_right();
        assert_eq!(state.endpoint_cursor, 3);
    }

    #[test]
    fn test_wizard_model_selection() {
        let mut state = AIWizardState::new();
        state.models = vec![
            ModelInfo {
                id: "gpt-4".to_string(),
                created: 0,
                owned_by: "openai".to_string(),
            },
            ModelInfo {
                id: "gpt-3.5-turbo".to_string(),
                created: 0,
                owned_by: "openai".to_string(),
            },
        ];
        state.model_index = 0;
        state.step = AIWizardStep::ModelSelect;

        assert_eq!(state.current_model().unwrap().id, "gpt-4");

        state.select_next_model();
        assert_eq!(state.model_index, 1);
        assert_eq!(state.current_model().unwrap().id, "gpt-3.5-turbo");

        state.select_next_model();
        assert_eq!(state.model_index, 1); // Should not go beyond

        state.select_prev_model();
        assert_eq!(state.model_index, 0);
    }

    #[test]
    fn test_model_scroll_offset_keeps_selection_visible() {
        let mut state = AIWizardState::new();
        state.models = (0..10)
            .map(|i| ModelInfo {
                id: format!("model-{}", i),
                created: 0,
                owned_by: "test".to_string(),
            })
            .collect();

        state.model_index = 0;
        state.ensure_model_visible(3);
        assert_eq!(state.model_scroll_offset, 0);

        state.model_index = 4;
        state.ensure_model_visible(3);
        assert_eq!(state.model_scroll_offset, 2);

        state.model_index = 1;
        state.ensure_model_visible(3);
        assert_eq!(state.model_scroll_offset, 1);

        state.model_index = 9;
        state.ensure_model_visible(3);
        assert_eq!(state.model_scroll_offset, 7);
    }

    #[test]
    fn test_selection_index_from_point_respects_scroll_offset() {
        let mut state = AIWizardState::new();
        state.models = (0..8)
            .map(|i| ModelInfo {
                id: format!("model-{}", i),
                created: 0,
                owned_by: "test".to_string(),
            })
            .collect();
        state.model_scroll_offset = 3;
        state.list_inner_area = Some(Rect {
            x: 10,
            y: 5,
            width: 20,
            height: 3,
        });

        assert_eq!(state.selection_index_from_point(10, 5), Some(3));
        assert_eq!(state.selection_index_from_point(10, 6), Some(4));
        assert_eq!(state.selection_index_from_point(10, 7), Some(5));
        assert_eq!(state.selection_index_from_point(10, 8), None);
    }

    #[test]
    fn test_wizard_edit_mode() {
        let mut state = AIWizardState::new();
        state.open_edit(
            true,
            None,
            "http://localhost:11434/v1",
            "my-key",
            "llama3.2",
        );

        assert!(state.is_edit);
        assert!(state.is_default_ai);
        assert_eq!(state.endpoint, "http://localhost:11434/v1");
        assert_eq!(state.api_key, "my-key");
        assert_eq!(state.selected_model, "llama3.2");
    }

    #[test]
    fn test_wizard_target_description() {
        let mut state = AIWizardState::new();

        state.is_default_ai = true;
        assert_eq!(state.target_description(), "Default AI Settings");

        state.is_default_ai = false;
        state.profile_name = Some("dev".to_string());
        assert_eq!(state.target_description(), "Profile: dev");
    }
}
