//! Git View screen — file status, expandable diffs, and recent commits

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};
use gwt_git::commit::CommitEntry;
use gwt_git::diff::{FileEntry, FileStatus};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_DIFF_LINES: usize = 50;
const RECENT_COMMIT_COUNT: usize = 5;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// State for the Git View screen.
#[derive(Debug, Default)]
pub struct GitViewState {
    pub files: Vec<FileEntry>,
    pub selected: usize,
    pub expanded: Vec<bool>,
    pub diff_cache: Vec<Option<String>>,
    pub diff_scroll: Vec<usize>,
    pub commits: Vec<CommitEntry>,
    pub loading: bool,
    pub pr_url: Option<String>,
}

impl GitViewState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set files and reset selection/expansion state.
    pub fn set_files(&mut self, files: Vec<FileEntry>) {
        let len = files.len();
        self.files = files;
        self.expanded = vec![false; len];
        self.diff_cache = vec![None; len];
        self.diff_scroll = vec![0; len];
        self.loading = false;
        self.clamp_selection();
    }

    pub fn set_commits(&mut self, commits: Vec<CommitEntry>) {
        self.commits = commits;
    }

    pub fn clamp_selection(&mut self) {
        if self.files.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.files.len() {
            self.selected = self.files.len() - 1;
        }
    }

    pub fn select_next(&mut self) {
        if self.files.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.files.len() - 1);
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Toggle diff expansion for the selected file.
    pub fn toggle_expand(&mut self) {
        if self.selected < self.expanded.len() {
            self.expanded[self.selected] = !self.expanded[self.selected];
        }
    }

    /// Check if the selected file is expanded.
    pub fn is_expanded(&self, idx: usize) -> bool {
        self.expanded.get(idx).copied().unwrap_or(false)
    }

    /// Get cached diff for an index.
    pub fn cached_diff(&self, idx: usize) -> Option<&str> {
        self.diff_cache.get(idx).and_then(|d| d.as_deref())
    }

    /// Store a diff in the cache.
    pub fn cache_diff(&mut self, idx: usize, content: String) {
        if idx < self.diff_cache.len() {
            self.diff_cache[idx] = Some(content);
        }
    }

    /// Scroll diff down for selected file.
    pub fn scroll_diff_down(&mut self) {
        if self.selected < self.diff_scroll.len() {
            self.diff_scroll[self.selected] = self.diff_scroll[self.selected].saturating_add(1);
        }
    }

    /// Scroll diff up for selected file.
    pub fn scroll_diff_up(&mut self) {
        if self.selected < self.diff_scroll.len() {
            self.diff_scroll[self.selected] = self.diff_scroll[self.selected].saturating_sub(1);
        }
    }

    pub fn diff_scroll_offset(&self, idx: usize) -> usize {
        self.diff_scroll.get(idx).copied().unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Messages for the Git View screen.
#[derive(Debug)]
pub enum GitViewMessage {
    Refresh,
    SelectNext,
    SelectPrev,
    ToggleExpand,
    ScrollDiffDown,
    ScrollDiffUp,
    Loaded {
        files: Vec<FileEntry>,
        commits: Vec<CommitEntry>,
        pr_url: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

pub fn handle_key(state: &GitViewState, key: &KeyEvent) -> Option<GitViewMessage> {
    if state.is_expanded(state.selected) {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => return Some(GitViewMessage::ToggleExpand),
            KeyCode::Down | KeyCode::Char('j') => return Some(GitViewMessage::ScrollDiffDown),
            KeyCode::Up | KeyCode::Char('k') => return Some(GitViewMessage::ScrollDiffUp),
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(GitViewMessage::SelectNext),
        KeyCode::Char('k') | KeyCode::Up => Some(GitViewMessage::SelectPrev),
        KeyCode::Enter => Some(GitViewMessage::ToggleExpand),
        KeyCode::Char('r') => Some(GitViewMessage::Refresh),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

pub fn update(state: &mut GitViewState, msg: GitViewMessage) {
    match msg {
        GitViewMessage::SelectNext => state.select_next(),
        GitViewMessage::SelectPrev => state.select_prev(),
        GitViewMessage::ToggleExpand => state.toggle_expand(),
        GitViewMessage::ScrollDiffDown => state.scroll_diff_down(),
        GitViewMessage::ScrollDiffUp => state.scroll_diff_up(),
        GitViewMessage::Refresh => {
            state.loading = true;
        }
        GitViewMessage::Loaded {
            files,
            commits,
            pr_url,
        } => {
            state.set_files(files);
            state.set_commits(commits);
            state.pr_url = pr_url;
        }
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

pub fn render(state: &GitViewState, buf: &mut Buffer, area: Rect) {
    if area.height < 3 || area.width < 10 {
        return;
    }

    let header_height = 2u16;
    let commit_height = (state.commits.len() as u16 + 1).min(RECENT_COMMIT_COUNT as u16 + 1);
    let pr_height: u16 = if state.pr_url.is_some() { 1 } else { 0 };
    let bottom = commit_height + pr_height;
    let list_height = area.height.saturating_sub(header_height + bottom);

    let header_area = Rect::new(area.x, area.y, area.width, header_height);
    let list_area = Rect::new(area.x, area.y + header_height, area.width, list_height);
    let commit_area = Rect::new(
        area.x,
        area.y + header_height + list_height,
        area.width,
        commit_height,
    );
    let pr_area = Rect::new(
        area.x,
        area.y + header_height + list_height + commit_height,
        area.width,
        pr_height,
    );

    render_header(state, buf, header_area);
    render_file_list(state, buf, list_area);
    render_commits(state, buf, commit_area);
    if state.pr_url.is_some() {
        render_pr_link(state, buf, pr_area);
    }
}

fn render_header(state: &GitViewState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }

    let (mut staged, mut unstaged, mut untracked) = (0usize, 0usize, 0usize);
    for f in &state.files {
        match f.status {
            FileStatus::Staged => staged += 1,
            FileStatus::Unstaged => unstaged += 1,
            FileStatus::Untracked => untracked += 1,
        }
    }

    let title = if state.loading {
        " Git View (loading...)".to_string()
    } else {
        format!(" Git View  S:{staged}  U:{unstaged}  ?:{untracked}")
    };

    let title_span = Span::styled(title, Style::default().fg(Color::White).bold());
    buf.set_line(area.x, area.y, &Line::from(vec![title_span]), area.width);

    if area.height >= 2 {
        let hints = Line::from(vec![
            Span::styled(" [Enter] Expand", Style::default().fg(Color::DarkGray)),
            Span::styled("  ", Style::default()),
            Span::styled("[r] Refresh", Style::default().fg(Color::DarkGray)),
        ]);
        buf.set_line(area.x, area.y + 1, &hints, area.width);
    }
}

fn render_file_list(state: &GitViewState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 || state.files.is_empty() {
        if state.files.is_empty() && area.height > 0 {
            let msg = if state.loading {
                "Loading..."
            } else {
                "Working tree clean"
            };
            let para = Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray));
            let y = area.y + area.height / 2;
            let text_area = Rect::new(area.x, y, area.width, 1);
            ratatui::widgets::Widget::render(para, text_area, buf);
        }
        return;
    }

    let mut y = area.y;
    let max_y = area.y + area.height;

    for (idx, file) in state.files.iter().enumerate() {
        if y >= max_y {
            break;
        }

        let is_selected = idx == state.selected;
        render_file_row(file, is_selected, buf, area.x, y, area.width);
        y += 1;

        // Render expanded diff
        if state.is_expanded(idx) {
            if let Some(diff) = state.cached_diff(idx) {
                let scroll = state.diff_scroll_offset(idx);
                let max_lines = MAX_DIFF_LINES.min((max_y - y) as usize);
                for line in diff.lines().skip(scroll).take(max_lines) {
                    if y >= max_y {
                        break;
                    }
                    let color = diff_line_color(line);
                    let span = Span::styled(
                        format!("  {}", truncate_line(line, area.width.saturating_sub(2) as usize)),
                        Style::default().fg(color),
                    );
                    buf.set_line(area.x, y, &Line::from(vec![span]), area.width);
                    y += 1;
                }
            } else if y < max_y {
                let span = Span::styled("  (loading diff...)", Style::default().fg(Color::DarkGray));
                buf.set_line(area.x, y, &Line::from(vec![span]), area.width);
                y += 1;
            }
        }
    }
}

fn render_file_row(
    file: &FileEntry,
    is_selected: bool,
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
) {
    let status_tag = match file.status {
        FileStatus::Staged => "[S]",
        FileStatus::Unstaged => "[U]",
        FileStatus::Untracked => "[?]",
    };
    let status_color = match file.status {
        FileStatus::Staged => Color::Green,
        FileStatus::Unstaged => Color::Yellow,
        FileStatus::Untracked => Color::DarkGray,
    };

    let sel = if is_selected { ">" } else { " " };
    let sel_style = if is_selected {
        Style::default().fg(Color::White).bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let path_str = file.path.to_string_lossy();
    let spans = vec![
        Span::styled(sel, sel_style),
        Span::styled(format!(" {status_tag}"), Style::default().fg(status_color)),
        Span::styled(format!(" {path_str}"), Style::default().fg(Color::White)),
    ];

    if is_selected {
        for col in x..x + width {
            buf[(col, y)].set_style(Style::default().bg(Color::Rgb(40, 40, 60)));
        }
    }

    buf.set_line(x, y, &Line::from(spans), width);
}

fn render_commits(state: &GitViewState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }

    let header = Span::styled(
        " Recent Commits",
        Style::default().fg(Color::White).bold(),
    );
    buf.set_line(area.x, area.y, &Line::from(vec![header]), area.width);

    for (i, commit) in state.commits.iter().take(RECENT_COMMIT_COUNT).enumerate() {
        let y = area.y + 1 + i as u16;
        if y >= area.y + area.height {
            break;
        }
        let line = Line::from(vec![
            Span::styled(format!("  {}", commit.hash), Style::default().fg(Color::Yellow)),
            Span::styled(
                format!(" {}", truncate_line(&commit.subject, area.width.saturating_sub(12) as usize)),
                Style::default().fg(Color::White),
            ),
        ]);
        buf.set_line(area.x, y, &line, area.width);
    }
}

fn render_pr_link(state: &GitViewState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }
    if let Some(ref url) = state.pr_url {
        let line = Line::from(vec![
            Span::styled(" PR: ", Style::default().fg(Color::Cyan).bold()),
            Span::styled(url.as_str(), Style::default().fg(Color::Blue)),
        ]);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn diff_line_color(line: &str) -> Color {
    if line.starts_with('+') && !line.starts_with("+++") {
        Color::Green
    } else if line.starts_with('-') && !line.starts_with("---") {
        Color::Red
    } else if line.starts_with("@@") {
        Color::Cyan
    } else {
        Color::DarkGray
    }
}

fn truncate_line(line: &str, max_len: usize) -> String {
    if line.chars().count() > max_len && max_len > 3 {
        let truncated: String = line.chars().take(max_len - 3).collect();
        format!("{truncated}...")
    } else {
        line.to_string()
    }
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

pub fn load_git_view(repo_path: &Path) -> (Vec<FileEntry>, Vec<CommitEntry>) {
    let files = gwt_git::diff::get_status(repo_path).unwrap_or_default();
    let commits = gwt_git::commit::recent_commits(repo_path, RECENT_COMMIT_COUNT).unwrap_or_default();
    (files, commits)
}

/// Load diff content for a specific file entry.
pub fn load_diff_content(repo_path: &Path, file: &FileEntry) -> String {
    file.diff_content(repo_path)
        .unwrap_or_else(|e| format!("(error loading diff: {e})"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn make_file(path: &str, status: FileStatus) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            status,
        }
    }

    fn make_commit(hash: &str, subject: &str) -> CommitEntry {
        CommitEntry {
            hash: hash.to_string(),
            subject: subject.to_string(),
            author: "Test".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    // -- State tests --

    #[test]
    fn set_files_resets_expansion() {
        let mut state = GitViewState::new();
        state.set_files(vec![
            make_file("a.rs", FileStatus::Staged),
            make_file("b.rs", FileStatus::Unstaged),
        ]);
        assert_eq!(state.files.len(), 2);
        assert_eq!(state.expanded.len(), 2);
        assert!(!state.expanded[0]);
        assert!(!state.expanded[1]);
    }

    #[test]
    fn select_next_prev() {
        let mut state = GitViewState::new();
        state.set_files(vec![
            make_file("a.rs", FileStatus::Staged),
            make_file("b.rs", FileStatus::Unstaged),
            make_file("c.rs", FileStatus::Untracked),
        ]);
        assert_eq!(state.selected, 0);
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_next();
        assert_eq!(state.selected, 2);
        state.select_next();
        assert_eq!(state.selected, 2); // clamped
        state.select_prev();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn toggle_expand() {
        let mut state = GitViewState::new();
        state.set_files(vec![make_file("a.rs", FileStatus::Staged)]);
        assert!(!state.is_expanded(0));
        state.toggle_expand();
        assert!(state.is_expanded(0));
        state.toggle_expand();
        assert!(!state.is_expanded(0));
    }

    #[test]
    fn diff_cache() {
        let mut state = GitViewState::new();
        state.set_files(vec![make_file("a.rs", FileStatus::Staged)]);
        assert!(state.cached_diff(0).is_none());
        state.cache_diff(0, "+new line\n-old line".to_string());
        assert_eq!(state.cached_diff(0), Some("+new line\n-old line"));
    }

    #[test]
    fn diff_scroll() {
        let mut state = GitViewState::new();
        state.set_files(vec![make_file("a.rs", FileStatus::Staged)]);
        assert_eq!(state.diff_scroll_offset(0), 0);
        state.scroll_diff_down();
        assert_eq!(state.diff_scroll_offset(0), 1);
        state.scroll_diff_up();
        assert_eq!(state.diff_scroll_offset(0), 0);
        state.scroll_diff_up();
        assert_eq!(state.diff_scroll_offset(0), 0); // clamped
    }

    #[test]
    fn clamp_selection_empty() {
        let mut state = GitViewState::new();
        state.selected = 10;
        state.set_files(vec![]);
        assert_eq!(state.selected, 0);
    }

    // -- Key handling tests --

    #[test]
    fn handle_key_navigation() {
        let state = GitViewState::new();
        let key_j = KeyEvent::new(KeyCode::Char('j'), crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_j),
            Some(GitViewMessage::SelectNext)
        ));

        let key_k = KeyEvent::new(KeyCode::Char('k'), crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_k),
            Some(GitViewMessage::SelectPrev)
        ));
    }

    #[test]
    fn handle_key_expand() {
        let state = GitViewState::new();
        let key_enter = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_enter),
            Some(GitViewMessage::ToggleExpand)
        ));
    }

    #[test]
    fn handle_key_refresh() {
        let state = GitViewState::new();
        let key_r = KeyEvent::new(KeyCode::Char('r'), crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_r),
            Some(GitViewMessage::Refresh)
        ));
    }

    #[test]
    fn handle_key_expanded_mode() {
        let mut state = GitViewState::new();
        state.set_files(vec![make_file("a.rs", FileStatus::Staged)]);
        state.expanded[0] = true;

        let key_esc = KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_esc),
            Some(GitViewMessage::ToggleExpand)
        ));

        let key_j = KeyEvent::new(KeyCode::Char('j'), crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_j),
            Some(GitViewMessage::ScrollDiffDown)
        ));
    }

    // -- Update tests --

    #[test]
    fn update_loaded() {
        let mut state = GitViewState::new();
        state.loading = true;
        update(
            &mut state,
            GitViewMessage::Loaded {
                files: vec![make_file("a.rs", FileStatus::Staged)],
                commits: vec![make_commit("abc", "init")],
                pr_url: Some("https://github.com/owner/repo/pull/1".to_string()),
            },
        );
        assert!(!state.loading);
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.commits.len(), 1);
        assert!(state.pr_url.is_some());
    }

    #[test]
    fn update_refresh() {
        let mut state = GitViewState::new();
        update(&mut state, GitViewMessage::Refresh);
        assert!(state.loading);
    }

    // -- Render tests --

    #[test]
    fn render_empty_state() {
        let state = GitViewState::new();
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
    fn render_with_files() {
        let mut state = GitViewState::new();
        state.set_files(vec![
            make_file("src/main.rs", FileStatus::Staged),
            make_file("src/lib.rs", FileStatus::Unstaged),
            make_file("new.txt", FileStatus::Untracked),
        ]);
        state.set_commits(vec![
            make_commit("abc1234", "feat: add feature"),
            make_commit("def5678", "fix: bug"),
        ]);
        state.pr_url = Some("https://github.com/owner/repo/pull/42".to_string());

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
    fn render_small_area_does_not_panic() {
        let state = GitViewState::new();
        let backend = TestBackend::new(5, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_with_expanded_diff() {
        let mut state = GitViewState::new();
        state.set_files(vec![make_file("a.rs", FileStatus::Staged)]);
        state.expanded[0] = true;
        state.cache_diff(0, "+added\n-removed\n context".to_string());

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    // -- Helper tests --

    #[test]
    fn diff_line_color_cases() {
        assert_eq!(diff_line_color("+added"), Color::Green);
        assert_eq!(diff_line_color("+++"), Color::DarkGray);
        assert_eq!(diff_line_color("-removed"), Color::Red);
        assert_eq!(diff_line_color("---"), Color::DarkGray);
        assert_eq!(diff_line_color("@@ -1,3 +1,3 @@"), Color::Cyan);
        assert_eq!(diff_line_color(" context"), Color::DarkGray);
    }

    #[test]
    fn truncate_line_long() {
        let long = "a".repeat(100);
        let result = truncate_line(&long, 20);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 20);
    }

    #[test]
    fn truncate_line_short() {
        let result = truncate_line("short", 20);
        assert_eq!(result, "short");
    }

    #[test]
    fn load_git_view_in_test_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(path)
            .output()
            .unwrap();

        let (files, commits) = load_git_view(path);
        assert!(files.is_empty());
        assert_eq!(commits.len(), 1);
    }
}
