use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::config::launch_defaults::LaunchDefaults;

// ---------------------------------------------------------------------------
// DialogField — which field is focused
// ---------------------------------------------------------------------------

/// Which field in the launch dialog is focused.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum DialogField {
    #[default]
    Agent,
    AgentVersion,
    Model,
    Branch,
    SessionMode,
    ResumeSessionId,
    SkipPermissions,
    FastMode,
    ReasoningLevel,
    ExtraArgs,
    LaunchButton,
    CancelButton,
}

// ---------------------------------------------------------------------------
// DialogSessionMode
// ---------------------------------------------------------------------------

/// Session launch mode.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum DialogSessionMode {
    #[default]
    Normal,
    Continue,
    Resume,
}

impl DialogSessionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Continue => "Continue",
            Self::Resume => "Resume",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Normal => Self::Continue,
            Self::Continue => Self::Resume,
            Self::Resume => Self::Normal,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Continue => "continue",
            Self::Resume => "resume",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "continue" => Self::Continue,
            "resume" => Self::Resume,
            _ => Self::Normal,
        }
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Agent IDs corresponding to `agent_options` indices.
const AGENT_IDS: &[&str] = &["claude", "codex", "gemini"];

/// Model options per agent (indexed by agent_options position).
fn model_options_for_agents() -> Vec<Vec<String>> {
    vec![
        // Claude Code
        vec![
            "opus".to_string(),
            "sonnet".to_string(),
            "haiku".to_string(),
        ],
        // Codex CLI
        vec!["o3".to_string(), "o4-mini".to_string()],
        // Gemini CLI
        vec![
            "gemini-2.5-pro".to_string(),
            "gemini-2.5-flash".to_string(),
        ],
    ]
}

fn default_version_options() -> Vec<String> {
    vec!["installed".to_string(), "latest".to_string()]
}

