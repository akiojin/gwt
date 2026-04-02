//! Branches screen — branch list with PR/agent status (gwt-cli migration)

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use gwt_core::git::issue_cache::IssueExactCache;
use gwt_core::git::issue_linkage::WorktreeIssueLinkStore;
use gwt_core::git::{Branch, PrCache};
use gwt_core::worktree::{Worktree, WorktreeManager, WorktreeStatus as CoreWorktreeStatus};

// ---------------------------------------------------------------------------
// Safety status
// ---------------------------------------------------------------------------

/// Safety status for branch cleanup assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SafetyStatus {
    #[default]
    Unknown,
    Safe,
    Warning,
    Danger,
    Disabled,
}

impl SafetyStatus {
    pub fn icon(self) -> &'static str {
        match self {
            SafetyStatus::Safe => "o",
            SafetyStatus::Warning => "!",
            SafetyStatus::Danger => "x",
            SafetyStatus::Unknown => "?",
            SafetyStatus::Disabled => "-",
        }
    }

    pub fn color(self) -> Color {
        match self {
            SafetyStatus::Safe => Color::Green,
            SafetyStatus::Warning => Color::Yellow,
            SafetyStatus::Danger => Color::Red,
            SafetyStatus::Unknown => Color::DarkGray,
            SafetyStatus::Disabled => Color::DarkGray,
        }
    }
}

// ---------------------------------------------------------------------------
// Branch item
// ---------------------------------------------------------------------------

/// A single branch entry with metadata for display.
#[derive(Debug, Clone)]
pub struct BranchItem {
    pub name: String,
    pub is_current: bool,
    pub has_worktree: bool,
    pub worktree_path: Option<String>,
    pub session_count: usize,
    pub running_session_count: usize,
    pub stopped_session_count: usize,
    pub worktree_indicator: char,
    pub has_changes: bool,
    pub has_unpushed: bool,
    pub is_protected: bool,
    pub last_tool_usage: Option<String>,
    pub last_tool_id: Option<String>,
    pub quick_start_available: bool,
    pub linked_issue_number: Option<u64>,
    pub linked_issue_state: Option<String>,
    pub pr_title: Option<String>,
    pub pr_number: Option<u64>,
    pub pr_state: Option<String>,
    pub safety_status: SafetyStatus,
    pub is_remote: bool,
    pub last_commit_timestamp: Option<i64>,
}

impl BranchItem {
    /// Create a BranchItem from a gwt-core Branch.
    pub fn from_branch(branch: &Branch) -> Self {
        let is_remote = infer_is_remote(branch);
        let is_protected = is_protected_branch(&branch.name, is_remote);
        let safety = if is_protected || branch.is_current {
            SafetyStatus::Disabled
        } else {
            SafetyStatus::Unknown
        };

        Self {
            name: branch.name.clone(),
            is_current: branch.is_current,
            has_worktree: false,
            worktree_path: None,
            session_count: 0,
            running_session_count: 0,
            stopped_session_count: 0,
            worktree_indicator: '.',
            has_changes: false,
            has_unpushed: branch.ahead > 0,
            is_protected,
            last_tool_usage: None,
            last_tool_id: None,
            quick_start_available: false,
            linked_issue_number: None,
            linked_issue_state: None,
            pr_title: None,
            pr_number: None,
            pr_state: None,
            safety_status: safety,
            is_remote,
            last_commit_timestamp: branch.commit_timestamp,
        }
    }

    /// Get agent display color based on tool_id.
    pub fn agent_color(&self) -> Color {
        match self.last_tool_id.as_deref() {
            Some(id) => {
                let lower = id.to_lowercase();
                if lower.contains("claude") {
                    Color::Yellow
                } else if lower.contains("codex") {
                    Color::Cyan
                } else if lower.contains("gemini") {
                    Color::Magenta
                } else if lower.contains("opencode") {
                    Color::Green
                } else {
                    Color::White
                }
            }
            None => Color::DarkGray,
        }
    }

    /// Short display name for the agent.
    pub fn agent_label(&self) -> &str {
        self.last_tool_usage.as_deref().unwrap_or("")
    }

    /// Matches filter query against branch name and PR title.
    pub fn matches_filter(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        if self.name.to_lowercase().contains(&q) {
            return true;
        }
        if let Some(ref title) = self.pr_title {
            if title.to_lowercase().contains(&q) {
                return true;
            }
        }
        false
    }
}

fn infer_is_remote(branch: &Branch) -> bool {
    branch.name.starts_with("remotes/")
        || (branch.has_remote && branch.upstream.is_none() && branch.name.contains('/'))
}

fn normalize_branch_name(name: &str) -> &str {
    if let Some(stripped) = name.strip_prefix("remotes/") {
        if let Some((_, rest)) = stripped.split_once('/') {
            return rest;
        }
        return stripped;
    }
    if let Some(stripped) = name.strip_prefix("origin/") {
        return stripped;
    }
    if let Some(stripped) = name.strip_prefix("upstream/") {
        return stripped;
    }
    name
}

/// Check if a branch name is protected (main/master/develop).
fn is_protected_branch(name: &str, is_remote: bool) -> bool {
    let short = remote_short_name(name, is_remote);
    matches!(short, "main" | "master" | "develop" | "dev")
}

fn remote_short_name(name: &str, is_remote: bool) -> &str {
    if let Some(stripped) = name
        .strip_prefix("remotes/")
        .and_then(|s| s.split_once('/').map(|(_, r)| r))
    {
        return stripped;
    }
    if is_remote {
        if let Some((_, rest)) = name.split_once('/') {
            return rest;
        }
    }
    name
}

