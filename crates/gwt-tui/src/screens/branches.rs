//! Branches management screen.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

/// Sort mode for the branch list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Default,
    Name,
    Date,
}

impl SortMode {
    /// Cycle to the next sort mode.
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::Name,
            Self::Name => Self::Date,
            Self::Date => Self::Default,
        }
    }

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Name => "Name",
            Self::Date => "Date",
        }
    }
}

/// View mode filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    All,
    Local,
    Remote,
}

impl ViewMode {
    /// Cycle to the next view mode.
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Local,
            Self::Local => Self::Remote,
            Self::Remote => Self::All,
        }
    }

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Local => "Local",
            Self::Remote => "Remote",
        }
    }
}

/// Branch category derived from name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BranchCategory {
    Main,
    Develop,
    Feature,
    Other,
}

impl BranchCategory {
    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Main => "Main",
            Self::Develop => "Develop",
            Self::Feature => "Feature",
            Self::Other => "Other",
        }
    }
}

/// Categorize a branch by its name.
pub fn categorize_branch(name: &str) -> BranchCategory {
    let base = name.strip_prefix("origin/").unwrap_or(name);
    if base == "main" || base == "master" {
        BranchCategory::Main
    } else if base == "develop" || base == "development" {
        BranchCategory::Develop
    } else if base.starts_with("feature/") || base.starts_with("feat/") {
        BranchCategory::Feature
    } else {
        BranchCategory::Other
    }
}

/// A single branch entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchItem {
    pub name: String,
    pub is_head: bool,
    pub is_local: bool,
    pub category: BranchCategory,
    pub worktree_path: Option<std::path::PathBuf>,
}

/// A SPEC entry loaded from a branch worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailSpecItem {
    pub id: String,
    pub title: String,
    pub phase: String,
    pub status: String,
}

/// Lifecycle action requested for a Docker container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerLifecycleAction {
    Start,
    Stop,
    Restart,
}

/// Pending Docker action selected in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDockerAction {
    pub container_id: String,
    pub action: DockerLifecycleAction,
}

/// Number of detail sections in the branch detail view.
const DETAIL_SECTION_COUNT: usize = 4;

/// Labels for the detail sections.
const DETAIL_SECTION_LABELS: [&str; DETAIL_SECTION_COUNT] =
    ["Overview", "SPECs", "Git", "Sessions"];

/// Public accessor for detail section labels (used by app.rs for pane titles).
pub fn detail_section_labels() -> &'static [&'static str] {
    &DETAIL_SECTION_LABELS
}

/// Action labels shown in the action modal overlay.
const ACTION_LABELS: [&str; 3] = ["Launch Agent", "Open Shell", "Delete Worktree"];

/// State for the branches screen.
#[derive(Debug, Clone, Default)]
pub struct BranchesState {
    pub(crate) branches: Vec<BranchItem>,
    pub(crate) selected: usize,
    pub(crate) sort_mode: SortMode,
    pub(crate) view_mode: ViewMode,
    pub(crate) search_query: String,
    pub(crate) search_active: bool,
    pub(crate) detail_view: bool,
    /// Active detail section: 0=Overview, 1=SPECs, 2=Git, 3=Sessions.
    pub(crate) detail_section: usize,
    /// Whether the action modal overlay is visible.
    pub(crate) action_modal_visible: bool,
    /// Selected action index within the action modal.
    pub(crate) action_modal_selected: usize,
    /// Flag: caller should open agent selection.
    pub(crate) pending_launch_agent: bool,
    /// Flag: caller should spawn shell in worktree cwd.
    pub(crate) pending_open_shell: bool,
    /// Flag: caller should show worktree delete confirmation.
    pub(crate) pending_delete_worktree: bool,
    /// SPECs loaded from the selected branch worktree.
    pub(crate) detail_specs: Vec<DetailSpecItem>,
    /// Git status files for the selected branch worktree.
    pub(crate) detail_files: Vec<String>,
    /// Recent commits for the selected branch worktree.
    pub(crate) detail_commits: Vec<String>,
    /// Docker containers available for the selected branch context.
    pub(crate) docker_containers: Vec<gwt_docker::ContainerInfo>,
    /// Selected Docker container index in the overview area.
    pub(crate) docker_selected: usize,
    /// Pending Docker action intent to be handled by the caller.
    pub(crate) pending_docker_action: Option<PendingDockerAction>,
}

impl BranchesState {
    /// Return branches filtered by current view mode and search query,
    /// then sorted according to the active `sort_mode`.
    pub fn filtered_branches(&self) -> Vec<&BranchItem> {
        let query_lower = self.search_query.to_lowercase();
        let mut result: Vec<&BranchItem> = self
            .branches
            .iter()
            .filter(|b| match self.view_mode {
                ViewMode::All => true,
                ViewMode::Local => b.is_local,
                ViewMode::Remote => !b.is_local,
            })
            .filter(|b| query_lower.is_empty() || b.name.to_lowercase().contains(&query_lower))
            .collect();

        match self.sort_mode {
            SortMode::Default => {} // insertion order
            // Date has no dedicated field yet; fall back to alphabetical like Name.
            SortMode::Name | SortMode::Date => result.sort_by(|a, b| a.name.cmp(&b.name)),
        }

        result
    }

