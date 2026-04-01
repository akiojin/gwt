//! Versions tab — list git tags (releases)

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// A single version/tag entry.
#[derive(Debug, Clone)]
pub struct VersionTag {
    pub name: String,
    pub date: String,
    pub message: String,
}

/// State for the Versions screen.
#[derive(Debug, Default)]
pub struct VersionsState {
    pub tags: Vec<VersionTag>,
    pub selected: usize,
    pub scroll: usize,
    pub detail_mode: bool,
    pub detail_content: String,
    pub detail_scroll: usize,
}

impl VersionsState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Messages for the Versions screen.
#[derive(Debug)]
pub enum VersionsMessage {
    SelectPrev,
    SelectNext,
    OpenDetail,
    CloseDetail,
    ScrollDetailUp,
    ScrollDetailDown,
}

/// Handle key input for the Versions screen.
pub fn handle_key(state: &VersionsState, key: &KeyEvent) -> Option<VersionsMessage> {
    if state.detail_mode {
        match key.code {
            KeyCode::Esc => Some(VersionsMessage::CloseDetail),
            KeyCode::Up | KeyCode::Char('k') => Some(VersionsMessage::ScrollDetailUp),
            KeyCode::Down | KeyCode::Char('j') => Some(VersionsMessage::ScrollDetailDown),
            _ => None,
        }
    } else {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(VersionsMessage::SelectPrev),
            KeyCode::Down | KeyCode::Char('j') => Some(VersionsMessage::SelectNext),
            KeyCode::Enter => Some(VersionsMessage::OpenDetail),
            _ => None,
        }
    }
}

/// Apply a VersionsMessage to state.
pub fn update(state: &mut VersionsState, msg: VersionsMessage) {
    match msg {
        VersionsMessage::SelectPrev => {
            state.selected = state.selected.saturating_sub(1);
        }
        VersionsMessage::SelectNext => {
            let max = state.tags.len().saturating_sub(1);
            if state.selected < max {
                state.selected += 1;
            }
        }
        VersionsMessage::OpenDetail => {
            if !state.tags.is_empty() {
                state.detail_mode = true;
                state.detail_scroll = 0;
                // detail_content is populated externally (app.rs)
            }
        }
        VersionsMessage::CloseDetail => {
            state.detail_mode = false;
            state.detail_content.clear();
            state.detail_scroll = 0;
        }
        VersionsMessage::ScrollDetailUp => {
            state.detail_scroll = state.detail_scroll.saturating_sub(1);
        }
        VersionsMessage::ScrollDetailDown => {
            state.detail_scroll = state.detail_scroll.saturating_add(1);
        }
    }
}

/// Render the Versions screen.
pub fn render(state: &VersionsState, buf: &mut Buffer, area: Rect) {
    if area.height < 3 {
        return;
    }

    if state.detail_mode {
        render_detail(state, buf, area);
        return;
    }

    let layout = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(1),   // List
    ])
    .split(area);

    // Header
    let count = state.tags.len();
    let header = format!(" Versions ({count})");
    let header_span = Span::styled(header, Style::default().fg(Color::Cyan).bold());
    buf.set_span(layout[0].x, layout[0].y, &header_span, layout[0].width);

    // List
    if state.tags.is_empty() {
        let msg = Paragraph::new("No tags found")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        let y = layout[1].y + layout[1].height / 2;
        let text_area = Rect::new(layout[1].x, y, layout[1].width, 1);
        ratatui::widgets::Widget::render(msg, text_area, buf);
        return;
    }

    let list_area = layout[1];
    let max_rows = list_area.height as usize;

    let offset = if state.selected >= max_rows {
        state.selected - max_rows + 1
    } else {
        0
    };

    for (i, tag) in state.tags.iter().skip(offset).take(max_rows).enumerate() {
        let y = list_area.y + i as u16;
        let is_selected = (i + offset) == state.selected;

        let marker = if is_selected { ">" } else { " " };
        let line = format!(
            " {marker} {:<20} {:<12} {}",
            tag.name, tag.date, tag.message
        );

        let style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default()
        };

        let span = Span::styled(line, style);
        buf.set_span(list_area.x, y, &span, list_area.width);
    }
}

fn render_detail(state: &VersionsState, buf: &mut Buffer, area: Rect) {
    let tag_name = state
        .tags
        .get(state.selected)
        .map(|t| t.name.as_str())
        .unwrap_or("?");

    let layout = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(1),   // Content
    ])
    .split(area);

    // Header
    let header = format!(" {tag_name}  [Esc] Back");
    let header_span = Span::styled(header, Style::default().fg(Color::Cyan).bold());
    buf.set_span(layout[0].x, layout[0].y, &header_span, layout[0].width);

    // Content
    let lines: Vec<&str> = state.detail_content.lines().collect();
    let content_area = layout[1];
    let max_rows = content_area.height as usize;
    let scroll = state.detail_scroll.min(lines.len().saturating_sub(1));

    for (i, line) in lines.iter().skip(scroll).take(max_rows).enumerate() {
        let y = content_area.y + i as u16;
        let span = Span::styled(*line, Style::default().fg(Color::White));
        buf.set_span(content_area.x, y, &span, content_area.width);
    }
}