fn default_reasoning_options() -> Vec<String> {
    vec![
        "low".to_string(),
        "medium".to_string(),
        "high".to_string(),
        "xhigh".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// LaunchDialogState
// ---------------------------------------------------------------------------

/// State for the agent launch dialog.
#[derive(Debug)]
pub struct LaunchDialogState {
    // Agent selection
    pub agent_options: Vec<String>,
    pub selected_agent: usize,

    // Agent version
    pub version_options: Vec<String>,
    pub selected_version: usize,

    // Model (per-agent memory)
    pub model_options: Vec<Vec<String>>,
    pub selected_model: usize,
    pub model_by_agent: HashMap<String, String>,

    // Branch
    pub branch_input: String,

    // Session
    pub session_mode: DialogSessionMode,
    pub resume_session_id: String,

    // Permissions
    pub skip_permissions: bool,

    // Codex-specific
    pub fast_mode: bool,
    pub reasoning_level: usize,
    pub reasoning_options: Vec<String>,

    // Advanced
    pub extra_args: String,
    pub env_overrides: String,

    // UI state
    pub focused_field: DialogField,
}

impl Default for LaunchDialogState {
    fn default() -> Self {
        Self {
            agent_options: vec![
                "Claude Code".to_string(),
                "Codex CLI".to_string(),
                "Gemini CLI".to_string(),
            ],
            selected_agent: 0,
            version_options: default_version_options(),
            selected_version: 0,
            model_options: model_options_for_agents(),
            selected_model: 0,
            model_by_agent: HashMap::new(),
            branch_input: String::new(),
            session_mode: DialogSessionMode::Normal,
            resume_session_id: String::new(),
            skip_permissions: false,
            fast_mode: false,
            reasoning_level: 2, // "high"
            reasoning_options: default_reasoning_options(),
            extra_args: String::new(),
            env_overrides: String::new(),
            focused_field: DialogField::Agent,
        }
    }
}

impl LaunchDialogState {
    // -----------------------------------------------------------------------
    // Defaults persistence helpers
    // -----------------------------------------------------------------------

    /// Restore state from persisted [`LaunchDefaults`].
    pub fn apply_defaults(&mut self, defaults: &LaunchDefaults) {
        // Restore agent selection
        if let Some(idx) = AGENT_IDS.iter().position(|id| *id == defaults.selected_agent) {
            self.selected_agent = idx;
        }

        // Restore session mode
        self.session_mode = DialogSessionMode::from_str_lossy(&defaults.session_mode);

        // Restore model_by_agent map
        self.model_by_agent = defaults.model_by_agent.clone();

        // Restore model for currently selected agent
        let agent_id = self.current_agent_id().to_string();
        if let Some(model) = defaults.model_by_agent.get(&agent_id).cloned() {
            self.restore_model_for_agent(&model);
        }

        // Restore version per agent
        if let Some(ver) = defaults.version_by_agent.get(&agent_id).cloned() {
            if let Some(idx) = self.version_options.iter().position(|v| *v == ver) {
                self.selected_version = idx;
            }
        }

        self.skip_permissions = defaults.skip_permissions;
        self.fast_mode = defaults.fast_mode;
        self.extra_args = defaults.extra_args.clone();
        self.env_overrides = defaults.env_overrides.clone();

        // Restore reasoning level
        if let Some(idx) = self
            .reasoning_options
            .iter()
            .position(|o| *o == defaults.reasoning_level)
        {
            self.reasoning_level = idx;
        }
    }

    /// Export current state as [`LaunchDefaults`] for persistence.
    pub fn to_defaults(&self) -> LaunchDefaults {
        LaunchDefaults {
            selected_agent: self.current_agent_id().to_string(),
            session_mode: self.session_mode.as_str().to_string(),
            model_by_agent: self.model_by_agent.clone(),
            version_by_agent: {
                let mut m = HashMap::new();
                let agent_id = self.current_agent_id();
                if let Some(ver) = self.version_options.get(self.selected_version) {
                    m.insert(agent_id.to_string(), ver.clone());
                }
                m
            },
            skip_permissions: self.skip_permissions,
            reasoning_level: self
                .reasoning_options
                .get(self.reasoning_level)
                .cloned()
                .unwrap_or_default(),
            fast_mode: self.fast_mode,
            extra_args: self.extra_args.clone(),
            env_overrides: self.env_overrides.clone(),
        }
    }

    // -----------------------------------------------------------------------
    // Agent / model helpers
    // -----------------------------------------------------------------------

    /// Get the machine-readable agent ID for the currently selected agent.
    pub fn current_agent_id(&self) -> &str {
        AGENT_IDS
            .get(self.selected_agent)
            .copied()
            .unwrap_or("claude")
    }

    /// Whether the currently selected agent is Codex.
    pub fn is_codex(&self) -> bool {
        self.current_agent_id() == "codex"
    }

    /// Whether the session mode shows a resume-session-id field.
    pub fn shows_resume_session_id(&self) -> bool {
        matches!(
            self.session_mode,
            DialogSessionMode::Continue | DialogSessionMode::Resume
        )
    }

    // -----------------------------------------------------------------------
    // Field navigation
    // -----------------------------------------------------------------------

    /// Cycle focus to the next applicable dialog field (skipping inapplicable ones).
    pub fn focus_next(&mut self) {
        self.focused_field = self.next_field_after(&self.focused_field);
    }

    /// Cycle focus to the previous applicable dialog field.
    pub fn focus_prev(&mut self) {
        self.focused_field = self.prev_field_before(&self.focused_field);
    }

    fn next_field_after(&self, field: &DialogField) -> DialogField {
        let mut candidate = raw_next_field(field);
        // Skip at most a full cycle to avoid infinite loop
        for _ in 0..12 {
            if self.field_applicable(&candidate) {
                return candidate;
            }
            candidate = raw_next_field(&candidate);
        }
        candidate
    }

    fn prev_field_before(&self, field: &DialogField) -> DialogField {
        let mut candidate = raw_prev_field(field);
        for _ in 0..12 {
            if self.field_applicable(&candidate) {
                return candidate;
            }
            candidate = raw_prev_field(&candidate);
        }
        candidate
    }

    fn field_applicable(&self, field: &DialogField) -> bool {
        match field {
            DialogField::FastMode | DialogField::ReasoningLevel => self.is_codex(),
            DialogField::ResumeSessionId => self.shows_resume_session_id(),
            _ => true,
        }
    }

    // -----------------------------------------------------------------------
    // Selectors
    // -----------------------------------------------------------------------

    /// Cycle the selected agent option forward, saving/restoring per-agent model.
    pub fn next_agent(&mut self) {
        if self.agent_options.is_empty() {
            return;
        }
        // Save current model for current agent
        self.save_current_model();

        self.selected_agent = (self.selected_agent + 1) % self.agent_options.len();

        // Restore model for new agent
        let agent_id = self.current_agent_id().to_string();
        if let Some(model) = self.model_by_agent.get(&agent_id).cloned() {
            self.restore_model_for_agent(&model);
        } else {
            self.selected_model = 0;
        }
    }

    /// Cycle the selected model option forward.
    pub fn next_model(&mut self) {
        if let Some(models) = self.model_options.get(self.selected_agent) {
            if !models.is_empty() {
                self.selected_model = (self.selected_model + 1) % models.len();
                self.save_current_model();
            }
        }
    }

    /// Cycle the version selector forward.
    pub fn next_version(&mut self) {
        if !self.version_options.is_empty() {
            self.selected_version = (self.selected_version + 1) % self.version_options.len();
        }
    }

    /// Toggle session mode.
    pub fn next_session_mode(&mut self) {
        self.session_mode = self.session_mode.next();
    }

    /// Toggle skip permissions.
    pub fn toggle_skip_permissions(&mut self) {
        self.skip_permissions = !self.skip_permissions;
    }

    /// Toggle fast mode.
    pub fn toggle_fast_mode(&mut self) {
        self.fast_mode = !self.fast_mode;
    }

    /// Cycle reasoning level forward.
    pub fn next_reasoning_level(&mut self) {
        if !self.reasoning_options.is_empty() {
            self.reasoning_level = (self.reasoning_level + 1) % self.reasoning_options.len();
        }
    }

    // -----------------------------------------------------------------------
    // Labels
    // -----------------------------------------------------------------------

    /// Get the currently selected agent option label.
    pub fn selected_agent_label(&self) -> &str {
        self.agent_options
            .get(self.selected_agent)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Get the currently selected version label.
    pub fn selected_version_label(&self) -> &str {
        self.version_options
            .get(self.selected_version)
            .map(|s| s.as_str())
            .unwrap_or("installed")
    }

    /// Get the currently selected model label.
    pub fn selected_model_label(&self) -> &str {
        self.model_options
            .get(self.selected_agent)
            .and_then(|models| models.get(self.selected_model))
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Get the currently selected model name, or `None` if no models available.
    pub fn selected_model_name(&self) -> Option<&str> {
        self.model_options
            .get(self.selected_agent)
            .and_then(|models| models.get(self.selected_model))
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    /// Get the currently selected reasoning level label.
    pub fn selected_reasoning_label(&self) -> &str {
        self.reasoning_options
            .get(self.reasoning_level)
            .map(|s| s.as_str())
            .unwrap_or("high")
    }

    /// Get the version string for the builder (`None` for "installed").
    pub fn version_for_builder(&self) -> Option<&str> {
        let label = self.selected_version_label();
        if label == "installed" {
            None
        } else {
            Some(label)
        }
    }

    // -----------------------------------------------------------------------
    // Visible row count (for dynamic dialog height)
    // -----------------------------------------------------------------------

    /// Number of content rows the dialog needs (excluding border).
    pub fn visible_row_count(&self) -> u16 {
        let mut count: u16 = 8; // Agent + Version + Model + Branch + Session + Perms + spacer + buttons
        if self.is_codex() {
            count += 2; // FastMode + ReasoningLevel
        }
        if self.shows_resume_session_id() {
            count += 1; // ResumeSessionId
        }
        // ExtraArgs is always visible
        count += 1;
        count
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn save_current_model(&mut self) {
        if let Some(model) = self.selected_model_name() {
            let agent_id = self.current_agent_id().to_string();
            self.model_by_agent.insert(agent_id, model.to_string());
        }
    }

    fn restore_model_for_agent(&mut self, model_name: &str) {
        if let Some(models) = self.model_options.get(self.selected_agent) {
            if let Some(idx) = models.iter().position(|m| m == model_name) {
                self.selected_model = idx;
                return;
            }
        }
        self.selected_model = 0;
    }
}

// ---------------------------------------------------------------------------
// Raw field ordering (the full enum order without skipping)
// ---------------------------------------------------------------------------

fn raw_next_field(field: &DialogField) -> DialogField {
    match field {
        DialogField::Agent => DialogField::AgentVersion,
        DialogField::AgentVersion => DialogField::Model,
        DialogField::Model => DialogField::Branch,
        DialogField::Branch => DialogField::SessionMode,
        DialogField::SessionMode => DialogField::ResumeSessionId,
        DialogField::ResumeSessionId => DialogField::SkipPermissions,
        DialogField::SkipPermissions => DialogField::FastMode,
        DialogField::FastMode => DialogField::ReasoningLevel,
        DialogField::ReasoningLevel => DialogField::ExtraArgs,
        DialogField::ExtraArgs => DialogField::LaunchButton,
        DialogField::LaunchButton => DialogField::CancelButton,
        DialogField::CancelButton => DialogField::Agent,
    }
}

fn raw_prev_field(field: &DialogField) -> DialogField {
    match field {
        DialogField::Agent => DialogField::CancelButton,
        DialogField::AgentVersion => DialogField::Agent,
        DialogField::Model => DialogField::AgentVersion,
        DialogField::Branch => DialogField::Model,
        DialogField::SessionMode => DialogField::Branch,
        DialogField::ResumeSessionId => DialogField::SessionMode,
        DialogField::SkipPermissions => DialogField::ResumeSessionId,
        DialogField::FastMode => DialogField::SkipPermissions,
        DialogField::ReasoningLevel => DialogField::FastMode,
        DialogField::ExtraArgs => DialogField::ReasoningLevel,
        DialogField::LaunchButton => DialogField::ExtraArgs,
        DialogField::CancelButton => DialogField::LaunchButton,
    }
}

// ---------------------------------------------------------------------------
// Styling helpers
// ---------------------------------------------------------------------------

fn field_style(focused: bool) -> Style {
    if focused {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    }
}

fn button_style(focused: bool, color: Color) -> Style {
    if focused {
        Style::new()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(color)
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the launch dialog.
///
/// The caller is responsible for centering the `area` if needed.
pub fn render(buf: &mut Buffer, area: Rect, state: &LaunchDialogState) {
    Clear.render(area, buf);

    let block = Block::default()
        .title(" Launch Agent ")
        .borders(Borders::ALL)
        .style(Style::new().bg(Color::Black));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height < 7 || inner.width < 30 {
        return;
    }

    // Build dynamic row constraints
    let mut constraints = vec![
        Constraint::Length(1), // Agent
        Constraint::Length(1), // Version
        Constraint::Length(1), // Model
        Constraint::Length(1), // Branch
        Constraint::Length(1), // Session
    ];
    if state.shows_resume_session_id() {
        constraints.push(Constraint::Length(1)); // ResumeSessionId
    }
    constraints.push(Constraint::Length(1)); // Perms
    if state.is_codex() {
        constraints.push(Constraint::Length(1)); // FastMode
        constraints.push(Constraint::Length(1)); // ReasoningLevel
    }
    constraints.push(Constraint::Length(1)); // ExtraArgs
    constraints.push(Constraint::Length(1)); // Spacer
    constraints.push(Constraint::Length(1)); // Buttons

    let rows = Layout::vertical(constraints).split(inner);

    let label_w = 10;
    let mut row_idx = 0;

    // Agent selector
    render_selector_row(
        buf,
        rows[row_idx],
        label_w,
        "Agent:",
        state.selected_agent_label(),
        state.focused_field == DialogField::Agent,
    );
    row_idx += 1;

    // Version selector
    render_selector_row(
        buf,
        rows[row_idx],
        label_w,
        "Version:",
        state.selected_version_label(),
        state.focused_field == DialogField::AgentVersion,
    );
    row_idx += 1;

    // Model selector
    render_selector_row(
        buf,
        rows[row_idx],
        label_w,
        "Model:",
        state.selected_model_label(),
        state.focused_field == DialogField::Model,
    );
    row_idx += 1;

    // Branch input
    let branch_display = if state.branch_input.is_empty() {
        "<branch name>"
    } else {
        &state.branch_input
    };
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{:<label_w$}", "Branch:"),
            Style::new().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("[{}]", branch_display),
            field_style(state.focused_field == DialogField::Branch),
        ),
    ]))
    .render(rows[row_idx], buf);
    row_idx += 1;

    // Session mode
    render_selector_row(
        buf,
        rows[row_idx],
        label_w,
        "Session:",
        state.session_mode.label(),
        state.focused_field == DialogField::SessionMode,
    );
    row_idx += 1;

    // ResumeSessionId (conditional)
    if state.shows_resume_session_id() {
        let id_display = if state.resume_session_id.is_empty() {
            "<session id>"
        } else {
            &state.resume_session_id
        };
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{:<label_w$}", "SessID:"),
                Style::new().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("[{}]", id_display),
                field_style(state.focused_field == DialogField::ResumeSessionId),
            ),
        ]))
        .render(rows[row_idx], buf);
        row_idx += 1;
    }

    // Skip permissions toggle
    let check = if state.skip_permissions { "x" } else { " " };
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{:<label_w$}", "Perms:"),
            Style::new().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("[{check}] Skip Permissions"),
            field_style(state.focused_field == DialogField::SkipPermissions),
        ),
    ]))
    .render(rows[row_idx], buf);
    row_idx += 1;

    // Codex-specific: FastMode
    if state.is_codex() {
        let fast_check = if state.fast_mode { "x" } else { " " };
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{:<label_w$}", "Fast:"),
                Style::new().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("[{fast_check}] Fast Mode"),
                field_style(state.focused_field == DialogField::FastMode),
            ),
        ]))
        .render(rows[row_idx], buf);
        row_idx += 1;

        // ReasoningLevel
        render_selector_row(
            buf,
            rows[row_idx],
            label_w,
            "Reason:",
            state.selected_reasoning_label(),
            state.focused_field == DialogField::ReasoningLevel,
        );
        row_idx += 1;
    }

    // ExtraArgs
    let args_display = if state.extra_args.is_empty() {
        "<extra args>"
    } else {
        &state.extra_args
    };
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{:<label_w$}", "Args:"),
            Style::new().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("[{}]", args_display),
            field_style(state.focused_field == DialogField::ExtraArgs),
        ),
    ]))
    .render(rows[row_idx], buf);
    row_idx += 1;

    // Spacer
    row_idx += 1;

    // Buttons
    if row_idx < rows.len() {
        Paragraph::new(Line::from(vec![
            Span::raw("       "),
            Span::styled(
                " Launch ",
                button_style(
                    state.focused_field == DialogField::LaunchButton,
                    Color::Green,
                ),
            ),
            Span::raw("  "),
            Span::styled(
                " Cancel ",
                button_style(
                    state.focused_field == DialogField::CancelButton,
                    Color::Red,
                ),
            ),
        ]))
        .render(rows[row_idx], buf);
    }
}

