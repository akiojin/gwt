//! GitView Screen - Git status view for selected branch (SPEC-1ea18899)

use gwt_core::git::DivergenceStatus;
use ratatui::{prelude::*, widgets::*};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use crate::tui::components::LinkRegion;

/// File status in git (FR-012)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileStatus {
    #[default]
    Staged,
    Unstaged,
    Untracked,
}

impl FileStatus {
    /// Get display icon for status (FR-012)
    pub fn icon(&self) -> &'static str {
        match self {
            FileStatus::Staged => "[S]",
            FileStatus::Unstaged => "[U]",
            FileStatus::Untracked => "[?]",
        }
    }

    /// Get color for status
    pub fn color(&self) -> Color {
        match self {
            FileStatus::Staged => Color::Green,
            FileStatus::Unstaged => Color::Yellow,
            FileStatus::Untracked => Color::Gray,
        }
    }
}

/// File entry with diff information (FR-012, FR-020)
#[derive(Debug, Clone, Default)]
pub struct FileEntry {
    /// File path
    pub path: String,
    /// File status (staged/unstaged/untracked)
    pub status: FileStatus,
    /// Whether the file is binary
    pub is_binary: bool,
    /// Size change for binary files (FR-022)
    pub size_change: Option<i64>,
    /// Diff content (lazy loaded on expand)
    pub diff: Option<String>,
    /// Total diff line count (for truncation)
    pub diff_line_count: usize,
}

/// Commit entry (FR-014, FR-030)
#[derive(Debug, Clone, Default)]
pub struct CommitEntry {
    /// Short commit hash (7 chars)
    pub hash: String,
    /// Commit message subject (first line)
    pub subject: String,
    /// Full commit message body (for expand)
    #[allow(dead_code)]
    pub body: Option<String>,
    /// Author name
    pub author: String,
    /// Commit timestamp
    #[allow(dead_code)]
    pub timestamp: i64,
    /// Changed files (for expand)
    pub changed_files: Vec<String>,
}

/// Cached git data for a branch
#[derive(Debug, Clone, Default)]
pub struct GitViewData {
    /// File entries
    pub files: Vec<FileEntry>,
    /// Commit entries (last 5)
    pub commits: Vec<CommitEntry>,
    /// Cache creation time
    #[allow(dead_code)]
    pub cached_at: Option<Instant>,
}

/// Cache for all branches (FR-050)
#[derive(Debug, Clone, Default)]
pub struct GitViewCache {
    /// Branch name -> cached data
    data: HashMap<String, GitViewData>,
}

impl GitViewCache {
    /// Create new empty cache
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    /// Get cached data for branch
    pub fn get(&self, branch: &str) -> Option<&GitViewData> {
        self.data.get(branch)
    }

    /// Insert data for branch
    pub fn insert(&mut self, branch: String, data: GitViewData) {
        self.data.insert(branch, data);
    }

    /// Clear all cached data (FR-052)
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Check if branch is cached
    #[allow(dead_code)]
    pub fn contains(&self, branch: &str) -> bool {
        self.data.contains_key(branch)
    }
}

/// GitView screen state (SPEC-1ea18899)
#[derive(Debug, Clone)]
pub struct GitViewState {
    /// Target branch name
    pub branch_name: String,
    /// Worktree path (None if no worktree)
    #[allow(dead_code)]
    pub worktree_path: Option<PathBuf>,
    /// PR URL
    pub pr_url: Option<String>,
    /// PR title
    pub pr_title: Option<String>,
    /// PR number
    pub pr_number: Option<u64>,
    /// Divergence status (ahead/behind)
    pub divergence: DivergenceStatus,
    /// File entries
    pub files: Vec<FileEntry>,
    /// Number of visible files (for Show more) (FR-013)
    pub visible_file_count: usize,
    /// Commit entries (last 5)
    pub commits: Vec<CommitEntry>,
    /// Current selection index (unified across sections)
    pub selected_index: usize,
    /// Expanded items (index -> expanded)
    pub expanded: HashSet<usize>,
    /// PR link region for mouse click (FR-007)
    pub pr_link_region: Option<LinkRegion>,
    /// Loading state
    pub is_loading: bool,
    /// Whether branch has worktree
    pub has_worktree: bool,
}

