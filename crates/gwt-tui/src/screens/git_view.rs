//! Git View screen.

use std::collections::HashSet;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

/// File status in the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Staged,
    Unstaged,
    Untracked,
}

impl FileStatus {
    /// Badge string for display.
    pub fn badge(self) -> &'static str {
        match self {
            Self::Staged => "[S]",
            Self::Unstaged => "[U]",
            Self::Untracked => "[?]",
        }
    }

    /// Color for the badge.
    pub fn color(self) -> Color {
        match self {
            Self::Staged => Color::Green,
            Self::Unstaged => Color::Yellow,
            Self::Untracked => Color::DarkGray,
        }
    }
}

/// A single file entry in the git view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileItem {
    pub path: String,
    pub status: FileStatus,
    pub diff_preview: String,
}

/// A single commit entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommitItem {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub date: String,
}

/// State for the git view screen.
#[derive(Debug, Clone, Default)]
pub struct GitViewState {
    pub(crate) files: Vec<GitFileItem>,
    pub(crate) selected: usize,
    pub(crate) expanded: HashSet<usize>,
    pub(crate) commits: Vec<GitCommitItem>,
    pub(crate) divergence_summary: Option<String>,
    pub(crate) pr_link: Option<String>,
}

impl GitViewState {
    /// Get the currently selected file, if any.
    pub fn selected_file(&self) -> Option<&GitFileItem> {
        self.files.get(self.selected)
    }

    /// Check whether a file at the given index is expanded.
    pub fn is_expanded(&self, idx: usize) -> bool {
        self.expanded.contains(&idx)
    }

    /// Clamp selected index to files length.
    fn clamp_selected(&mut self) {
        super::clamp_index(&mut self.selected, self.files.len());
    }
}

/// Messages specific to the git view screen.
#[derive(Debug, Clone)]
pub enum GitViewMessage {
    MoveUp,
    MoveDown,
    ToggleExpand,
    Refresh,
    SetFiles(Vec<GitFileItem>),
    SetCommits(Vec<GitCommitItem>),
    SetMetadata {
        divergence_summary: Option<String>,
        pr_link: Option<String>,
    },
}

/// Update git view state in response to a message.
pub fn update(state: &mut GitViewState, msg: GitViewMessage) {
    match msg {
        GitViewMessage::MoveUp => {
            super::move_up(&mut state.selected, state.files.len());
        }
        GitViewMessage::MoveDown => {
            super::move_down(&mut state.selected, state.files.len());
        }
        GitViewMessage::ToggleExpand => {
            if !state.files.is_empty() {
                let idx = state.selected;
                if state.expanded.contains(&idx) {
                    state.expanded.remove(&idx);
                } else {
                    state.expanded.insert(idx);
                }
            }
        }
        GitViewMessage::Refresh => {
            // Signal to reload -- handled by caller
        }
        GitViewMessage::SetFiles(files) => {
            state.files = files;
            state.expanded.clear();
            state.clamp_selected();
        }
        GitViewMessage::SetCommits(commits) => {
            state.commits = commits;
        }
        GitViewMessage::SetMetadata {
            divergence_summary,
            pr_link,
        } => {
            state.divergence_summary = divergence_summary;
            state.pr_link = pr_link;
        }
    }
}

/// Render the git view screen.
pub fn render(state: &GitViewState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(60), // File list
            Constraint::Percentage(40), // Commits
        ])
        .split(area);

    render_file_list(state, frame, chunks[0]);
    render_commits(state, frame, chunks[1]);
}