    /// Get the currently selected branch (from filtered list).
    pub fn selected_branch(&self) -> Option<&BranchItem> {
        let filtered = self.filtered_branches();
        filtered.get(self.selected).copied()
    }

    /// Clamp selected index to filtered length.
    fn clamp_selected(&mut self) {
        let len = self.filtered_branches().len();
        super::clamp_index(&mut self.selected, len);
    }

    /// Clamp selected Docker container index to available containers.
    fn clamp_docker_selected(&mut self) {
        let len = self.docker_containers.len();
        super::clamp_index(&mut self.docker_selected, len);
    }

    /// Return the currently selected Docker container, if any.
    fn selected_docker_container(&self) -> Option<&gwt_docker::ContainerInfo> {
        self.docker_containers.get(self.docker_selected)
    }
}

/// Messages specific to the branches screen.
#[derive(Debug, Clone)]
pub enum BranchesMessage {
    MoveUp,
    MoveDown,
    Select,
    ToggleSort,
    ToggleView,
    SearchStart,
    SearchInput(char),
    SearchBackspace,
    SearchClear,
    Refresh,
    SetBranches(Vec<BranchItem>),
    /// Cycle to the next detail section.
    NextDetailSection,
    /// Cycle to the previous detail section.
    PrevDetailSection,
    /// Open the action modal overlay.
    OpenActionModal,
    /// Close the action modal overlay.
    CloseActionModal,
    /// Move up within the action modal.
    ActionModalUp,
    /// Move down within the action modal.
    ActionModalDown,
    /// Select the current action in the modal.
    ActionModalSelect,
    /// Launch agent action.
    LaunchAgent,
    /// Open shell action.
    OpenShell,
    /// Delete worktree action.
    DeleteWorktree,
    /// Move to the next Docker container in the overview area.
    DockerContainerDown,
    /// Move to the previous Docker container in the overview area.
    DockerContainerUp,
    /// Request a start lifecycle action for the selected Docker container.
    DockerContainerStart,
    /// Request a stop lifecycle action for the selected Docker container.
    DockerContainerStop,
    /// Request a restart lifecycle action for the selected Docker container.
    DockerContainerRestart,
}

/// Update branches state in response to a message.
pub fn update(state: &mut BranchesState, msg: BranchesMessage) {
    match msg {
        BranchesMessage::MoveUp => {
            let len = state.filtered_branches().len();
            super::move_up(&mut state.selected, len);
        }
        BranchesMessage::MoveDown => {
            let len = state.filtered_branches().len();
            super::move_down(&mut state.selected, len);
        }
        BranchesMessage::Select => {
            if !state.filtered_branches().is_empty() {
                state.action_modal_visible = true;
                state.action_modal_selected = 0;
            }
        }
        BranchesMessage::ToggleSort => {
            state.sort_mode = state.sort_mode.next();
        }
        BranchesMessage::ToggleView => {
            state.view_mode = state.view_mode.next();
            state.clamp_selected();
        }
        BranchesMessage::SearchStart => {
            state.search_active = true;
        }
        BranchesMessage::SearchInput(ch) => {
            if state.search_active {
                state.search_query.push(ch);
                state.clamp_selected();
            }
        }
        BranchesMessage::SearchBackspace => {
            if state.search_active {
                state.search_query.pop();
                state.clamp_selected();
            }
        }
        BranchesMessage::SearchClear => {
            state.search_query.clear();
            state.search_active = false;
            state.clamp_selected();
        }
        BranchesMessage::Refresh => {
            // Signal to reload branches — handled by caller
        }
        BranchesMessage::SetBranches(branches) => {
            state.branches = branches;
            state.clamp_selected();
        }
        BranchesMessage::NextDetailSection => {
            state.detail_section = (state.detail_section + 1) % DETAIL_SECTION_COUNT;
        }
        BranchesMessage::PrevDetailSection => {
            state.detail_section = if state.detail_section == 0 {
                DETAIL_SECTION_COUNT - 1
            } else {
                state.detail_section - 1
            };
        }
        BranchesMessage::OpenActionModal => {
            if !state.filtered_branches().is_empty() {
                state.action_modal_visible = true;
                state.action_modal_selected = 0;
            }
        }
        BranchesMessage::CloseActionModal => {
            state.action_modal_visible = false;
        }
        BranchesMessage::ActionModalUp => {
            super::move_up(&mut state.action_modal_selected, ACTION_LABELS.len());
        }
        BranchesMessage::ActionModalDown => {
            super::move_down(&mut state.action_modal_selected, ACTION_LABELS.len());
        }
        BranchesMessage::ActionModalSelect => {
            let selected = state.action_modal_selected;
            state.action_modal_visible = false;
            match selected {
                0 => state.pending_launch_agent = true,
                1 => state.pending_open_shell = true,
                _ => state.pending_delete_worktree = true,
            }
        }
        BranchesMessage::LaunchAgent => {
            state.pending_launch_agent = true;
        }
        BranchesMessage::OpenShell => {
            state.pending_open_shell = true;
        }
        BranchesMessage::DeleteWorktree => {
            state.pending_delete_worktree = true;
        }
        BranchesMessage::DockerContainerDown => {
            if !state.docker_containers.is_empty() {
                super::move_down(&mut state.docker_selected, state.docker_containers.len());
            }
        }
        BranchesMessage::DockerContainerUp => {
            if !state.docker_containers.is_empty() {
                super::move_up(&mut state.docker_selected, state.docker_containers.len());
            }
        }
        BranchesMessage::DockerContainerStart => {
            if let Some(container) = state.selected_docker_container() {
                state.pending_docker_action = Some(PendingDockerAction {
                    container_id: container.id.clone(),
                    action: DockerLifecycleAction::Start,
                });
            }
        }
        BranchesMessage::DockerContainerStop => {
            if let Some(container) = state.selected_docker_container() {
                state.pending_docker_action = Some(PendingDockerAction {
                    container_id: container.id.clone(),
                    action: DockerLifecycleAction::Stop,
                });
            }
        }
        BranchesMessage::DockerContainerRestart => {
            if let Some(container) = state.selected_docker_container() {
                state.pending_docker_action = Some(PendingDockerAction {
                    container_id: container.id.clone(),
                    action: DockerLifecycleAction::Restart,
                });
            }
        }
    }
}