impl Default for GitViewState {
    fn default() -> Self {
        Self {
            branch_name: String::new(),
            worktree_path: None,
            pr_url: None,
            pr_title: None,
            pr_number: None,
            divergence: DivergenceStatus::NoRemote,
            files: Vec::new(),
            visible_file_count: 20,
            commits: Vec::new(),
            selected_index: 0,
            expanded: HashSet::new(),
            pr_link_region: None,
            is_loading: false,
            has_worktree: false,
        }
    }
}

/// Maximum visible files before "Show more" (FR-013)
const MAX_VISIBLE_FILES: usize = 20;
/// Maximum diff lines per file (FR-021)
const MAX_DIFF_LINES: usize = 50;
/// Number of commits to show (FR-014)
const COMMIT_COUNT: usize = 5;

impl GitViewState {
    /// Create new GitViewState from branch info
    pub fn new(
        branch_name: String,
        worktree_path: Option<PathBuf>,
        pr_number: Option<u64>,
        pr_url: Option<String>,
        pr_title: Option<String>,
        divergence: DivergenceStatus,
    ) -> Self {
        let has_worktree = worktree_path.is_some();
        Self {
            branch_name,
            worktree_path,
            pr_url,
            pr_title,
            pr_number,
            divergence,
            files: Vec::new(),
            visible_file_count: MAX_VISIBLE_FILES,
            commits: Vec::new(),
            selected_index: 0,
            expanded: HashSet::new(),
            pr_link_region: None,
            is_loading: true,
            has_worktree,
        }
    }

    /// Load data from cache
    pub fn load_from_cache(&mut self, data: &GitViewData) {
        self.files = data.files.clone();
        self.commits = data.commits.clone();
        self.is_loading = false;
        // Reset selection to first item (FR-010a)
        self.selected_index = if self.pr_url.is_some() {
            0 // PR link
        } else if !self.files.is_empty() {
            if self.pr_url.is_some() {
                1
            } else {
                0
            }
        } else if !self.commits.is_empty() {
            self.files_section_end_index()
        } else {
            0
        };
    }

    /// Calculate total item count for navigation (FR-003)
    pub fn total_item_count(&self) -> usize {
        let pr_items = if self.pr_url.is_some() { 1 } else { 0 };
        let file_items = self.visible_file_count.min(self.files.len());
        let show_more = if self.files.len() > self.visible_file_count {
            1
        } else {
            0
        };
        let commit_items = self.commits.len();

        pr_items + file_items + show_more + commit_items
    }

    /// Get the index where files section ends
    fn files_section_end_index(&self) -> usize {
        let pr_items = if self.pr_url.is_some() { 1 } else { 0 };
        let file_items = self.visible_file_count.min(self.files.len());
        let show_more = if self.files.len() > self.visible_file_count {
            1
        } else {
            0
        };
        pr_items + file_items + show_more
    }

    /// Select next item (FR-003)
    pub fn select_next(&mut self) {
        let max_index = self.total_item_count();
        if max_index > 0 && self.selected_index < max_index - 1 {
            self.selected_index += 1;
        }
    }

    /// Select previous item (FR-003)
    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Toggle expand/collapse for current item (FR-004)
    pub fn toggle_expand(&mut self) {
        // Check if current selection is expandable
        if self.is_expandable(self.selected_index) {
            if self.expanded.contains(&self.selected_index) {
                self.expanded.remove(&self.selected_index);
            } else {
                self.expanded.insert(self.selected_index);
            }
        }
    }