/// Load version tags from git.
pub fn load_tags(repo_root: &std::path::Path) -> Vec<VersionTag> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["tag", "-l", "--sort=-creatordate", "--format=%(refname:short)|%(creatordate:short)|%(subject)"])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            VersionTag {
                name: parts.first().copied().unwrap_or("").to_string(),
                date: parts.get(1).copied().unwrap_or("").to_string(),
                message: parts.get(2).copied().unwrap_or("").to_string(),
            }
        })
        .collect()
}

/// Load tag detail via git show.
pub fn load_tag_detail(repo_root: &std::path::Path, tag_name: &str) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["show", "--stat", tag_name])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => format!("(Failed to load details for tag '{tag_name}')"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn sample_tags() -> Vec<VersionTag> {
        vec![
            VersionTag {
                name: "v1.0.0".into(),
                date: "2024-01-01".into(),
                message: "Initial release".into(),
            },
            VersionTag {
                name: "v1.1.0".into(),
                date: "2024-02-01".into(),
                message: "Feature update".into(),
            },
            VersionTag {
                name: "v2.0.0".into(),
                date: "2024-03-01".into(),
                message: "Major release".into(),
            },
        ]
    }

    #[test]
    fn new_state_is_empty() {
        let s = VersionsState::new();
        assert!(s.tags.is_empty());
        assert_eq!(s.selected, 0);
        assert!(!s.detail_mode);
    }

    #[test]
    fn select_prev_next() {
        let mut s = VersionsState::new();
        s.tags = sample_tags();
        update(&mut s, VersionsMessage::SelectNext);
        assert_eq!(s.selected, 1);
        update(&mut s, VersionsMessage::SelectPrev);
        assert_eq!(s.selected, 0);
        // Saturate at zero
        update(&mut s, VersionsMessage::SelectPrev);
        assert_eq!(s.selected, 0);
        // Saturate at max
        s.selected = 2;
        update(&mut s, VersionsMessage::SelectNext);
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn open_close_detail() {
        let mut s = VersionsState::new();
        s.tags = sample_tags();
        update(&mut s, VersionsMessage::OpenDetail);
        assert!(s.detail_mode);
        update(&mut s, VersionsMessage::CloseDetail);
        assert!(!s.detail_mode);
    }

    #[test]
    fn detail_scroll() {
        let mut s = VersionsState::new();
        s.tags = sample_tags();
        s.detail_mode = true;
        s.detail_content = "line1\nline2\nline3".to_string();
        update(&mut s, VersionsMessage::ScrollDetailDown);
        assert_eq!(s.detail_scroll, 1);
        update(&mut s, VersionsMessage::ScrollDetailUp);
        assert_eq!(s.detail_scroll, 0);
        update(&mut s, VersionsMessage::ScrollDetailUp);
        assert_eq!(s.detail_scroll, 0);
    }

    #[test]
    fn handle_key_list_mode() {
        let s = VersionsState::new();
        assert!(matches!(handle_key(&s, &key(KeyCode::Up)), Some(VersionsMessage::SelectPrev)));
        assert!(matches!(handle_key(&s, &key(KeyCode::Down)), Some(VersionsMessage::SelectNext)));
        assert!(matches!(handle_key(&s, &key(KeyCode::Enter)), Some(VersionsMessage::OpenDetail)));
        assert!(handle_key(&s, &key(KeyCode::Char('z'))).is_none());
    }

    #[test]
    fn handle_key_detail_mode() {
        let mut s = VersionsState::new();
        s.detail_mode = true;
        assert!(matches!(handle_key(&s, &key(KeyCode::Esc)), Some(VersionsMessage::CloseDetail)));
        assert!(matches!(handle_key(&s, &key(KeyCode::Up)), Some(VersionsMessage::ScrollDetailUp)));
        assert!(matches!(handle_key(&s, &key(KeyCode::Down)), Some(VersionsMessage::ScrollDetailDown)));
    }

    #[test]
    fn render_empty_no_panic() {
        let s = VersionsState::new();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&s, &mut buf, area);
    }

    #[test]
    fn render_with_tags_no_panic() {
        let mut s = VersionsState::new();
        s.tags = sample_tags();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&s, &mut buf, area);
    }

    #[test]
    fn render_detail_mode_no_panic() {
        let mut s = VersionsState::new();
        s.tags = sample_tags();
        s.detail_mode = true;
        s.detail_content = "tag v1.0.0\nDate: 2024-01-01\n\nInitial release".to_string();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&s, &mut buf, area);
    }
}