/// Load detail data (SPECs, git status, commits) for the selected branch.
///
/// Best-effort: all errors are silently ignored.
pub fn load_branch_detail(state: &mut BranchesState, _repo_path: &std::path::Path) {
    state.detail_specs.clear();
    state.detail_files.clear();
    state.detail_commits.clear();
    state.docker_containers.clear();
    state.docker_selected = 0;
    state.pending_docker_action = None;

    if let Ok(containers) = gwt_docker::list_containers() {
        state.docker_containers = containers;
        state.clamp_docker_selected();
    }

    let worktree_path = state
        .selected_branch()
        .and_then(|b| b.worktree_path.clone());

    let Some(wt_path) = worktree_path else {
        return;
    };

    // Load SPECs from worktree specs/ directory
    if let Ok(entries) = std::fs::read_dir(wt_path.join("specs")) {
        let mut specs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !dir_name.starts_with("SPEC-") {
                continue;
            }
            let metadata_path = path.join("metadata.json");
            let Ok(content) = std::fs::read_to_string(&metadata_path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
                continue;
            };
            let id = value
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or(dir_name)
                .to_string();
            let title = value
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let phase = value
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let status = value
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            specs.push(DetailSpecItem {
                id,
                title,
                phase,
                status,
            });
        }
        specs.sort_by(|a, b| spec_sort_key(&a.id).cmp(&spec_sort_key(&b.id)));
        state.detail_specs = specs;
    }

    // Load git status
    if let Ok(entries) = gwt_git::diff::get_status(&wt_path) {
        state.detail_files = entries
            .iter()
            .map(|e| {
                let tag = match e.status {
                    gwt_git::FileStatus::Staged => "[S]",
                    gwt_git::FileStatus::Unstaged => "[U]",
                    gwt_git::FileStatus::Untracked => "[?]",
                };
                format!("{} {}", tag, e.path.display())
            })
            .collect();
    }

    // Load recent commits
    if let Ok(commits) = gwt_git::commit::recent_commits(&wt_path, 5) {
        state.detail_commits = commits
            .iter()
            .map(|c| format!("{} {}", c.hash, c.subject))
            .collect();
    }
}

fn spec_sort_key(spec_id: &str) -> (u64, String) {
    let numeric = spec_id
        .strip_prefix("SPEC-")
        .and_then(|suffix| suffix.parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    (numeric, spec_id.to_string())
}

/// Which sub-pane of the branches screen is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchesFocus {
    List,
    Detail,
    None,
}

/// Render the branch list pane content (header + list, no borders).
///
/// Called by app.rs which provides the bordered pane. This renders borderless
/// content into the inner area of the top management pane.
pub fn render_list(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let list_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header: view/sort/search (no border)
            Constraint::Min(0),    // Branch list
        ])
        .split(area);

    render_header(state, frame, list_chunks[0]);
    render_branch_list(state, frame, list_chunks[1]);
}

/// Render the branch detail content (no borders, no title).
///
/// Called by app.rs which provides the bordered pane. This renders borderless
/// content into the inner area of the bottom detail pane.
/// `session_count` is the number of active sessions matching this branch.
pub fn render_detail_content(
    state: &BranchesState,
    frame: &mut Frame,
    area: Rect,
    session_count: usize,
) {
    match state.detail_section {
        0 => render_detail_overview(state, frame, area),
        1 => render_detail_specs(state, frame, area),
        2 => render_detail_git_status(state, frame, area),
        3 => render_detail_sessions(frame, area, session_count),
        _ => {}
    }
}

/// Render the action modal overlay on top of a given area.
///
/// Called by app.rs when the action modal is visible.
pub fn render_action_modal_overlay(state: &BranchesState, frame: &mut Frame, area: Rect) {
    render_action_modal(state, frame, area);
}

/// Render the branches screen (legacy entry point).
///
/// In the lazygit layout, app.rs calls render_list / render_detail_content directly.
pub fn render(state: &BranchesState, frame: &mut Frame, area: Rect, _focus: BranchesFocus) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let list_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(main_chunks[0]);

    render_header(state, frame, list_chunks[0]);
    render_branch_list(state, frame, list_chunks[1]);
    render_detail_content(state, frame, main_chunks[1], 0);

    if state.action_modal_visible {
        render_action_modal(state, frame, main_chunks[1]);
    }
}