    /// Check if item at index is expandable
    fn is_expandable(&self, index: usize) -> bool {
        let pr_items = if self.pr_url.is_some() { 1 } else { 0 };

        // PR link is not expandable
        if index < pr_items {
            return false;
        }

        let file_start = pr_items;
        let file_count = self.visible_file_count.min(self.files.len());
        let file_end = file_start + file_count;

        // Files are expandable
        if index >= file_start && index < file_end {
            return true;
        }

        // Show more is not expandable (it's a trigger)
        let show_more = if self.files.len() > self.visible_file_count {
            1
        } else {
            0
        };
        let show_more_index = file_end;
        if show_more > 0 && index == show_more_index {
            return false;
        }

        // Commits are expandable
        let commit_start = file_end + show_more;
        let commit_end = commit_start + self.commits.len();
        if index >= commit_start && index < commit_end {
            return true;
        }

        false
    }

    /// Check if current selection is "Show more"
    #[allow(dead_code)]
    pub fn is_show_more_selected(&self) -> bool {
        if self.files.len() <= self.visible_file_count {
            return false;
        }
        let pr_items = if self.pr_url.is_some() { 1 } else { 0 };
        let file_count = self.visible_file_count.min(self.files.len());
        let show_more_index = pr_items + file_count;
        self.selected_index == show_more_index
    }

    /// Expand "Show more" to show all files (FR-013)
    #[allow(dead_code)]
    pub fn show_more_files(&mut self) {
        self.visible_file_count = self.files.len();
    }

    /// Update PR info and keep selection stable when PR presence changes
    pub fn update_pr_info(
        &mut self,
        pr_number: Option<u64>,
        pr_title: Option<String>,
        pr_url: Option<String>,
    ) {
        let had_pr = self.pr_url.is_some();
        let had_items = self.total_item_count();
        let has_pr = pr_url.is_some();

        self.pr_number = pr_number;
        self.pr_title = pr_title;
        self.pr_url = pr_url;

        if had_pr != has_pr && had_items > 0 {
            if has_pr {
                self.selected_index = self.selected_index.saturating_add(1);
            } else {
                self.selected_index = self.selected_index.saturating_sub(1);
            }
        }
    }

    /// Check if current selection is PR link
    pub fn is_pr_link_selected(&self) -> bool {
        self.pr_url.is_some() && self.selected_index == 0
    }

    /// Get selected file index (if file is selected)
    #[allow(dead_code)]
    pub fn selected_file_index(&self) -> Option<usize> {
        let pr_items = if self.pr_url.is_some() { 1 } else { 0 };
        let file_start = pr_items;
        let file_count = self.visible_file_count.min(self.files.len());
        let file_end = file_start + file_count;

        if self.selected_index >= file_start && self.selected_index < file_end {
            Some(self.selected_index - file_start)
        } else {
            None
        }
    }

    /// Get selected commit index (if commit is selected)
    #[allow(dead_code)]
    pub fn selected_commit_index(&self) -> Option<usize> {
        let pr_items = if self.pr_url.is_some() { 1 } else { 0 };
        let file_count = self.visible_file_count.min(self.files.len());
        let show_more = if self.files.len() > self.visible_file_count {
            1
        } else {
            0
        };
        let commit_start = pr_items + file_count + show_more;
        let commit_end = commit_start + self.commits.len();

        if self.selected_index >= commit_start && self.selected_index < commit_end {
            Some(self.selected_index - commit_start)
        } else {
            None
        }
    }
}

/// Build git view data from worktree path
pub fn build_git_view_data(worktree_path: &PathBuf) -> GitViewData {
    let files = parse_git_status(worktree_path);
    let commits = parse_git_log(worktree_path, COMMIT_COUNT);

    GitViewData {
        files,
        commits,
        cached_at: Some(Instant::now()),
    }
}