// ---------------------------------------------------------------------------
// View / Sort modes
// ---------------------------------------------------------------------------

/// View mode filter for the branch list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    All,
    Local,
    Remote,
}

impl ViewMode {
    pub fn label(self) -> &'static str {
        match self {
            ViewMode::All => "All",
            ViewMode::Local => "Local",
            ViewMode::Remote => "Remote",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            ViewMode::All => ViewMode::Local,
            ViewMode::Local => ViewMode::Remote,
            ViewMode::Remote => ViewMode::All,
        }
    }
}

/// Sort mode for the branch list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Default,
    Name,
    Updated,
}

impl SortMode {
    pub fn label(self) -> &'static str {
        match self {
            SortMode::Default => "Default",
            SortMode::Name => "Name",
            SortMode::Updated => "Updated",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            SortMode::Default => SortMode::Name,
            SortMode::Name => SortMode::Updated,
            SortMode::Updated => SortMode::Default,
        }
    }
}

// ---------------------------------------------------------------------------
// Branch list state
// ---------------------------------------------------------------------------

/// State for the branch list screen.
#[derive(Debug)]
pub struct BranchListState {
    pub branches: Vec<BranchItem>,
    pub selected: usize,
    pub filter_query: String,
    pub filter_mode: bool,
    pub view_mode: ViewMode,
    pub sort_mode: SortMode,
    pub scroll_offset: usize,
    pub loading: bool,
}

impl Default for BranchListState {
    fn default() -> Self {
        Self {
            branches: Vec::new(),
            selected: 0,
            filter_query: String::new(),
            filter_mode: false,
            view_mode: ViewMode::All,
            sort_mode: SortMode::Default,
            scroll_offset: 0,
            loading: false,
        }
    }
}