/// Render the header bar with view mode, sort mode, and search (plain bar, no borders).
fn render_header(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let search_display = if state.search_active {
        format!("  Search: {}_", state.search_query)
    } else if !state.search_query.is_empty() {
        format!("  Search: {}", state.search_query)
    } else {
        String::new()
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" View: {} ", state.view_mode.label()),
            Style::default().fg(Color::White),
        ),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(" Sort: {} ", state.sort_mode.label()),
            Style::default().fg(Color::White),
        ),
        Span::styled(search_display, Style::default().fg(Color::Yellow)),
    ]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

/// Render the branch list grouped by category (borderless).
fn render_branch_list(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_branches();

    if filtered.is_empty() {
        let msg = if !state.branches.is_empty() {
            "No matching branches"
        } else {
            "No branches loaded"
        };
        let p = Paragraph::new(msg)
            .block(Block::default())
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, area);
        return;
    }

    // Build items with category headers, tracking visual index offset
    let mut items: Vec<ListItem> = Vec::new();
    let mut current_category: Option<BranchCategory> = None;
    let mut headers_before_selected: usize = 0;

    for (idx, branch) in filtered.iter().enumerate() {
        if current_category != Some(branch.category) {
            current_category = Some(branch.category);
            if idx <= state.selected {
                headers_before_selected += 1;
            }
            let header = Line::from(Span::styled(
                format!("── {} ──", branch.category.label()),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            items.push(ListItem::new(header));
        }

        let head_indicator = if branch.is_head { "* " } else { "  " };
        let locality = if branch.is_local { "L" } else { "R" };

        let line = Line::from(vec![
            Span::styled(head_indicator, Style::default().fg(Color::Green)),
            Span::styled(format!("[{}] ", locality), Style::default().fg(Color::Cyan)),
            Span::styled(&branch.name, Style::default().fg(Color::White)),
        ]);
        items.push(ListItem::new(line));
    }

    // Visual index = data index + number of headers inserted before it
    let visual_selected = state.selected + headers_before_selected;

    let list = List::new(items).block(Block::default()).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(visual_selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Overview section: branch name, HEAD indicator, category, worktree path.
fn render_detail_overview(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let mut lines = Vec::new();

    match state.selected_branch() {
        Some(branch) => {
            let head = if branch.is_head { " (HEAD)" } else { "" };
            let locality = if branch.is_local { "Local" } else { "Remote" };
            lines.push(format!(" Branch: {}{}", branch.name, head));
            lines.push(format!(" Category: {}", branch.category.label()));
            lines.push(format!(" Type: {}", locality));
            if let Some(worktree) = branch.worktree_path.as_ref() {
                lines.push(format!(" Worktree: {}", worktree.display()));
            }
        }
        None => lines.push(" No branch selected".to_string()),
    }

    lines.push(String::new());
    lines.push(" Docker status".to_string());
    if state.docker_containers.is_empty() {
        lines.push(" No containers found".to_string());
    } else if let Some(container) = state.selected_docker_container() {
        lines.push(format!(" Selected: {}", container.name));
        lines.push(format!(
            " Status: {}",
            docker_status_label(container.status)
        ));
        lines.push(format!(" Ports: {}", docker_ports_label(&container.ports)));
        lines.push(format!(
            " Controls: {}",
            docker_controls_hint(container.status)
        ));
    }

    let block = Block::default().title("Overview");
    let paragraph = Paragraph::new(lines.join("\n"))
        .block(block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, area);
}

fn docker_status_label(status: gwt_docker::ContainerStatus) -> &'static str {
    match status {
        gwt_docker::ContainerStatus::Created => "Created",
        gwt_docker::ContainerStatus::Running => "Running",
        gwt_docker::ContainerStatus::Paused => "Paused",
        gwt_docker::ContainerStatus::Stopped => "Stopped",
        gwt_docker::ContainerStatus::Exited => "Exited",
    }
}

fn docker_ports_label(ports: &str) -> &str {
    if ports.is_empty() {
        "No published ports"
    } else {
        ports
    }
}

fn docker_controls_hint(status: gwt_docker::ContainerStatus) -> &'static str {
    match status {
        gwt_docker::ContainerStatus::Running => "Up/Down select  T stop  R restart",
        gwt_docker::ContainerStatus::Paused => "Up/Down select  S start  T stop  R restart",
        gwt_docker::ContainerStatus::Created
        | gwt_docker::ContainerStatus::Stopped
        | gwt_docker::ContainerStatus::Exited => "Up/Down select  S start  R restart",
    }
}

/// SPECs section: list loaded from the worktree.
fn render_detail_specs(state: &BranchesState, frame: &mut Frame, area: Rect) {
    if state.selected_branch().is_none() {
        let block = Block::default().title("SPECs");
        let paragraph = Paragraph::new(" No branch selected")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    if state.detail_specs.is_empty() {
        let has_worktree = state
            .selected_branch()
            .and_then(|b| b.worktree_path.as_ref())
            .is_some();
        let msg = if has_worktree {
            " No SPECs found in worktree"
        } else {
            " No worktree (no SPECs available)"
        };
        let block = Block::default().title("SPECs");
        let paragraph = Paragraph::new(msg)
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = state
        .detail_specs
        .iter()
        .map(|spec| {
            let line = Line::from(vec![
                Span::styled(
                    format!(" {} ", spec.id),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(&spec.title, Style::default().fg(Color::White)),
                Span::styled(
                    format!("  [{}]", spec.status),
                    Style::default().fg(Color::Yellow),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default().title(format!("SPECs ({})", state.detail_specs.len()));
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

/// Git Status section: files and recent commits from the worktree.
fn render_detail_git_status(state: &BranchesState, frame: &mut Frame, area: Rect) {
    if state.selected_branch().is_none() {
        let block = Block::default().title("Git Status");
        let paragraph = Paragraph::new(" No branch selected")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let has_worktree = state
        .selected_branch()
        .and_then(|b| b.worktree_path.as_ref())
        .is_some();

    if !has_worktree {
        let block = Block::default().title("Git Status");
        let paragraph = Paragraph::new(" No worktree (no git status available)")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Files section
    if state.detail_files.is_empty() {
        lines.push(Line::from(Span::styled(
            " Working tree clean",
            Style::default().fg(Color::Green),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!(" Changed files ({})", state.detail_files.len()),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        for file in &state.detail_files {
            let color = if file.starts_with("[S]") {
                Color::Green
            } else if file.starts_with("[?]") {
                Color::Red
            } else {
                Color::Yellow
            };
            lines.push(Line::from(Span::styled(
                format!("  {file}"),
                Style::default().fg(color),
            )));
        }
    }

    // Commits section
    if !state.detail_commits.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Recent commits",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for commit in &state.detail_commits {
            lines.push(Line::from(Span::styled(
                format!("  {commit}"),
                Style::default().fg(Color::White),
            )));
        }
    }

    let block = Block::default().title("Git Status");
    let paragraph = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, area);
}

/// Sessions section: shows count of active sessions on this branch.
fn render_detail_sessions(frame: &mut Frame, area: Rect, session_count: usize) {
    let content = if session_count == 0 {
        " No active sessions".to_string()
    } else {
        format!(" {} active session(s) on this branch", session_count)
    };
    let style = if session_count > 0 {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default().title("Sessions");
    let paragraph = Paragraph::new(content).block(block).style(style);
    frame.render_widget(paragraph, area);
}

/// Action modal: centered overlay with selectable action list.
fn render_action_modal(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let dialog = super::centered_rect(30, 7, area);

    frame.render_widget(Clear, dialog);

    let items: Vec<ListItem> = ACTION_LABELS
        .iter()
        .enumerate()
        .map(|(idx, label)| {
            let style = super::list_item_style(idx == state.action_modal_selected);
            let prefix = if idx == state.action_modal_selected {
                "\u{25B6} "
            } else {
                "  "
            };
            ListItem::new(Line::from(Span::styled(format!("{prefix}{label}"), style)))
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Actions")
        .border_style(Style::default().fg(Color::Yellow));
    let list = List::new(items).block(block);
    frame.render_widget(list, dialog);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static DOCKER_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn sample_branches() -> Vec<BranchItem> {
        vec![
            BranchItem {
                name: "main".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Main,
                worktree_path: None,
            },
            BranchItem {
                name: "develop".to_string(),
                is_head: true,
                is_local: true,
                category: BranchCategory::Develop,
                worktree_path: None,
            },
            BranchItem {
                name: "feature/login".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Feature,
                worktree_path: None,
            },
            BranchItem {
                name: "origin/feature/api".to_string(),
                is_head: false,
                is_local: false,
                category: BranchCategory::Feature,
                worktree_path: None,
            },
            BranchItem {
                name: "hotfix/crash".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Other,
                worktree_path: None,
            },
        ]
    }

    fn sample_containers() -> Vec<gwt_docker::ContainerInfo> {
        vec![
            gwt_docker::ContainerInfo {
                id: "abc123".to_string(),
                name: "web".to_string(),
                status: gwt_docker::ContainerStatus::Running,
                image: "nginx:latest".to_string(),
                ports: "0.0.0.0:8080->80/tcp".to_string(),
            },
            gwt_docker::ContainerInfo {
                id: "def456".to_string(),
                name: "db".to_string(),
                status: gwt_docker::ContainerStatus::Stopped,
                image: "postgres:16".to_string(),
                ports: String::new(),
            },
        ]
    }

    fn write_fake_docker(script_body: &str) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let script_path = dir.path().join("docker");
        let mut file = fs::File::create(&script_path).expect("create fake docker");
        file.write_all(script_body.as_bytes())
            .expect("write fake docker");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata().expect("stat fake docker").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).expect("chmod fake docker");
        }

        (dir, script_path)
    }

    fn with_fake_docker<R>(script_body: &str, f: impl FnOnce() -> R) -> R {
        let _guard = DOCKER_TEST_LOCK.lock().expect("lock docker tests");
        let (_dir, script_path) = write_fake_docker(script_body);
        let previous = std::env::var_os("GWT_DOCKER_BIN");
        std::env::set_var("GWT_DOCKER_BIN", &script_path);

        let result = f();

        match previous {
            Some(value) => std::env::set_var("GWT_DOCKER_BIN", value),
            None => std::env::remove_var("GWT_DOCKER_BIN"),
        }

        result
    }

    fn buffer_to_lines(buf: &ratatui::buffer::Buffer) -> Vec<String> {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn default_state() {
        let state = BranchesState::default();
        assert!(state.branches.is_empty());
        assert_eq!(state.selected, 0);
        assert_eq!(state.sort_mode, SortMode::Default);
        assert_eq!(state.view_mode, ViewMode::All);
        assert!(state.search_query.is_empty());
        assert!(!state.search_active);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        assert_eq!(state.selected, 0);

        update(&mut state, BranchesMessage::MoveDown);
        assert_eq!(state.selected, 1);

        // Move to last
        for _ in 0..4 {
            update(&mut state, BranchesMessage::MoveDown);
        }
        // Should wrap to 0
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_up_wraps() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        assert_eq!(state.selected, 0);

        update(&mut state, BranchesMessage::MoveUp);
        assert_eq!(state.selected, 4); // wraps to last
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = BranchesState::default();
        update(&mut state, BranchesMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, BranchesMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn select_returns_selected_branch() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.selected = 2;
        let branch = state.selected_branch().unwrap();
        assert_eq!(branch.name, "feature/login");
    }

    #[test]
    fn toggle_sort_cycles() {
        let mut state = BranchesState::default();
        assert_eq!(state.sort_mode, SortMode::Default);

        update(&mut state, BranchesMessage::ToggleSort);
        assert_eq!(state.sort_mode, SortMode::Name);

        update(&mut state, BranchesMessage::ToggleSort);
        assert_eq!(state.sort_mode, SortMode::Date);

        update(&mut state, BranchesMessage::ToggleSort);
        assert_eq!(state.sort_mode, SortMode::Default);
    }

    #[test]
    fn toggle_view_cycles() {
        let mut state = BranchesState::default();
        assert_eq!(state.view_mode, ViewMode::All);

        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::Local);

        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::Remote);

        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::All);
    }

    #[test]
    fn search_start_activates() {
        let mut state = BranchesState::default();
        update(&mut state, BranchesMessage::SearchStart);
        assert!(state.search_active);
    }

    #[test]
    fn search_input_appends() {
        let mut state = BranchesState::default();
        update(&mut state, BranchesMessage::SearchStart);
        update(&mut state, BranchesMessage::SearchInput('f'));
        update(&mut state, BranchesMessage::SearchInput('e'));
        assert_eq!(state.search_query, "fe");
    }

    #[test]
    fn search_input_ignored_when_inactive() {
        let mut state = BranchesState::default();
        update(&mut state, BranchesMessage::SearchInput('x'));
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn search_clear_resets() {
        let mut state = BranchesState::default();
        state.search_active = true;
        state.search_query = "test".to_string();

        update(&mut state, BranchesMessage::SearchClear);
        assert!(!state.search_active);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn set_branches_populates() {
        let mut state = BranchesState::default();
        state.selected = 99;
        update(&mut state, BranchesMessage::SetBranches(sample_branches()));
        assert_eq!(state.branches.len(), 5);
        assert_eq!(state.selected, 4); // clamped
    }

    #[test]
    fn filtered_branches_respects_view_mode() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();

        state.view_mode = ViewMode::Local;
        assert_eq!(state.filtered_branches().len(), 4);

        state.view_mode = ViewMode::Remote;
        assert_eq!(state.filtered_branches().len(), 1);
        assert_eq!(state.filtered_branches()[0].name, "origin/feature/api");
    }

    #[test]
    fn filtered_branches_respects_search() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.search_query = "feature".to_string();

        let filtered = state.filtered_branches();
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn categorize_branch_works() {
        assert_eq!(categorize_branch("main"), BranchCategory::Main);
        assert_eq!(categorize_branch("master"), BranchCategory::Main);
        assert_eq!(categorize_branch("develop"), BranchCategory::Develop);
        assert_eq!(categorize_branch("feature/login"), BranchCategory::Feature);
        assert_eq!(
            categorize_branch("origin/feature/x"),
            BranchCategory::Feature
        );
        assert_eq!(categorize_branch("hotfix/crash"), BranchCategory::Other);
    }

    #[test]
    fn render_with_branches_does_not_panic() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        // Header bar shows view/sort info (no bordered block title)
        assert!(text.contains("View:"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = BranchesState::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
    }

    #[test]
    fn sort_name_returns_alphabetical_order() {
        let mut state = BranchesState::default();
        state.branches = sample_branches(); // main, develop, feature/login, origin/feature/api, hotfix/crash
        state.sort_mode = SortMode::Name;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "develop",
                "feature/login",
                "hotfix/crash",
                "main",
                "origin/feature/api",
            ]
        );
    }

    #[test]
    fn sort_date_returns_alphabetical_fallback() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.sort_mode = SortMode::Date;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        // Date falls back to alphabetical since no date field exists
        assert_eq!(
            names,
            vec![
                "develop",
                "feature/login",
                "hotfix/crash",
                "main",
                "origin/feature/api",
            ]
        );
    }

    #[test]
    fn sort_default_preserves_insertion_order() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.sort_mode = SortMode::Default;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "main",
                "develop",
                "feature/login",
                "origin/feature/api",
                "hotfix/crash",
            ]
        );
    }

    #[test]
    fn search_then_navigate_selects_filtered_item() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();

        // Search for "feature" — matches feature/login and origin/feature/api
        update(&mut state, BranchesMessage::SearchStart);
        update(&mut state, BranchesMessage::SearchInput('f'));
        update(&mut state, BranchesMessage::SearchInput('e'));
        update(&mut state, BranchesMessage::SearchInput('a'));
        update(&mut state, BranchesMessage::SearchInput('t'));

        let filtered = state.filtered_branches();
        assert_eq!(filtered.len(), 2);
        assert_eq!(state.selected, 0);

        // MoveDown should navigate within filtered list
        update(&mut state, BranchesMessage::MoveDown);
        assert_eq!(state.selected, 1);

        let branch = state.selected_branch().unwrap();
        assert_eq!(branch.name, "origin/feature/api");
    }

    #[test]
    fn select_returns_correct_branch_after_sort() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.sort_mode = SortMode::Name;

        // After sort by name: develop, feature/login, hotfix/crash, main, origin/feature/api
        state.selected = 3;
        let branch = state.selected_branch().unwrap();
        assert_eq!(branch.name, "main");
    }

    #[test]
    fn view_toggle_clamps_selected() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.selected = 4; // last item (Other)

        // Switch to Remote — only 1 item
        update(&mut state, BranchesMessage::ToggleView);
        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::Remote);
        assert_eq!(state.selected, 0); // clamped
    }

    // ---- Branch detail tests ----

    #[test]
    fn detail_section_defaults_to_zero() {
        let state = BranchesState::default();
        assert_eq!(state.detail_section, 0);
    }

    #[test]
    fn next_detail_section_cycles_through_all() {
        let mut state = BranchesState::default();
        for expected in 1..=3 {
            update(&mut state, BranchesMessage::NextDetailSection);
            assert_eq!(state.detail_section, expected);
        }
        // Wraps back to 0
        update(&mut state, BranchesMessage::NextDetailSection);
        assert_eq!(state.detail_section, 0);
    }

    #[test]
    fn prev_detail_section_wraps_from_zero() {
        let mut state = BranchesState::default();
        assert_eq!(state.detail_section, 0);
        update(&mut state, BranchesMessage::PrevDetailSection);
        assert_eq!(state.detail_section, 3);
    }

    #[test]
    fn prev_detail_section_decrements() {
        let mut state = BranchesState::default();
        state.detail_section = 3;
        update(&mut state, BranchesMessage::PrevDetailSection);
        assert_eq!(state.detail_section, 2);
    }

    #[test]
    fn action_modal_defaults_to_hidden() {
        let state = BranchesState::default();
        assert!(!state.action_modal_visible);
        assert_eq!(state.action_modal_selected, 0);
    }

    #[test]
    fn open_action_modal_sets_visible() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        update(&mut state, BranchesMessage::OpenActionModal);
        assert!(state.action_modal_visible);
        assert_eq!(state.action_modal_selected, 0);
    }

    #[test]
    fn open_action_modal_ignored_when_empty() {
        let mut state = BranchesState::default();
        update(&mut state, BranchesMessage::OpenActionModal);
        assert!(!state.action_modal_visible);
    }

    #[test]
    fn close_action_modal_hides() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        update(&mut state, BranchesMessage::CloseActionModal);
        assert!(!state.action_modal_visible);
    }

    #[test]
    fn action_modal_down_cycles() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        update(&mut state, BranchesMessage::ActionModalDown);
        assert_eq!(state.action_modal_selected, 1);
        update(&mut state, BranchesMessage::ActionModalDown);
        assert_eq!(state.action_modal_selected, 2);
        update(&mut state, BranchesMessage::ActionModalDown);
        assert_eq!(state.action_modal_selected, 0); // wraps
    }

    #[test]
    fn action_modal_up_wraps() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        update(&mut state, BranchesMessage::ActionModalUp);
        assert_eq!(state.action_modal_selected, 2); // wraps to last
    }

    #[test]
    fn action_modal_select_launch_agent() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        state.action_modal_selected = 0;
        update(&mut state, BranchesMessage::ActionModalSelect);
        assert!(!state.action_modal_visible);
        assert!(state.pending_launch_agent);
    }

    #[test]
    fn action_modal_select_open_shell() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        state.action_modal_selected = 1;
        update(&mut state, BranchesMessage::ActionModalSelect);
        assert!(!state.action_modal_visible);
        assert!(state.pending_open_shell);
    }

    #[test]
    fn action_modal_select_delete_worktree() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        state.action_modal_selected = 2;
        update(&mut state, BranchesMessage::ActionModalSelect);
        assert!(!state.action_modal_visible);
        assert!(state.pending_delete_worktree);
    }

    #[test]
    fn launch_agent_sets_flag() {
        let mut state = BranchesState::default();
        assert!(!state.pending_launch_agent);
        update(&mut state, BranchesMessage::LaunchAgent);
        assert!(state.pending_launch_agent);
    }

    #[test]
    fn open_shell_sets_flag() {
        let mut state = BranchesState::default();
        assert!(!state.pending_open_shell);
        update(&mut state, BranchesMessage::OpenShell);
        assert!(state.pending_open_shell);
    }

    #[test]
    fn delete_worktree_sets_flag() {
        let mut state = BranchesState::default();
        assert!(!state.pending_delete_worktree);
        update(&mut state, BranchesMessage::DeleteWorktree);
        assert!(state.pending_delete_worktree);
    }

    #[test]
    fn render_with_detail_split_does_not_panic() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        // Header bar shows view/sort info (no bordered block title)
        assert!(text.contains("View:"));
    }

    #[test]
    fn render_detail_overview_shows_branch_info() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.detail_section = 0; // Overview
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        // Check bottom half contains Overview content
        let mut found_overview = false;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if line.contains("Overview") {
                found_overview = true;
                break;
            }
        }
        assert!(found_overview, "Detail panel should contain 'Overview'");
    }

    #[test]
    fn render_detail_overview_shows_docker_status_area() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.detail_section = 0; // Overview
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();

        let mut found_docker_status = false;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if line.contains("Docker status") {
                found_docker_status = true;
                break;
            }
        }

        assert!(
            found_docker_status,
            "Detail panel should contain a Docker status area"
        );
    }

    #[test]
    fn render_detail_overview_shows_no_containers_message() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.detail_section = 0;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        assert!(
            lines
                .iter()
                .any(|line| line.contains("No containers found")),
            "Detail panel should explain when there are no Docker containers"
        );
    }

    #[test]
    fn load_branch_detail_populates_docker_containers() {
        with_fake_docker(
            "#!/bin/sh\nprintf 'abc123\\tweb\\trunning\\tnginx:latest\\t0.0.0.0:8080->80/tcp\\n'\n",
            || {
                let tmp = tempfile::tempdir().expect("create temp worktree");
                let mut state = BranchesState::default();
                state.branches = vec![BranchItem {
                    name: "feature/docker".to_string(),
                    is_head: true,
                    is_local: true,
                    category: BranchCategory::Feature,
                    worktree_path: Some(tmp.path().to_path_buf()),
                }];

                load_branch_detail(&mut state, tmp.path());

                assert_eq!(state.docker_containers.len(), 1);
                let container = &state.docker_containers[0];
                assert_eq!(container.id, "abc123");
                assert_eq!(container.name, "web");
                assert_eq!(container.status, gwt_docker::ContainerStatus::Running);
                assert_eq!(container.ports, "0.0.0.0:8080->80/tcp");
            },
        );
    }

    #[test]
    fn docker_selection_and_lifecycle_intent_update_state() {
        let mut state = BranchesState::default();
        state.docker_containers = sample_containers();

        update(&mut state, BranchesMessage::DockerContainerDown);
        assert_eq!(state.docker_selected, 1);

        update(&mut state, BranchesMessage::DockerContainerUp);
        assert_eq!(state.docker_selected, 0);

        update(&mut state, BranchesMessage::DockerContainerRestart);
        assert_eq!(
            state.pending_docker_action,
            Some(PendingDockerAction {
                container_id: "abc123".to_string(),
                action: DockerLifecycleAction::Restart,
            })
        );
    }

    #[test]
    fn render_detail_overview_shows_selected_docker_container_details() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.docker_containers = sample_containers();
        state.docker_selected = 1;
        state.detail_section = 0;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        assert!(
            lines.iter().any(|line| line.contains("Selected: db")),
            "Detail panel should show the selected Docker container"
        );
        assert!(
            lines.iter().any(|line| line.contains("Status: Stopped")),
            "Detail panel should show Docker status"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Ports: No published ports")),
            "Detail panel should show Docker ports"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Controls: Up/Down select  S start  R restart")),
            "Detail panel should show Docker control hints"
        );
    }

    #[test]
    fn render_action_modal_shows_action_list() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.action_modal_visible = true;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut found_actions = false;
        let mut found_launch = false;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if line.contains("Actions") {
                found_actions = true;
            }
            if line.contains("Launch Agent") {
                found_launch = true;
            }
        }
        assert!(found_actions, "Should contain 'Actions' title");
        assert!(found_launch, "Should contain 'Launch Agent' action");
    }

    #[test]
    fn detail_section_labels_are_correct() {
        // Detail section tab labels are now rendered by app.rs in the pane border.
        // Verify the labels returned by detail_section_labels().
        let labels = detail_section_labels();
        assert!(labels.contains(&"Overview"));
        assert!(labels.contains(&"SPECs"));
        assert!(labels.contains(&"Git"));
        assert!(labels.contains(&"Sessions"));
        // Actions is an overlay modal, not a section tab
        assert!(!labels.contains(&"Actions"));
    }

    #[test]
    fn render_no_selected_branch_shows_placeholder() {
        let state = BranchesState::default(); // empty branches
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut found_no_branch = false;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if line.contains("No branch selected") {
                found_no_branch = true;
                break;
            }
        }
        assert!(
            found_no_branch,
            "Should show 'No branch selected' when empty"
        );
    }
}