/// Build git view data for branch without worktree (FR-053)
pub fn build_git_view_data_no_worktree(repo_root: &PathBuf, branch_name: &str) -> GitViewData {
    let commits = parse_git_log_for_branch(repo_root, branch_name, COMMIT_COUNT);

    GitViewData {
        files: Vec::new(),
        commits,
        cached_at: Some(Instant::now()),
    }
}

/// Parse git status output (porcelain format)
fn parse_git_status(worktree_path: &PathBuf) -> Vec<FileEntry> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "-uall"])
        .current_dir(worktree_path)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();

    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }

        let index_status = line.chars().next().unwrap_or(' ');
        let worktree_status = line.chars().nth(1).unwrap_or(' ');
        let path = line[3..].to_string();

        let status = if index_status != ' ' && index_status != '?' {
            FileStatus::Staged
        } else if worktree_status == '?' {
            FileStatus::Untracked
        } else {
            FileStatus::Unstaged
        };

        // Check if binary
        let is_binary = is_binary_file(worktree_path, &path);
        let size_change = if is_binary {
            get_file_size_change(worktree_path, &path)
        } else {
            None
        };

        // Get diff for non-binary files
        let (diff, diff_line_count) = if !is_binary && status != FileStatus::Untracked {
            let diff_content = get_file_diff(worktree_path, &path, status);
            let line_count = diff_content.lines().count();
            (Some(diff_content), line_count)
        } else {
            (None, 0)
        };

        files.push(FileEntry {
            path,
            status,
            is_binary,
            size_change,
            diff,
            diff_line_count,
        });
    }

    files
}

/// Check if file is binary
fn is_binary_file(worktree_path: &PathBuf, path: &str) -> bool {
    let output = Command::new("git")
        .args(["diff", "--numstat", "--", path])
        .current_dir(worktree_path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // Binary files show "-" for additions/deletions
            stdout.starts_with('-')
        }
        _ => false,
    }
}

/// Get file size change for binary files (FR-022)
fn get_file_size_change(worktree_path: &PathBuf, path: &str) -> Option<i64> {
    let file_path = worktree_path.join(path);
    let current_size = std::fs::metadata(&file_path).map(|m| m.len() as i64).ok();

    // Get old size from git
    let output = Command::new("git")
        .args(["cat-file", "-s", &format!("HEAD:{}", path)])
        .current_dir(worktree_path)
        .output();

    let old_size = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<i64>()
            .ok(),
        _ => None,
    };

    match (current_size, old_size) {
        (Some(curr), Some(old)) => Some(curr - old),
        (Some(curr), None) => Some(curr), // New file
        _ => None,
    }
}

/// Get diff content for a file
fn get_file_diff(worktree_path: &PathBuf, path: &str, status: FileStatus) -> String {
    let args = match status {
        FileStatus::Staged => vec!["diff", "--cached", "--", path],
        FileStatus::Unstaged => vec!["diff", "--", path],
        FileStatus::Untracked => return String::new(),
    };

    let output = Command::new("git")
        .args(&args)
        .current_dir(worktree_path)
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    }
}

/// Parse git log output
fn parse_git_log(worktree_path: &PathBuf, count: usize) -> Vec<CommitEntry> {
    let output = Command::new("git")
        .args([
            "log",
            &format!("-{}", count),
            "--format=%H|%s|%an|%at",
            "--name-only",
        ])
        .current_dir(worktree_path)
        .output();

    parse_git_log_output(output)
}

/// Parse git log for specific branch (without worktree)
fn parse_git_log_for_branch(
    repo_root: &PathBuf,
    branch_name: &str,
    count: usize,
) -> Vec<CommitEntry> {
    let output = Command::new("git")
        .args([
            "log",
            branch_name,
            &format!("-{}", count),
            "--format=%H|%s|%an|%at",
            "--name-only",
        ])
        .current_dir(repo_root)
        .output();

    parse_git_log_output(output)
}