/// Helper to render a selector row: `Label:  [value ▼]`
fn render_selector_row(
    buf: &mut Buffer,
    area: Rect,
    label_w: usize,
    label: &str,
    value: &str,
    focused: bool,
) {
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{:<label_w$}", label),
            Style::new().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("[{} \u{25bc}]", value),
            field_style(focused),
        ),
    ]))
    .render(area, buf);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dialog_field_cycling_claude() {
        let mut state = LaunchDialogState::default();
        // Claude: no FastMode/ReasoningLevel, no ResumeSessionId
        assert_eq!(state.focused_field, DialogField::Agent);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::AgentVersion);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::Model);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::Branch);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::SessionMode);
        state.focus_next();
        // ResumeSessionId is skipped (Normal mode)
        assert_eq!(state.focused_field, DialogField::SkipPermissions);
        state.focus_next();
        // FastMode/ReasoningLevel skipped (not Codex)
        assert_eq!(state.focused_field, DialogField::ExtraArgs);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::LaunchButton);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::CancelButton);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::Agent);
    }

    #[test]
    fn test_dialog_field_cycling_codex() {
        let mut state = LaunchDialogState::default();
        state.selected_agent = 1; // Codex

        state.focused_field = DialogField::SkipPermissions;
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::FastMode);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::ReasoningLevel);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::ExtraArgs);
    }

    #[test]
    fn test_dialog_field_cycling_resume_session() {
        let mut state = LaunchDialogState::default();
        state.session_mode = DialogSessionMode::Resume;

        state.focused_field = DialogField::SessionMode;
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::ResumeSessionId);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::SkipPermissions);
    }

    #[test]
    fn test_focus_prev() {
        let mut state = LaunchDialogState::default();
        state.focused_field = DialogField::Model;
        state.focus_prev();
        assert_eq!(state.focused_field, DialogField::AgentVersion);
        state.focus_prev();
        assert_eq!(state.focused_field, DialogField::Agent);
        state.focus_prev();
        assert_eq!(state.focused_field, DialogField::CancelButton);
    }

    #[test]
    fn test_dialog_render_centered() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        let state = LaunchDialogState::default();
        render(&mut buf, Rect::new(0, 0, 100, 30), &state);
        let all_content: String = (0..30)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_content.contains("Launch Agent"));
    }

    #[test]
    fn test_launch_dialog_default_state() {
        let state = LaunchDialogState::default();
        assert_eq!(state.agent_options.len(), 3);
        assert_eq!(state.selected_agent, 0);
        assert_eq!(state.selected_model, 0);
        assert_eq!(state.selected_version, 0);
        assert!(state.branch_input.is_empty());
        assert_eq!(state.session_mode, DialogSessionMode::Normal);
        assert!(!state.skip_permissions);
        assert!(!state.fast_mode);
        assert_eq!(state.reasoning_level, 2); // "high"
        assert_eq!(state.focused_field, DialogField::Agent);
        assert_eq!(state.selected_agent_label(), "Claude Code");
        assert_eq!(state.selected_model_label(), "opus");
        assert_eq!(state.selected_version_label(), "installed");
    }

    #[test]
    fn test_next_agent_cycles_with_model_memory() {
        let mut state = LaunchDialogState::default();
        // Claude: select "sonnet"
        state.next_model(); // opus -> sonnet
        assert_eq!(state.selected_model_label(), "sonnet");

        // Switch to Codex
        state.next_agent();
        assert_eq!(state.selected_agent_label(), "Codex CLI");
        assert_eq!(state.selected_model_label(), "o3");

        // Switch to Gemini
        state.next_agent();
        assert_eq!(state.selected_agent_label(), "Gemini CLI");

        // Switch back to Claude — model should be restored to "sonnet"
        state.next_agent();
        assert_eq!(state.selected_agent_label(), "Claude Code");
        assert_eq!(state.selected_model_label(), "sonnet");
    }

    #[test]
    fn test_next_model_cycles() {
        let mut state = LaunchDialogState::default();
        assert_eq!(state.selected_model_label(), "opus");
        state.next_model();
        assert_eq!(state.selected_model_label(), "sonnet");
        state.next_model();
        assert_eq!(state.selected_model_label(), "haiku");
        state.next_model();
        assert_eq!(state.selected_model_label(), "opus");
    }

    #[test]
    fn test_version_cycling() {
        let mut state = LaunchDialogState::default();
        assert_eq!(state.selected_version_label(), "installed");
        state.next_version();
        assert_eq!(state.selected_version_label(), "latest");
        state.next_version();
        assert_eq!(state.selected_version_label(), "installed");
    }

    #[test]
    fn test_agent_change_resets_model() {
        let mut state = LaunchDialogState::default();
        state.next_model();
        assert_eq!(state.selected_model_label(), "sonnet");
        state.next_agent();
        // First time visiting Codex — default to 0
        assert_eq!(state.selected_model_label(), "o3");
    }

    #[test]
    fn test_selected_model_name() {
        let state = LaunchDialogState::default();
        assert_eq!(state.selected_model_name(), Some("opus"));
    }

    #[test]
    fn test_model_options_per_agent() {
        let mut state = LaunchDialogState::default();
        assert_eq!(state.model_options[0].len(), 3);
        state.next_agent();
        assert_eq!(state.selected_model_label(), "o3");
        state.next_model();
        assert_eq!(state.selected_model_label(), "o4-mini");
        state.next_agent();
        assert_eq!(state.selected_model_label(), "gemini-2.5-pro");
        state.next_model();
        assert_eq!(state.selected_model_label(), "gemini-2.5-flash");
    }

    #[test]
    fn test_session_mode_cycling() {
        let mut state = LaunchDialogState::default();
        assert_eq!(state.session_mode, DialogSessionMode::Normal);
        state.next_session_mode();
        assert_eq!(state.session_mode, DialogSessionMode::Continue);
        state.next_session_mode();
        assert_eq!(state.session_mode, DialogSessionMode::Resume);
        state.next_session_mode();
        assert_eq!(state.session_mode, DialogSessionMode::Normal);
    }

    #[test]
    fn test_skip_permissions_toggle() {
        let mut state = LaunchDialogState::default();
        assert!(!state.skip_permissions);
        state.toggle_skip_permissions();
        assert!(state.skip_permissions);
        state.toggle_skip_permissions();
        assert!(!state.skip_permissions);
    }

    #[test]
    fn test_fast_mode_toggle() {
        let mut state = LaunchDialogState::default();
        assert!(!state.fast_mode);
        state.toggle_fast_mode();
        assert!(state.fast_mode);
        state.toggle_fast_mode();
        assert!(!state.fast_mode);
    }

    #[test]
    fn test_reasoning_level_cycling() {
        let mut state = LaunchDialogState::default();
        assert_eq!(state.selected_reasoning_label(), "high");
        state.next_reasoning_level();
        assert_eq!(state.selected_reasoning_label(), "xhigh");
        state.next_reasoning_level();
        assert_eq!(state.selected_reasoning_label(), "low");
    }

    #[test]
    fn test_is_codex() {
        let mut state = LaunchDialogState::default();
        assert!(!state.is_codex());
        state.selected_agent = 1;
        assert!(state.is_codex());
    }

    #[test]
    fn test_visible_row_count_claude() {
        let state = LaunchDialogState::default();
        // Agent + Version + Model + Branch + Session + Perms + spacer + buttons + ExtraArgs = 9
        assert_eq!(state.visible_row_count(), 9);
    }

    #[test]
    fn test_visible_row_count_codex() {
        let mut state = LaunchDialogState::default();
        state.selected_agent = 1; // Codex
        // 9 + FastMode + ReasoningLevel = 11
        assert_eq!(state.visible_row_count(), 11);
    }

    #[test]
    fn test_visible_row_count_resume() {
        let mut state = LaunchDialogState::default();
        state.session_mode = DialogSessionMode::Resume;
        // 9 + ResumeSessionId = 10
        assert_eq!(state.visible_row_count(), 10);
    }

    #[test]
    fn test_defaults_roundtrip() {
        let mut state = LaunchDialogState::default();
        state.next_model(); // sonnet
        state.skip_permissions = true;
        state.fast_mode = true;
        state.extra_args = "--verbose".to_string();

        let defaults = state.to_defaults();
        assert_eq!(defaults.selected_agent, "claude");
        assert!(defaults.skip_permissions);
        assert!(defaults.fast_mode);

        let mut state2 = LaunchDialogState::default();
        state2.apply_defaults(&defaults);
        assert_eq!(state2.selected_model_label(), "sonnet");
        assert!(state2.skip_permissions);
        assert!(state2.fast_mode);
        assert_eq!(state2.extra_args, "--verbose");
    }

    #[test]
    fn test_version_for_builder() {
        let mut state = LaunchDialogState::default();
        assert_eq!(state.version_for_builder(), None);
        state.next_version(); // latest
        assert_eq!(state.version_for_builder(), Some("latest"));
    }
}