impl BranchListState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return indices of branches that match the current filter and view mode.
    pub fn filtered_indices(&self) -> Vec<usize> {
        self.branches
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                let view_ok = match self.view_mode {
                    ViewMode::All => true,
                    ViewMode::Local => !b.is_remote,
                    ViewMode::Remote => b.is_remote,
                };
                view_ok && b.matches_filter(&self.filter_query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Return filtered and sorted branch indices.
    pub fn visible_indices(&self) -> Vec<usize> {
        let mut indices = self.filtered_indices();
        let branches = &self.branches;
        let sort_mode = self.sort_mode;

        indices.sort_by(|&a, &b| {
            let ba = &branches[a];
            let bb = &branches[b];

            // Current branch always first.
            if ba.is_current && !bb.is_current {
                return Ordering::Less;
            }
            if !ba.is_current && bb.is_current {
                return Ordering::Greater;
            }

            match sort_mode {
                SortMode::Default => {
                    // Protected first, then by name type, then worktree, then timestamp.
                    let type_a = branch_sort_priority(&ba.name, ba.is_remote);
                    let type_b = branch_sort_priority(&bb.name, bb.is_remote);
                    if type_a != type_b {
                        return type_a.cmp(&type_b);
                    }
                    if ba.has_worktree != bb.has_worktree {
                        return if ba.has_worktree {
                            Ordering::Less
                        } else {
                            Ordering::Greater
                        };
                    }
                    compare_timestamps(ba, bb)
                        .unwrap_or_else(|| ba.name.to_lowercase().cmp(&bb.name.to_lowercase()))
                }
                SortMode::Name => ba.name.to_lowercase().cmp(&bb.name.to_lowercase()),
                SortMode::Updated => compare_timestamps(ba, bb)
                    .unwrap_or_else(|| ba.name.to_lowercase().cmp(&bb.name.to_lowercase())),
            }
        });

        indices
    }

    /// Count of visible branches.
    pub fn visible_count(&self) -> usize {
        self.visible_indices().len()
    }

    /// Clamp selected index to visible range.
    pub fn clamp_selection(&mut self) {
        let count = self.visible_count();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        let count = self.visible_count();
        if count == 0 {
            return;
        }
        self.selected = (self.selected + 1).min(count - 1);
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Get the currently selected BranchItem, if any.
    pub fn selected_branch(&self) -> Option<&BranchItem> {
        let indices = self.visible_indices();
        indices
            .get(self.selected)
            .and_then(|&i| self.branches.get(i))
    }

    pub fn selected_branch_name(&self) -> Option<String> {
        self.selected_branch().map(|b| b.name.clone())
    }

    /// Set branches and reset selection.
    pub fn set_branches(&mut self, branches: Vec<BranchItem>) {
        self.branches = branches;
        self.clamp_selection();
        self.loading = false;
    }

    /// Toggle filter input mode.
    pub fn toggle_filter(&mut self) {
        self.filter_mode = !self.filter_mode;
        if !self.filter_mode {
            // Keep filter text when exiting filter mode.
        }
    }

    /// Clear filter text and exit filter mode.
    pub fn clear_filter(&mut self) {
        self.filter_query.clear();
        self.filter_mode = false;
        self.clamp_selection();
    }

    /// Cycle view mode (All -> Local -> Remote -> All).
    pub fn cycle_view_mode(&mut self) {
        self.view_mode = self.view_mode.cycle();
        self.clamp_selection();
    }

    /// Cycle sort mode.
    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.cycle();
    }

    /// Ensure scroll_offset keeps the selected item visible within a viewport.
    pub fn ensure_visible(&mut self, viewport_height: usize) {
        if viewport_height == 0 {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + viewport_height {
            self.scroll_offset = self.selected - viewport_height + 1;
        }
    }
}

/// Priority for branch name sorting (lower = higher priority).
fn branch_sort_priority(name: &str, is_remote: bool) -> u8 {
    let short = remote_short_name(name, is_remote).to_lowercase();
    if short == "main" || short == "master" {
        0
    } else if short == "develop" || short == "dev" {
        1
    } else if short.starts_with("feature/") {
        2
    } else if short.starts_with("bugfix/") || short.starts_with("hotfix/") {
        3
    } else if short.starts_with("release/") {
        4
    } else {
        5
    }
}

/// Compare two branch items by timestamp (newer first).
fn compare_timestamps(a: &BranchItem, b: &BranchItem) -> Option<Ordering> {
    match (a.last_commit_timestamp, b.last_commit_timestamp) {
        (Some(ta), Some(tb)) if ta != tb => Some(tb.cmp(&ta)),
        (Some(_), None) => Some(Ordering::Less),
        (None, Some(_)) => Some(Ordering::Greater),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Messages for the branches screen.
#[derive(Debug)]
pub enum BranchesMessage {
    Refresh,
    SelectNext,
    SelectPrev,
    ToggleFilter,
    FilterInput(char),
    FilterBackspace,
    FilterClear,
    CycleViewMode,
    CycleSortMode,
    Enter,
    Delete,
    Loaded(Vec<BranchItem>),
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

/// Handle a key event for the branches screen.
pub fn handle_key(state: &BranchListState, key: &KeyEvent) -> Option<BranchesMessage> {
    if state.filter_mode {
        return handle_filter_key(key);
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(BranchesMessage::SelectNext),
        KeyCode::Char('k') | KeyCode::Up => Some(BranchesMessage::SelectPrev),
        KeyCode::Char('/') => Some(BranchesMessage::ToggleFilter),
        KeyCode::Char('v') => Some(BranchesMessage::CycleViewMode),
        KeyCode::Char('s') => Some(BranchesMessage::CycleSortMode),
        KeyCode::Char('r') => Some(BranchesMessage::Refresh),
        KeyCode::Char('d') => Some(BranchesMessage::Delete),
        KeyCode::Enter => Some(BranchesMessage::Enter),
        _ => None,
    }
}

/// Handle key events while in filter input mode.
fn handle_filter_key(key: &KeyEvent) -> Option<BranchesMessage> {
    match key.code {
        KeyCode::Esc => Some(BranchesMessage::ToggleFilter),
        KeyCode::Enter => Some(BranchesMessage::ToggleFilter),
        KeyCode::Backspace => Some(BranchesMessage::FilterBackspace),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(BranchesMessage::FilterClear)
        }
        KeyCode::Char(c) => Some(BranchesMessage::FilterInput(c)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Apply a BranchesMessage to the BranchListState.
pub fn update(state: &mut BranchListState, msg: BranchesMessage) {
    match msg {
        BranchesMessage::SelectNext => state.select_next(),
        BranchesMessage::SelectPrev => state.select_prev(),
        BranchesMessage::ToggleFilter => state.toggle_filter(),
        BranchesMessage::FilterInput(c) => {
            state.filter_query.push(c);
            state.clamp_selection();
        }
        BranchesMessage::FilterBackspace => {
            state.filter_query.pop();
            state.clamp_selection();
        }
        BranchesMessage::FilterClear => state.clear_filter(),
        BranchesMessage::CycleViewMode => state.cycle_view_mode(),
        BranchesMessage::CycleSortMode => state.cycle_sort_mode(),
        BranchesMessage::Refresh => {
            state.loading = true;
        }
        BranchesMessage::Loaded(branches) => {
            state.set_branches(branches);
        }
        BranchesMessage::Enter | BranchesMessage::Delete => {
            // Handled at app level.
        }
    }
}

// ---------------------------------------------------------------------------
// Load branches from gwt-core
// ---------------------------------------------------------------------------

/// Load branches from the repository at `repo_root`.
pub fn load_branches(repo_root: &Path) -> Vec<BranchItem> {
    let local = Branch::list(repo_root).unwrap_or_default();
    let remote = Branch::list_remote(repo_root).unwrap_or_default();

    // Get tool usage map for agent info.
    let tool_map = gwt_core::config::get_last_tool_usage_map(repo_root);

    let mut items: Vec<BranchItem> = Vec::with_capacity(local.len() + remote.len());

    for branch in &local {
        let mut item = BranchItem::from_branch(branch);
        // Enrich with tool usage data.
        if let Some(entry) = tool_map.get(&branch.name) {
            item.last_tool_usage = Some(entry.format_tool_usage());
            item.last_tool_id = Some(entry.tool_id.clone());
            item.quick_start_available = entry.session_id.is_some();
        }
        items.push(item);
    }

    for branch in &remote {
        items.push(BranchItem::from_branch(branch));
    }

    items
}

pub fn load_branches_enriched(repo_root: &Path) -> Vec<BranchItem> {
    let mut items = load_branches(repo_root);

    let worktrees = WorktreeManager::new(repo_root)
        .and_then(|manager| manager.list())
        .unwrap_or_default();
    apply_worktree_metadata(&mut items, &worktrees);

    let issue_links = WorktreeIssueLinkStore::load(repo_root);
    let issue_cache = IssueExactCache::load(repo_root);
    apply_issue_metadata(&mut items, &issue_links, &issue_cache);

    let mut pr_cache = PrCache::new();
    pr_cache.populate(repo_root);
    apply_pr_metadata(&mut items, &pr_cache);

    items
}

fn apply_worktree_metadata(items: &mut [BranchItem], worktrees: &[Worktree]) {
    let worktree_map: HashMap<String, &Worktree> = worktrees
        .iter()
        .filter_map(|worktree| {
            worktree
                .branch
                .as_ref()
                .map(|branch| (normalize_branch_name(branch).to_string(), worktree))
        })
        .collect();

    for item in items {
        let key = normalize_branch_name(&item.name).to_string();
        let Some(worktree) = worktree_map.get(&key).copied() else {
            apply_safety_status(item);
            continue;
        };

        item.worktree_path = Some(worktree.path.display().to_string());
        item.has_worktree = matches!(worktree.status, CoreWorktreeStatus::Active);
        item.has_changes = worktree.has_changes;
        item.has_unpushed = worktree.has_unpushed;
        item.worktree_indicator = match worktree.status {
            CoreWorktreeStatus::Active => 'w',
            CoreWorktreeStatus::Locked => 'l',
            CoreWorktreeStatus::Prunable | CoreWorktreeStatus::Missing => 'x',
        };
        apply_safety_status(item);
    }
}

fn apply_pr_metadata(items: &mut [BranchItem], pr_cache: &PrCache) {
    for item in items {
        let key = normalize_branch_name(&item.name);
        if let Some(pr) = pr_cache.get(key) {
            item.pr_title = Some(pr.title.clone());
            item.pr_number = Some(pr.number);
            item.pr_state = Some(pr.state.to_lowercase());
        }
    }
}

fn apply_issue_metadata(
    items: &mut [BranchItem],
    issue_links: &WorktreeIssueLinkStore,
    issue_cache: &IssueExactCache,
) {
    for item in items {
        let key = normalize_branch_name(&item.name);
        if let Some(link) = issue_links.get_link(key) {
            item.linked_issue_number = Some(link.issue_number);
            item.linked_issue_state = issue_cache
                .get(link.issue_number)
                .map(|entry| entry.state.to_lowercase());
        }
    }
}

fn apply_safety_status(item: &mut BranchItem) {
    item.safety_status = if item.is_protected || item.is_current {
        SafetyStatus::Disabled
    } else if item.has_changes {
        SafetyStatus::Danger
    } else if item.has_unpushed {
        SafetyStatus::Warning
    } else {
        SafetyStatus::Safe
    };
}

fn branch_runtime_summary(branch: &BranchItem) -> Option<String> {
    let mut parts = Vec::new();
    if branch.running_session_count > 0 {
        parts.push(format!("●{}", branch.running_session_count));
    }
    if branch.stopped_session_count > 0 {
        parts.push(format!("○{}", branch.stopped_session_count));
    }
    if let Some(tool) = branch.last_tool_usage.as_deref() {
        if !tool.is_empty() {
            parts.push(tool.to_string());
        }
    }
    if branch.quick_start_available {
        parts.push("↺".to_string());
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the branches screen.
pub fn render(state: &BranchListState, buf: &mut Buffer, area: Rect) {
    if area.height < 3 || area.width < 10 {
        return;
    }

    // Layout: header (2 lines) + list + footer (1 line if filter mode)
    let footer_height = if state.filter_mode { 1 } else { 0 };
    let header_height = 2u16;
    let list_height = area.height.saturating_sub(header_height + footer_height);

    let header_area = Rect::new(area.x, area.y, area.width, header_height);
    let list_area = Rect::new(area.x, area.y + header_height, area.width, list_height);
    let footer_area = Rect::new(
        area.x,
        area.y + header_height + list_height,
        area.width,
        footer_height,
    );

    render_header(state, buf, header_area);
    render_list(state, buf, list_area);
    if state.filter_mode {
        render_filter_bar(state, buf, footer_area);
    }
}

/// Render the header with view/sort mode and stats.
fn render_header(state: &BranchListState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }

    // Line 1: Title
    let visible = state.visible_count();
    let total = state.branches.len();
    let title = if state.loading {
        " Branches (loading...)".to_string()
    } else if visible == total {
        format!(" Branches ({total})")
    } else {
        format!(" Branches ({visible}/{total})")
    };

    let title_span = Span::styled(title, Style::default().fg(Color::White).bold());
    let line1 = Line::from(vec![title_span]);
    buf.set_line(area.x, area.y, &line1, area.width);

    if area.height < 2 {
        return;
    }

    // Line 2: View mode + Sort mode + keyhints
    let mode_line = Line::from(vec![
        Span::styled(
            format!(" [v] {}", state.view_mode.label()),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(
            format!("[s] {}", state.sort_mode.label()),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("  ", Style::default()),
        Span::styled("[/] Filter", Style::default().fg(Color::DarkGray)),
        Span::styled("  ", Style::default()),
        Span::styled("[r] Refresh", Style::default().fg(Color::DarkGray)),
    ]);
    buf.set_line(area.x, area.y + 1, &mode_line, area.width);
}

/// Render the branch list rows.
fn render_list(state: &BranchListState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }

    let indices = state.visible_indices();

    if indices.is_empty() {
        let msg = if state.filter_query.is_empty() {
            "No branches found"
        } else {
            "No matching branches"
        };
        let para = Paragraph::new(msg)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        let y = area.y + area.height / 2;
        let text_area = Rect::new(area.x, y, area.width, 1);
        ratatui::widgets::Widget::render(para, text_area, buf);
        return;
    }

    let viewport = area.height as usize;

    // Determine scroll range.
    let offset = if state.selected < state.scroll_offset {
        state.selected
    } else if state.selected >= state.scroll_offset + viewport {
        state.selected - viewport + 1
    } else {
        state.scroll_offset
    };

    for (row, vis_idx) in indices.iter().skip(offset).take(viewport).enumerate() {
        let branch = &state.branches[*vis_idx];
        let is_selected = row + offset == state.selected;
        let y = area.y + row as u16;

        render_branch_row(branch, is_selected, buf, area.x, y, area.width);
    }
}

/// Render a single branch row.
fn render_branch_row(
    branch: &BranchItem,
    is_selected: bool,
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
) {
    let mut spans: Vec<Span> = Vec::new();

    // Selection indicator
    let sel_char = if is_selected { ">" } else { " " };
    let sel_style = if is_selected {
        Style::default().fg(Color::White).bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };
    spans.push(Span::styled(sel_char, sel_style));

    // Current branch marker
    let current_char = if branch.is_current { "*" } else { " " };
    spans.push(Span::styled(
        current_char,
        Style::default().fg(Color::Green),
    ));

    // Branch name (truncated)
    let name_color = if branch.is_current {
        Color::Green
    } else if branch.has_worktree {
        Color::White
    } else {
        Color::DarkGray
    };
    let max_name_len = 30.min(width as usize / 3);
    let display_name = if branch.name.len() > max_name_len {
        format!("{}...", &branch.name[..max_name_len - 3])
    } else {
        branch.name.clone()
    };
    spans.push(Span::styled(
        format!(" {display_name:<max_name_len$}"),
        Style::default().fg(name_color),
    ));

    spans.push(Span::styled(
        format!(" {}", branch.worktree_indicator),
        Style::default().fg(match branch.worktree_indicator {
            'w' => Color::Green,
            'l' => Color::Yellow,
            'x' => Color::Red,
            _ => Color::DarkGray,
        }),
    ));

    // Changes indicator
    let changes_char = if branch.has_changes { "*" } else { " " };
    spans.push(Span::styled(
        format!(" {changes_char}"),
        Style::default().fg(if branch.has_changes {
            Color::Yellow
        } else {
            Color::DarkGray
        }),
    ));

    // Safety status
    spans.push(Span::styled(
        format!(" {}", branch.safety_status.icon()),
        Style::default().fg(branch.safety_status.color()),
    ));

    spans.push(Span::styled(
        format!(" s:{}", branch.session_count),
        Style::default().fg(if branch.session_count > 0 {
            Color::Cyan
        } else {
            Color::DarkGray
        }),
    ));

    if let Some(number) = branch.linked_issue_number {
        let issue_color = match branch.linked_issue_state.as_deref() {
            Some(state) if state.eq_ignore_ascii_case("open") => Color::Green,
            Some(state) if state.eq_ignore_ascii_case("closed") => Color::Red,
            _ => Color::DarkGray,
        };
        let state_label = branch
            .linked_issue_state
            .as_deref()
            .unwrap_or("")
            .to_lowercase();
        let issue_text = if state_label.is_empty() {
            format!(" #{number}")
        } else {
            format!(" #{number} {state_label}")
        };
        spans.push(Span::styled(issue_text, Style::default().fg(issue_color)));
    }

    // PR info
    if let (Some(number), Some(ref pr_state)) = (branch.pr_number, &branch.pr_state) {
        let pr_color = match pr_state.as_str() {
            "open" => Color::Green,
            "merged" => Color::Magenta,
            "closed" => Color::Red,
            _ => Color::DarkGray,
        };
        spans.push(Span::styled(
            format!(" #{number} {pr_state}"),
            Style::default().fg(pr_color),
        ));

        // PR title (fill remaining space)
        if let Some(ref title) = branch.pr_title {
            let remaining =
                width as usize - spans.iter().map(|s| s.content.len()).sum::<usize>() - 1;
            if remaining > 5 {
                let display_title = if title.len() > remaining {
                    format!("{}...", &title[..remaining - 3])
                } else {
                    title.clone()
                };
                spans.push(Span::styled(
                    format!(" {display_title}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
    }

    let right_summary = branch_runtime_summary(branch);
    if let Some(summary) = right_summary.as_deref() {
        let used = spans.iter().map(|s| s.content.len()).sum::<usize>();
        let summary_width = summary.len() + 1;
        let available_gap = width as usize;
        if available_gap > used + summary_width {
            let gap = available_gap - used - summary_width;
            spans.push(Span::styled(" ".repeat(gap), Style::default()));
            spans.push(Span::styled(
                format!(" {summary}"),
                Style::default().fg(Color::Gray),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {summary}"),
                Style::default().fg(Color::Gray),
            ));
        }
    }

    let line = Line::from(spans);

    // Background highlight for selected row.
    if is_selected {
        for col in x..x + width {
            buf[(col, y)].set_style(Style::default().bg(Color::Rgb(40, 40, 60)));
        }
    }

    buf.set_line(x, y, &line, width);
}

/// Render the filter input bar at the bottom.
fn render_filter_bar(state: &BranchListState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }
    let line = Line::from(vec![
        Span::styled(" /", Style::default().fg(Color::Cyan).bold()),
        Span::styled(&state.filter_query, Style::default().fg(Color::White)),
        Span::styled(
            "_",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ]);
    buf.set_line(area.x, area.y, &line, area.width);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_branch(name: &str, is_current: bool) -> BranchItem {
        BranchItem {
            name: name.to_string(),
            is_current,
            has_worktree: false,
            worktree_path: None,
            session_count: 0,
            running_session_count: 0,
            stopped_session_count: 0,
            worktree_indicator: '.',
            has_changes: false,
            has_unpushed: false,
            is_protected: is_protected_branch(name, false),
            last_tool_usage: None,
            last_tool_id: None,
            quick_start_available: false,
            linked_issue_number: None,
            linked_issue_state: None,
            pr_title: None,
            pr_number: None,
            pr_state: None,
            safety_status: SafetyStatus::Unknown,
            is_remote: false,
            last_commit_timestamp: None,
        }
    }

    fn make_branch_with_agent(name: &str, tool_label: &str, tool_id: &str) -> BranchItem {
        let mut b = make_branch(name, false);
        b.last_tool_usage = Some(tool_label.to_string());
        b.last_tool_id = Some(tool_id.to_string());
        b
    }

    fn make_branch_with_pr(name: &str, pr_number: u64, state: &str, title: &str) -> BranchItem {
        let mut b = make_branch(name, false);
        b.pr_number = Some(pr_number);
        b.pr_state = Some(state.to_string());
        b.pr_title = Some(title.to_string());
        b
    }

    // -- BranchItem tests --

    #[test]
    fn branch_item_matches_filter_by_name() {
        let b = make_branch("feature/cool-thing", false);
        assert!(b.matches_filter("cool"));
        assert!(b.matches_filter("FEATURE"));
        assert!(!b.matches_filter("hotfix"));
    }

    #[test]
    fn branch_item_matches_filter_by_pr_title() {
        let b = make_branch_with_pr("feat/x", 42, "open", "Add login page");
        assert!(b.matches_filter("login"));
        assert!(b.matches_filter("Login")); // case-insensitive
        assert!(!b.matches_filter("payment")); // not in name or title
    }

    #[test]
    fn branch_item_empty_filter_matches_all() {
        let b = make_branch("any-branch", false);
        assert!(b.matches_filter(""));
    }

    #[test]
    fn from_branch_marks_origin_prefixed_refs_as_remote() {
        let branch = Branch {
            name: "origin/main".to_string(),
            is_current: false,
            has_remote: true,
            upstream: None,
            commit: "abc1234".to_string(),
            ahead: 0,
            behind: 0,
            commit_timestamp: None,
            is_gone: false,
        };

        let item = BranchItem::from_branch(&branch);
        assert!(item.is_remote);
        assert!(item.is_protected);
    }

    #[test]
    fn from_branch_keeps_local_refs_local() {
        let branch = Branch {
            name: "feature/auth".to_string(),
            is_current: false,
            has_remote: true,
            upstream: Some("origin/feature/auth".to_string()),
            commit: "abc1234".to_string(),
            ahead: 0,
            behind: 0,
            commit_timestamp: None,
            is_gone: false,
        };

        let item = BranchItem::from_branch(&branch);
        assert!(!item.is_remote);
        assert!(!item.is_protected);
    }

    #[test]
    fn agent_color_maps_correctly() {
        let claude = make_branch_with_agent("b", "Claude Code", "claude-code");
        assert_eq!(claude.agent_color(), Color::Yellow);

        let codex = make_branch_with_agent("b", "Codex", "codex-cli");
        assert_eq!(codex.agent_color(), Color::Cyan);

        let gemini = make_branch_with_agent("b", "Gemini", "gemini-cli");
        assert_eq!(gemini.agent_color(), Color::Magenta);

        let none = make_branch("b", false);
        assert_eq!(none.agent_color(), Color::DarkGray);
    }

    #[test]
    fn safety_status_icon_and_color() {
        assert_eq!(SafetyStatus::Safe.icon(), "o");
        assert_eq!(SafetyStatus::Safe.color(), Color::Green);
        assert_eq!(SafetyStatus::Danger.icon(), "x");
        assert_eq!(SafetyStatus::Danger.color(), Color::Red);
        assert_eq!(SafetyStatus::Warning.icon(), "!");
        assert_eq!(SafetyStatus::Warning.color(), Color::Yellow);
    }

    // -- BranchListState tests --

    #[test]
    fn state_filtered_indices_respects_view_mode() {
        let mut state = BranchListState::new();
        state.branches = vec![
            make_branch("main", true),
            {
                let mut b = make_branch("remotes/origin/main", false);
                b.is_remote = true;
                b
            },
            make_branch("feature/x", false),
        ];

        state.view_mode = ViewMode::All;
        assert_eq!(state.filtered_indices().len(), 3);

        state.view_mode = ViewMode::Local;
        assert_eq!(state.filtered_indices().len(), 2);

        state.view_mode = ViewMode::Remote;
        assert_eq!(state.filtered_indices().len(), 1);
    }

    #[test]
    fn state_filtered_indices_respects_filter_query() {
        let mut state = BranchListState::new();
        state.branches = vec![
            make_branch("main", true),
            make_branch("feature/auth", false),
            make_branch("feature/payments", false),
        ];

        state.filter_query = "auth".to_string();
        let indices = state.filtered_indices();
        assert_eq!(indices.len(), 1);
        assert_eq!(state.branches[indices[0]].name, "feature/auth");
    }

    #[test]
    fn state_sort_default_current_first() {
        let mut state = BranchListState::new();
        state.branches = vec![
            make_branch("feature/z", false),
            make_branch("main", true),
            make_branch("feature/a", false),
        ];

        let indices = state.visible_indices();
        assert_eq!(state.branches[indices[0]].name, "main");
    }

    #[test]
    fn state_sort_by_name() {
        let mut state = BranchListState::new();
        state.sort_mode = SortMode::Name;
        state.branches = vec![
            make_branch("feature/z", false),
            make_branch("feature/a", false),
            make_branch("feature/m", false),
        ];

        let indices = state.visible_indices();
        assert_eq!(state.branches[indices[0]].name, "feature/a");
        assert_eq!(state.branches[indices[1]].name, "feature/m");
        assert_eq!(state.branches[indices[2]].name, "feature/z");
    }

    #[test]
    fn state_sort_by_updated() {
        let mut state = BranchListState::new();
        state.sort_mode = SortMode::Updated;
        state.branches = vec![
            {
                let mut b = make_branch("old", false);
                b.last_commit_timestamp = Some(100);
                b
            },
            {
                let mut b = make_branch("new", false);
                b.last_commit_timestamp = Some(300);
                b
            },
            {
                let mut b = make_branch("mid", false);
                b.last_commit_timestamp = Some(200);
                b
            },
        ];

        let indices = state.visible_indices();
        assert_eq!(state.branches[indices[0]].name, "new");
        assert_eq!(state.branches[indices[1]].name, "mid");
        assert_eq!(state.branches[indices[2]].name, "old");
    }

    #[test]
    fn state_select_next_prev() {
        let mut state = BranchListState::new();
        state.branches = vec![
            make_branch("a", false),
            make_branch("b", false),
            make_branch("c", false),
        ];
        assert_eq!(state.selected, 0);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_next();
        assert_eq!(state.selected, 2);

        // Stays at end.
        state.select_next();
        assert_eq!(state.selected, 2);

        state.select_prev();
        assert_eq!(state.selected, 1);

        state.select_prev();
        assert_eq!(state.selected, 0);

        // Stays at start.
        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn state_clamp_selection() {
        let mut state = BranchListState::new();
        state.selected = 5;
        state.branches = vec![make_branch("a", false), make_branch("b", false)];
        state.clamp_selection();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn state_toggle_filter() {
        let mut state = BranchListState::new();
        assert!(!state.filter_mode);
        state.toggle_filter();
        assert!(state.filter_mode);
        state.toggle_filter();
        assert!(!state.filter_mode);
    }

    #[test]
    fn state_clear_filter() {
        let mut state = BranchListState::new();
        state.filter_query = "test".to_string();
        state.filter_mode = true;
        state.clear_filter();
        assert!(state.filter_query.is_empty());
        assert!(!state.filter_mode);
    }

    #[test]
    fn state_cycle_view_mode() {
        let mut state = BranchListState::new();
        assert_eq!(state.view_mode, ViewMode::All);
        state.cycle_view_mode();
        assert_eq!(state.view_mode, ViewMode::Local);
        state.cycle_view_mode();
        assert_eq!(state.view_mode, ViewMode::Remote);
        state.cycle_view_mode();
        assert_eq!(state.view_mode, ViewMode::All);
    }

    #[test]
    fn state_cycle_sort_mode() {
        let mut state = BranchListState::new();
        assert_eq!(state.sort_mode, SortMode::Default);
        state.cycle_sort_mode();
        assert_eq!(state.sort_mode, SortMode::Name);
        state.cycle_sort_mode();
        assert_eq!(state.sort_mode, SortMode::Updated);
        state.cycle_sort_mode();
        assert_eq!(state.sort_mode, SortMode::Default);
    }

    #[test]
    fn state_ensure_visible() {
        let mut state = BranchListState::new();
        state.selected = 20;
        state.scroll_offset = 0;
        state.ensure_visible(10);
        assert_eq!(state.scroll_offset, 11);

        state.selected = 5;
        state.ensure_visible(10);
        // 5 is within [11..21), so it's above viewport
        assert_eq!(state.scroll_offset, 5);
    }

    // -- Key handling tests --

    #[test]
    fn handle_key_navigation() {
        let state = BranchListState::new();

        let key_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_j),
            Some(BranchesMessage::SelectNext)
        ));

        let key_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_k),
            Some(BranchesMessage::SelectPrev)
        ));

        let key_slash = KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_slash),
            Some(BranchesMessage::ToggleFilter)
        ));
    }

    #[test]
    fn handle_key_filter_mode_input() {
        let mut state = BranchListState::new();
        state.filter_mode = true;

        let key_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_a),
            Some(BranchesMessage::FilterInput('a'))
        ));

        let key_bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_bs),
            Some(BranchesMessage::FilterBackspace)
        ));

        let key_esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_esc),
            Some(BranchesMessage::ToggleFilter)
        ));
    }

    // -- Update tests --

    #[test]
    fn update_select_next_prev() {
        let mut state = BranchListState::new();
        state.branches = vec![make_branch("a", false), make_branch("b", false)];

        update(&mut state, BranchesMessage::SelectNext);
        assert_eq!(state.selected, 1);

        update(&mut state, BranchesMessage::SelectPrev);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn update_filter_input() {
        let mut state = BranchListState::new();
        state.branches = vec![make_branch("a", false)];
        state.filter_mode = true;

        update(&mut state, BranchesMessage::FilterInput('h'));
        update(&mut state, BranchesMessage::FilterInput('i'));
        assert_eq!(state.filter_query, "hi");

        update(&mut state, BranchesMessage::FilterBackspace);
        assert_eq!(state.filter_query, "h");

        update(&mut state, BranchesMessage::FilterClear);
        assert!(state.filter_query.is_empty());
        assert!(!state.filter_mode);
    }

    #[test]
    fn update_loaded_sets_branches() {
        let mut state = BranchListState::new();
        state.loading = true;
        state.selected = 99;

        let branches = vec![make_branch("main", true), make_branch("dev", false)];
        update(&mut state, BranchesMessage::Loaded(branches));

        assert!(!state.loading);
        assert_eq!(state.branches.len(), 2);
        assert_eq!(state.selected, 1); // clamped
    }

    // -- Render tests --

    #[test]
    fn render_empty_state() {
        let state = BranchListState::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_with_branches() {
        let mut state = BranchListState::new();
        state.branches = vec![
            make_branch("main", true),
            make_branch_with_agent("feature/auth", "Claude Code", "claude-code"),
            make_branch_with_pr("feature/pay", 42, "open", "Add payments"),
        ];

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_branch_row_shows_session_count() {
        let mut state = BranchListState::new();
        let mut branch = make_branch("feature/demo", false);
        branch.session_count = 2;
        branch.running_session_count = 1;
        branch.stopped_session_count = 1;
        branch.worktree_indicator = 'w';
        branch.last_tool_usage = Some("Codex@1.2.3".to_string());
        branch.quick_start_available = true;
        branch.linked_issue_number = Some(42);
        branch.linked_issue_state = Some("open".to_string());
        state.branches = vec![branch];

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();

        let row_text: String = (0..80)
            .map(|x| {
                terminal
                    .backend()
                    .buffer()
                    .cell((x, 2))
                    .map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))
            })
            .collect();
        assert!(
            row_text.contains("s:2"),
            "expected session count in row, got: {row_text:?}"
        );
        assert!(row_text.contains("●1"), "expected running summary in row");
        assert!(row_text.contains("○1"), "expected stopped summary in row");
        assert!(row_text.contains(" w"), "expected worktree indicator in row");
        assert!(row_text.contains("Codex@1.2.3"), "expected tool summary in row");
        assert!(row_text.contains("↺"), "expected quick-start marker in row");
        assert!(row_text.contains("#42 open"), "expected linked issue in row");
    }

    #[test]
    fn branch_runtime_summary_formats_runtime_state() {
        let mut branch = make_branch("feature/demo", false);
        branch.running_session_count = 2;
        branch.stopped_session_count = 1;
        branch.last_tool_usage = Some("Claude@1.0.0".to_string());
        branch.quick_start_available = true;

        let summary = branch_runtime_summary(&branch).expect("summary");
        assert!(summary.contains("●2"));
        assert!(summary.contains("○1"));
        assert!(summary.contains("Claude@1.0.0"));
        assert!(summary.contains("↺"));
    }

    #[test]
    fn apply_worktree_metadata_updates_safety_and_worktree_flags() {
        let mut item = make_branch("feature/demo", false);
        let worktrees = vec![Worktree {
            path: std::path::PathBuf::from("/tmp/demo"),
            branch: Some("feature/demo".to_string()),
            commit: "abc1234".to_string(),
            status: CoreWorktreeStatus::Active,
            is_main: false,
            has_changes: true,
            has_unpushed: false,
        }];

        apply_worktree_metadata(std::slice::from_mut(&mut item), &worktrees);

        assert!(item.has_worktree);
        assert_eq!(item.worktree_indicator, 'w');
        assert_eq!(item.safety_status, SafetyStatus::Danger);
    }

    #[test]
    fn apply_issue_metadata_sets_linked_issue_fields() {
        let mut items = vec![make_branch("feature/issue-42-demo", false)];
        let mut store = WorktreeIssueLinkStore::default();
        store.set_link("feature/issue-42-demo", 42, gwt_core::git::issue_linkage::LinkSource::BranchParse);
        let mut cache = IssueExactCache::default();
        cache.upsert(gwt_core::git::issue_cache::IssueExactCacheEntry {
            number: 42,
            title: "Issue 42".to_string(),
            url: "https://example.com/issues/42".to_string(),
            state: "OPEN".to_string(),
            labels: vec![],
            updated_at: "2026-04-02T00:00:00Z".to_string(),
            fetched_at: 0,
        });

        apply_issue_metadata(&mut items, &store, &cache);

        assert_eq!(items[0].linked_issue_number, Some(42));
        assert_eq!(items[0].linked_issue_state.as_deref(), Some("open"));
    }

    #[test]
    fn render_with_filter_mode() {
        let mut state = BranchListState::new();
        state.branches = vec![make_branch("main", true)];
        state.filter_mode = true;
        state.filter_query = "test".to_string();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_small_area_does_not_panic() {
        let state = BranchListState::new();
        let backend = TestBackend::new(5, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    // -- Protected branch detection --

    #[test]
    fn is_protected_branch_detection() {
        assert!(is_protected_branch("main", false));
        assert!(is_protected_branch("master", false));
        assert!(is_protected_branch("develop", false));
        assert!(is_protected_branch("dev", false));
        assert!(is_protected_branch("remotes/origin/main", true));
        assert!(is_protected_branch("origin/main", true));
        assert!(!is_protected_branch("feature/cool", false));
    }

    // -- ViewMode / SortMode --

    #[test]
    fn view_mode_label() {
        assert_eq!(ViewMode::All.label(), "All");
        assert_eq!(ViewMode::Local.label(), "Local");
        assert_eq!(ViewMode::Remote.label(), "Remote");
    }

    #[test]
    fn sort_mode_label() {
        assert_eq!(SortMode::Default.label(), "Default");
        assert_eq!(SortMode::Name.label(), "Name");
        assert_eq!(SortMode::Updated.label(), "Updated");
    }
}