/// Parse git log command output
fn parse_git_log_output(output: Result<std::process::Output, std::io::Error>) -> Vec<CommitEntry> {
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();
    let mut current_commit: Option<CommitEntry> = None;
    let mut current_files: Vec<String> = Vec::new();

    for line in stdout.lines() {
        if line.contains('|') {
            // New commit line
            if let Some(mut commit) = current_commit.take() {
                commit.changed_files = current_files.clone();
                commits.push(commit);
                current_files.clear();
            }

            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() >= 4 {
                current_commit = Some(CommitEntry {
                    hash: parts[0][..7.min(parts[0].len())].to_string(),
                    subject: parts[1].to_string(),
                    author: parts[2].to_string(),
                    timestamp: parts[3].parse().unwrap_or(0),
                    body: None,
                    changed_files: Vec::new(),
                });
            }
        } else if !line.is_empty() {
            // File line
            current_files.push(line.to_string());
        }
    }

    // Don't forget the last commit
    if let Some(mut commit) = current_commit {
        commit.changed_files = current_files;
        commits.push(commit);
    }

    commits
}

/// Format size change for display (FR-022)
fn format_size_change(bytes: i64) -> String {
    let abs_bytes = bytes.abs();
    let sign = if bytes >= 0 { "+" } else { "-" };

    if abs_bytes >= 1024 * 1024 {
        format!("{}{:.1}MB", sign, abs_bytes as f64 / (1024.0 * 1024.0))
    } else if abs_bytes >= 1024 {
        format!("{}{:.1}KB", sign, abs_bytes as f64 / 1024.0)
    } else {
        format!("{}{}B", sign, abs_bytes)
    }
}

/// Render GitView screen (SPEC-1ea18899)
pub fn render_git_view(state: &mut GitViewState, frame: &mut Frame, area: Rect) {
    // Vertical layout: Header -> Files -> Commits (FR-010)
    let chunks = Layout::vertical([
        Constraint::Length(4),  // Header
        Constraint::Min(8),     // Files section
        Constraint::Length(10), // Commits section
    ])
    .split(area);

    render_header(state, frame, chunks[0]);
    render_files_section(state, frame, chunks[1]);
    render_commits_section(state, frame, chunks[2]);
}