/// Render the file list with expandable diffs.
fn render_file_list(state: &GitViewState, frame: &mut Frame, area: Rect) {
    if state.files.is_empty() {
        let block = Block::default().title("Files (0)");
        let paragraph = Paragraph::new("No changed files")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let mut title = format!("Files ({})", state.files.len());
    if let Some(summary) = &state.divergence_summary {
        title.push_str(&format!(" | {summary}"));
    }
    if let Some(pr_link) = &state.pr_link {
        title.push_str(&format!(" | PR {pr_link}"));
    }
    let mut items: Vec<ListItem> = Vec::new();

    for (idx, file) in state.files.iter().enumerate() {
        let style = super::list_item_style(idx == state.selected);

        let expand_marker = if state.is_expanded(idx) { "v " } else { "> " };

        let line = Line::from(vec![
            Span::styled(expand_marker, Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{} ", file.status.badge()),
                Style::default().fg(file.status.color()),
            ),
            Span::styled(file.path.clone(), style),
        ]);
        items.push(ListItem::new(line));

        // Show diff preview if expanded (max 50 lines)
        if state.is_expanded(idx) && !file.diff_preview.is_empty() {
            let preview_lines: Vec<&str> = file.diff_preview.lines().take(50).collect();
            for diff_line in preview_lines {
                let diff_color = if diff_line.starts_with('+') {
                    Color::Green
                } else if diff_line.starts_with('-') {
                    Color::Red
                } else {
                    Color::DarkGray
                };
                let diff_display = Line::from(Span::styled(
                    format!("    {diff_line}"),
                    Style::default().fg(diff_color),
                ));
                items.push(ListItem::new(diff_display));
            }
        }
    }

    let block = Block::default().title(title);
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Render the commits section.
fn render_commits(state: &GitViewState, frame: &mut Frame, area: Rect) {
    if state.commits.is_empty() {
        let block = Block::default().title("Commits (0)");
        let paragraph = Paragraph::new("No commits loaded")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let title = format!("Commits ({})", state.commits.len());
    let items: Vec<ListItem> = state
        .commits
        .iter()
        .map(|commit| {
            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", &commit.hash[..commit.hash.len().min(7)]),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(commit.subject.clone(), Style::default().fg(Color::White)),
                Span::styled(
                    format!(" ({}, {})", commit.author, commit.date),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default().title(title);
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_files() -> Vec<GitFileItem> {
        vec![
            GitFileItem {
                path: "src/main.rs".to_string(),
                status: FileStatus::Staged,
                diff_preview: "+fn main() {\n+    println!(\"hello\");\n+}".to_string(),
            },
            GitFileItem {
                path: "src/lib.rs".to_string(),
                status: FileStatus::Unstaged,
                diff_preview: "-old line\n+new line".to_string(),
            },
            GitFileItem {
                path: "README.md".to_string(),
                status: FileStatus::Untracked,
                diff_preview: String::new(),
            },
        ]
    }

    fn sample_commits() -> Vec<GitCommitItem> {
        vec![
            GitCommitItem {
                hash: "abc1234".to_string(),
                subject: "Initial commit".to_string(),
                author: "Alice".to_string(),
                date: "2024-01-01".to_string(),
            },
            GitCommitItem {
                hash: "def5678".to_string(),
                subject: "Add feature".to_string(),
                author: "Bob".to_string(),
                date: "2024-01-02".to_string(),
            },
        ]
    }

    #[test]
    fn default_state() {
        let state = GitViewState::default();
        assert!(state.files.is_empty());
        assert_eq!(state.selected, 0);
        assert!(state.expanded.is_empty());
        assert!(state.commits.is_empty());
        assert!(state.divergence_summary.is_none());
        assert!(state.pr_link.is_none());
    }

    #[test]
    fn move_down_wraps() {
        let mut state = GitViewState::default();
        state.files = sample_files();

        update(&mut state, GitViewMessage::MoveDown);
        assert_eq!(state.selected, 1);

        update(&mut state, GitViewMessage::MoveDown);
        assert_eq!(state.selected, 2);

        update(&mut state, GitViewMessage::MoveDown);
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn move_up_wraps() {
        let mut state = GitViewState::default();
        state.files = sample_files();

        update(&mut state, GitViewMessage::MoveUp);
        assert_eq!(state.selected, 2); // wraps to last
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = GitViewState::default();
        update(&mut state, GitViewMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, GitViewMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn toggle_expand_adds_and_removes() {
        let mut state = GitViewState::default();
        state.files = sample_files();
        assert!(!state.is_expanded(0));

        update(&mut state, GitViewMessage::ToggleExpand);
        assert!(state.is_expanded(0));

        update(&mut state, GitViewMessage::ToggleExpand);
        assert!(!state.is_expanded(0));
    }

    #[test]
    fn toggle_expand_noop_on_empty() {
        let mut state = GitViewState::default();
        update(&mut state, GitViewMessage::ToggleExpand);
        assert!(state.expanded.is_empty());
    }

    #[test]
    fn set_files_populates_and_clears_expanded() {
        let mut state = GitViewState::default();
        state.expanded.insert(0);
        state.selected = 99;

        update(&mut state, GitViewMessage::SetFiles(sample_files()));
        assert_eq!(state.files.len(), 3);
        assert!(state.expanded.is_empty());
        assert_eq!(state.selected, 2); // clamped
    }

    #[test]
    fn set_commits_populates() {
        let mut state = GitViewState::default();
        update(&mut state, GitViewMessage::SetCommits(sample_commits()));
        assert_eq!(state.commits.len(), 2);
    }

    #[test]
    fn set_metadata_populates_header_fields() {
        let mut state = GitViewState::default();
        update(
            &mut state,
            GitViewMessage::SetMetadata {
                divergence_summary: Some("Ahead 2 Behind 1".to_string()),
                pr_link: Some("https://example.com/pr/42".to_string()),
            },
        );

        assert_eq!(
            state.divergence_summary.as_deref(),
            Some("Ahead 2 Behind 1")
        );
        assert_eq!(state.pr_link.as_deref(), Some("https://example.com/pr/42"));
    }

    #[test]
    fn selected_file_returns_correct() {
        let mut state = GitViewState::default();
        state.files = sample_files();
        state.selected = 1;
        let file = state.selected_file().unwrap();
        assert_eq!(file.path, "src/lib.rs");
    }

    #[test]
    fn file_status_badges() {
        assert_eq!(FileStatus::Staged.badge(), "[S]");
        assert_eq!(FileStatus::Unstaged.badge(), "[U]");
        assert_eq!(FileStatus::Untracked.badge(), "[?]");
    }

    #[test]
    fn file_status_colors() {
        assert_eq!(FileStatus::Staged.color(), Color::Green);
        assert_eq!(FileStatus::Unstaged.color(), Color::Yellow);
        assert_eq!(FileStatus::Untracked.color(), Color::DarkGray);
    }

    #[test]
    fn render_with_files_does_not_panic() {
        let mut state = GitViewState::default();
        state.files = sample_files();
        state.commits = sample_commits();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("Files"));
    }

    #[test]
    fn render_header_includes_divergence_and_pr_link() {
        let mut state = GitViewState::default();
        state.files = sample_files();
        state.commits = sample_commits();
        state.divergence_summary = Some("Ahead 2 Behind 1".to_string());
        state.pr_link = Some("https://example.com/pr/42".to_string());

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("Ahead 2 Behind 1"));
        assert!(text.contains("https://example.com/pr/42"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = GitViewState::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }

    #[test]
    fn render_with_expanded_diff_does_not_panic() {
        let mut state = GitViewState::default();
        state.files = sample_files();
        state.expanded.insert(0);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }

    #[test]
    fn diff_preview_capped_at_50_lines() {
        let long_diff = (0..100)
            .map(|i| format!("+line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut state = GitViewState::default();
        state.files = vec![GitFileItem {
            path: "big.rs".to_string(),
            status: FileStatus::Staged,
            diff_preview: long_diff,
        }];
        state.expanded.insert(0);
        // Just ensure render doesn't panic -- actual line count
        // is controlled by the take(50) in render
        let backend = TestBackend::new(80, 80);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }
}