/// Render header section (FR-011)
fn render_header(state: &mut GitViewState, frame: &mut Frame, area: Rect) {
    state.pr_link_region = None;
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Branch: ", Style::default().fg(Color::Gray)),
            Span::styled(&state.branch_name, Style::default().fg(Color::Cyan).bold()),
        ]),
        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::Gray)),
            format_divergence(&state.divergence),
        ]),
    ];

    // PR line (FR-011a)
    if let Some(ref url) = state.pr_url {
        let pr_text = match (state.pr_number, state.pr_title.as_deref()) {
            (Some(number), Some(title)) => format!("#{} {}", number, title),
            (Some(number), None) => format!("#{}", number),
            (None, Some(title)) => title.to_string(),
            (None, None) => "Pull Request".to_string(),
        };
        let is_selected = state.is_pr_link_selected();
        let style = if is_selected {
            Style::default().fg(Color::Blue).underlined().bold()
        } else {
            Style::default().fg(Color::Blue).underlined()
        };

        lines.push(Line::from(vec![
            Span::styled("PR: ", Style::default().fg(Color::Gray)),
            Span::styled(pr_text.clone(), style),
            Span::styled(" ", Style::default()),
            Span::styled(
                if is_selected { "[Enter to open]" } else { "" },
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        // Store link region for mouse click (calculated after render)
        let pr_x = area.x + 4; // "PR: " length
        let pr_width = pr_text.chars().count() as u16;
        state.pr_link_region = Some(LinkRegion {
            area: Rect::new(pr_x, area.y + 2, pr_width, 1),
            url: url.clone(),
        });
    } else {
        lines.push(Line::from(vec![
            Span::styled("PR: ", Style::default().fg(Color::Gray)),
            Span::styled("No PR", Style::default().fg(Color::DarkGray)),
        ]));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" GitView ")
        .title_style(Style::default().fg(Color::White).bold());

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

/// Format divergence status for display
fn format_divergence(divergence: &DivergenceStatus) -> Span<'static> {
    match divergence {
        DivergenceStatus::UpToDate => Span::styled("Up to date", Style::default().fg(Color::Green)),
        DivergenceStatus::Ahead(n) => {
            Span::styled(format!("Ahead {}", n), Style::default().fg(Color::Cyan))
        }
        DivergenceStatus::Behind(n) => {
            Span::styled(format!("Behind {}", n), Style::default().fg(Color::Yellow))
        }
        DivergenceStatus::Diverged { ahead, behind } => Span::styled(
            format!("Diverged +{} -{}", ahead, behind),
            Style::default().fg(Color::Magenta),
        ),
        DivergenceStatus::NoRemote => {
            Span::styled("No remote", Style::default().fg(Color::DarkGray))
        }
    }
}

/// Render files section (FR-012, FR-013)
fn render_files_section(state: &mut GitViewState, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" Files ")
        .title_style(Style::default().fg(Color::White));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Handle no worktree case (FR-053)
    if !state.has_worktree {
        let no_worktree = Paragraph::new("No worktree - file changes not available")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(no_worktree, inner);
        return;
    }

    // Handle loading state
    if state.is_loading {
        let loading = Paragraph::new("Loading...").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(loading, inner);
        return;
    }

    // Handle empty files
    if state.files.is_empty() {
        let no_changes = Paragraph::new("No changes").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(no_changes, inner);
        return;
    }

    let pr_offset = if state.pr_url.is_some() { 1 } else { 0 };
    let visible_count = state.visible_file_count.min(state.files.len());
    let has_more = state.files.len() > state.visible_file_count;

    let mut items: Vec<ListItem> = Vec::new();

    for (i, file) in state.files.iter().take(visible_count).enumerate() {
        let global_index = pr_offset + i;
        let is_selected = state.selected_index == global_index;
        let is_expanded = state.expanded.contains(&global_index);

        // File line
        let icon = file.status.icon();
        let icon_color = file.status.color();

        let mut spans = vec![
            Span::styled(icon, Style::default().fg(icon_color)),
            Span::raw(" "),
            Span::styled(
                &file.path,
                if is_selected {
                    Style::default().bg(Color::DarkGray).fg(Color::White)
                } else {
                    Style::default()
                },
            ),
        ];

        // Binary file size change (FR-022)
        if file.is_binary {
            if let Some(change) = file.size_change {
                spans.push(Span::styled(
                    format!(" ({})", format_size_change(change)),
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                spans.push(Span::styled(
                    " (binary)",
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        items.push(ListItem::new(Line::from(spans)));

        // Expanded diff content (FR-020, FR-021)
        if is_expanded && !file.is_binary {
            if let Some(ref diff) = file.diff {
                let diff_lines: Vec<&str> = diff.lines().take(MAX_DIFF_LINES).collect();
                let truncated = file.diff_line_count > MAX_DIFF_LINES;

                for line in diff_lines {
                    let style = if line.starts_with('+') && !line.starts_with("+++") {
                        Style::default().fg(Color::Green)
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        Style::default().fg(Color::Red)
                    } else if line.starts_with("@@") {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };

                    items.push(ListItem::new(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(line, style),
                    ])));
                }

                if truncated {
                    items.push(ListItem::new(Line::from(Span::styled(
                        format!(
                            "    ... ({} more lines)",
                            file.diff_line_count - MAX_DIFF_LINES
                        ),
                        Style::default().fg(Color::DarkGray),
                    ))));
                }
            }
        }
    }

    // Show more item (FR-013)
    if has_more {
        let show_more_index = pr_offset + visible_count;
        let is_selected = state.selected_index == show_more_index;
        let remaining = state.files.len() - visible_count;

        items.push(ListItem::new(Line::from(Span::styled(
            format!("    Show more ({} remaining)", remaining),
            if is_selected {
                Style::default().bg(Color::DarkGray).fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Yellow)
            },
        ))));
    }

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Render commits section (FR-014)
fn render_commits_section(state: &mut GitViewState, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" Commits ")
        .title_style(Style::default().fg(Color::White));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Handle loading state
    if state.is_loading {
        let loading = Paragraph::new("Loading...").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(loading, inner);
        return;
    }

    // Handle empty commits
    if state.commits.is_empty() {
        let no_commits = Paragraph::new("No commits").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(no_commits, inner);
        return;
    }

    let pr_offset = if state.pr_url.is_some() { 1 } else { 0 };
    let file_count = state.visible_file_count.min(state.files.len());
    let show_more_offset = if state.files.len() > state.visible_file_count {
        1
    } else {
        0
    };
    let commit_start = pr_offset + file_count + show_more_offset;

    let mut items: Vec<ListItem> = Vec::new();

    for (i, commit) in state.commits.iter().enumerate() {
        let global_index = commit_start + i;
        let is_selected = state.selected_index == global_index;
        let is_expanded = state.expanded.contains(&global_index);

        // Commit line
        let style = if is_selected {
            Style::default().bg(Color::DarkGray).fg(Color::White)
        } else {
            Style::default()
        };

        items.push(ListItem::new(Line::from(vec![
            Span::styled(&commit.hash, Style::default().fg(Color::Yellow)),
            Span::raw(" "),
            Span::styled(&commit.subject, style),
            Span::styled(
                format!(" ({})", &commit.author),
                Style::default().fg(Color::DarkGray),
            ),
        ])));

        // Expanded commit details (FR-030)
        if is_expanded {
            // Changed files
            for file in &commit.changed_files {
                items.push(ListItem::new(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(file, Style::default().fg(Color::Cyan)),
                ])));
            }
        }
    }

    let list = List::new(items);
    frame.render_widget(list, inner);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn test_gitview_state_new() {
        let state = GitViewState::new(
            "feature/test".to_string(),
            Some(PathBuf::from("/tmp/test")),
            Some(1),
            Some("https://github.com/test/pr/1".to_string()),
            Some("Test PR".to_string()),
            DivergenceStatus::Ahead(3),
        );

        assert_eq!(state.branch_name, "feature/test");
        assert!(state.has_worktree);
        assert!(state.pr_url.is_some());
        assert!(state.is_loading);
    }

    #[test]
    fn test_gitview_state_select_next() {
        let mut state = GitViewState::default();
        state.files = vec![FileEntry::default(); 5];
        state.commits = vec![CommitEntry::default(); 3];
        state.visible_file_count = 5;

        state.select_next();
        assert_eq!(state.selected_index, 1);

        state.select_next();
        assert_eq!(state.selected_index, 2);
    }

    #[test]
    fn test_gitview_state_select_prev() {
        let mut state = GitViewState::default();
        state.files = vec![FileEntry::default(); 5];
        state.selected_index = 3;
        state.visible_file_count = 5;

        state.select_prev();
        assert_eq!(state.selected_index, 2);

        state.select_prev();
        assert_eq!(state.selected_index, 1);
    }

    #[test]
    fn test_gitview_state_toggle_expand() {
        let mut state = GitViewState::default();
        state.files = vec![FileEntry::default(); 3];
        state.visible_file_count = 3;
        state.selected_index = 1; // Second file

        state.toggle_expand();
        assert!(state.expanded.contains(&1));

        state.toggle_expand();
        assert!(!state.expanded.contains(&1));
    }

    #[test]
    fn test_gitview_cache_operations() {
        let mut cache = GitViewCache::new();

        assert!(!cache.contains("test-branch"));

        cache.insert(
            "test-branch".to_string(),
            GitViewData {
                files: vec![FileEntry::default()],
                commits: vec![CommitEntry::default()],
                cached_at: Some(Instant::now()),
            },
        );

        assert!(cache.contains("test-branch"));
        assert!(cache.get("test-branch").is_some());

        cache.clear();
        assert!(!cache.contains("test-branch"));
    }

    #[test]
    fn test_gitview_state_total_item_count() {
        let mut state = GitViewState::default();
        state.files = vec![FileEntry::default(); 25]; // More than MAX_VISIBLE_FILES
        state.commits = vec![CommitEntry::default(); 5];
        state.visible_file_count = 20;

        // 20 files + 1 show more + 5 commits = 26
        assert_eq!(state.total_item_count(), 26);

        // With PR link
        state.pr_url = Some("https://example.com".to_string());
        // 1 PR + 20 files + 1 show more + 5 commits = 27
        assert_eq!(state.total_item_count(), 27);
    }

    #[test]
    fn test_gitview_state_show_more() {
        let mut state = GitViewState::default();
        state.files = vec![FileEntry::default(); 30];
        state.visible_file_count = 20;

        assert!(state.files.len() > state.visible_file_count);

        state.show_more_files();
        assert_eq!(state.visible_file_count, 30);
    }

    #[test]
    fn test_file_status_icon() {
        assert_eq!(FileStatus::Staged.icon(), "[S]");
        assert_eq!(FileStatus::Unstaged.icon(), "[U]");
        assert_eq!(FileStatus::Untracked.icon(), "[?]");
    }

    #[test]
    fn test_format_size_change() {
        assert_eq!(format_size_change(100), "+100B");
        assert_eq!(format_size_change(-50), "-50B");
        assert_eq!(format_size_change(2048), "+2.0KB");
        assert_eq!(format_size_change(1048576), "+1.0MB");
    }

    // T201: PRリンクフォーカス状態のテスト
    #[test]
    fn test_gitview_pr_link_focus() {
        // With PR URL, index 0 should be PR link
        let state = GitViewState::new(
            "feature/test".to_string(),
            Some(PathBuf::from("/tmp/test")),
            Some(1),
            Some("https://github.com/test/pr/1".to_string()),
            Some("Test PR".to_string()),
            DivergenceStatus::Ahead(1),
        );
        assert!(state.is_pr_link_selected());

        // Without PR URL, index 0 should not be PR link
        let state = GitViewState::new(
            "feature/test".to_string(),
            Some(PathBuf::from("/tmp/test")),
            None,
            None,
            None,
            DivergenceStatus::NoRemote,
        );
        assert!(!state.is_pr_link_selected());
    }

    // T301: ワークツリーなしブランチの表示テスト
    #[test]
    fn test_gitview_no_worktree() {
        let state = GitViewState::new(
            "feature/no-worktree".to_string(),
            None, // No worktree
            None,
            None,
            None,
            DivergenceStatus::NoRemote,
        );

        assert!(!state.has_worktree);
        assert!(state.worktree_path.is_none());
        // Files should be empty for no-worktree branches
        assert!(state.files.is_empty());
    }

    #[test]
    fn test_gitview_update_pr_info_adds_offset() {
        let mut state = GitViewState::default();
        state.files = vec![FileEntry::default(); 1];
        state.selected_index = 0;

        state.update_pr_info(
            Some(42),
            Some("Test PR".to_string()),
            Some("https://example.com/pr/42".to_string()),
        );

        assert_eq!(state.selected_index, 1);
        assert!(state.pr_url.is_some());
    }

    #[test]
    fn test_gitview_update_pr_info_removes_offset() {
        let mut state = GitViewState::default();
        state.pr_url = Some("https://example.com/pr/1".to_string());
        state.selected_index = 1;

        state.update_pr_info(None, None, None);

        assert_eq!(state.selected_index, 0);
        assert!(state.pr_url.is_none());
    }
}
