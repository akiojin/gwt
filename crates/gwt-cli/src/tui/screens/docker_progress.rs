//! Docker Progress Screen (SPEC-f5f5657e)
//!
//! Displays Docker container startup progress with animated spinner.

use ratatui::{prelude::*, widgets::*};

/// Spinner animation frames (ASCII only per CLAUDE.md)
const SPINNER_FRAMES: &[&str] = &["|", "/", "-", "\\"];

/// Docker operation status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerStatus {
    /// Detecting Docker files in worktree
    DetectingFiles,
    /// Building Docker image
    BuildingImage,
    /// Starting container(s)
    StartingContainer,
    /// Waiting for services to be ready
    WaitingForServices,
    /// Container is ready
    Ready,
    /// Operation failed
    Failed(String),
}

impl DockerStatus {
    /// Get status message for display
    pub fn message(&self) -> &str {
        match self {
            DockerStatus::DetectingFiles => "Detecting Docker files...",
            DockerStatus::BuildingImage => "Building Docker image...",
            DockerStatus::StartingContainer => "Starting container...",
            DockerStatus::WaitingForServices => "Waiting for services...",
            DockerStatus::Ready => "Container ready",
            DockerStatus::Failed(_) => "Failed",
        }
    }

    /// Check if the operation is in progress
    pub fn is_in_progress(&self) -> bool {
        !matches!(self, DockerStatus::Ready | DockerStatus::Failed(_))
    }

    /// Check if the operation failed
    pub fn is_failed(&self) -> bool {
        matches!(self, DockerStatus::Failed(_))
    }
}

/// Docker progress screen state
#[derive(Debug)]
pub struct DockerProgressState {
    /// Current status
    pub status: DockerStatus,
    /// Current spinner frame index
    pub spinner_frame: usize,
    /// Container name being started
    pub container_name: String,
    /// Detected Docker file type description
    pub docker_file_type: String,
    /// Worktree name
    pub worktree_name: String,
}

impl Default for DockerProgressState {
    fn default() -> Self {
        Self::new()
    }
}

impl DockerProgressState {
    /// Create a new DockerProgressState
    pub fn new() -> Self {
        Self {
            status: DockerStatus::DetectingFiles,
            spinner_frame: 0,
            container_name: String::new(),
            docker_file_type: String::new(),
            worktree_name: String::new(),
        }
    }

    /// Advance the spinner animation
    pub fn tick(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
    }

    /// Get the current spinner character
    pub fn spinner(&self) -> &str {
        SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
    }

    /// Set the status
    pub fn set_status(&mut self, status: DockerStatus) {
        self.status = status;
    }

    /// Set container information
    pub fn set_container_info(&mut self, name: &str, docker_type: &str, worktree: &str) {
        self.container_name = name.to_string();
        self.docker_file_type = docker_type.to_string();
        self.worktree_name = worktree.to_string();
    }
}

/// Render the Docker progress screen
pub fn render_docker_progress(state: &DockerProgressState, frame: &mut Frame, area: Rect) {
    // Calculate centered popup area
    let popup_width = 60.min(area.width.saturating_sub(4));
    let popup_height = 10.min(area.height.saturating_sub(4));

    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    // Clear background
    frame.render_widget(Clear, popup_area);

    // Outer block
    let block = Block::default()
        .title(" Docker Setup ")
        .title_style(Style::default().fg(Color::Cyan).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .border_type(BorderType::Rounded);

    frame.render_widget(block, popup_area);

    // Inner area for content
    let inner = Rect {
        x: popup_area.x + 2,
        y: popup_area.y + 2,
        width: popup_area.width.saturating_sub(4),
        height: popup_area.height.saturating_sub(4),
    };

    // Build content lines
    let mut lines: Vec<Line> = Vec::new();

    // Worktree info
    if !state.worktree_name.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Worktree: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.worktree_name, Style::default().fg(Color::White)),
        ]));
    }

    // Docker file type
    if !state.docker_file_type.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Type: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.docker_file_type, Style::default().fg(Color::White)),
        ]));
    }

    // Container name
    if !state.container_name.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Container: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.container_name, Style::default().fg(Color::Yellow)),
        ]));
    }

    // Empty line
    lines.push(Line::from(""));

    // Status with spinner
    let status_style = if state.status.is_failed() {
        Style::default().fg(Color::Red)
    } else if matches!(state.status, DockerStatus::Ready) {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let status_line = if state.status.is_in_progress() {
        Line::from(vec![
            Span::styled(state.spinner(), status_style),
            Span::raw(" "),
            Span::styled(state.status.message(), status_style),
        ])
    } else {
        Line::from(Span::styled(state.status.message(), status_style))
    };
    lines.push(status_line);

    // Error message if failed
    if let DockerStatus::Failed(ref msg) = state.status {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            msg.chars().take(inner.width as usize).collect::<String>(),
            Style::default().fg(Color::Red),
        )));
    }

    let paragraph = Paragraph::new(lines).alignment(Alignment::Left);

    frame.render_widget(paragraph, inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-401: Progress display rendering test
    #[test]
    fn test_docker_status_messages() {
        assert_eq!(DockerStatus::DetectingFiles.message(), "Detecting Docker files...");
        assert_eq!(DockerStatus::BuildingImage.message(), "Building Docker image...");
        assert_eq!(DockerStatus::StartingContainer.message(), "Starting container...");
        assert_eq!(DockerStatus::WaitingForServices.message(), "Waiting for services...");
        assert_eq!(DockerStatus::Ready.message(), "Container ready");
        assert_eq!(DockerStatus::Failed("error".to_string()).message(), "Failed");
    }

    #[test]
    fn test_docker_status_is_in_progress() {
        assert!(DockerStatus::DetectingFiles.is_in_progress());
        assert!(DockerStatus::BuildingImage.is_in_progress());
        assert!(DockerStatus::StartingContainer.is_in_progress());
        assert!(DockerStatus::WaitingForServices.is_in_progress());
        assert!(!DockerStatus::Ready.is_in_progress());
        assert!(!DockerStatus::Failed("error".to_string()).is_in_progress());
    }

    #[test]
    fn test_docker_status_is_failed() {
        assert!(!DockerStatus::DetectingFiles.is_failed());
        assert!(!DockerStatus::Ready.is_failed());
        assert!(DockerStatus::Failed("error".to_string()).is_failed());
    }

    #[test]
    fn test_docker_progress_state_new() {
        let state = DockerProgressState::new();
        assert!(matches!(state.status, DockerStatus::DetectingFiles));
        assert_eq!(state.spinner_frame, 0);
        assert!(state.container_name.is_empty());
    }

    #[test]
    fn test_docker_progress_state_tick() {
        let mut state = DockerProgressState::new();
        assert_eq!(state.spinner_frame, 0);
        state.tick();
        assert_eq!(state.spinner_frame, 1);
        state.tick();
        assert_eq!(state.spinner_frame, 2);
        state.tick();
        assert_eq!(state.spinner_frame, 3);
        state.tick();
        assert_eq!(state.spinner_frame, 0); // Wraps around
    }

    #[test]
    fn test_docker_progress_state_set_container_info() {
        let mut state = DockerProgressState::new();
        state.set_container_info("gwt-my-worktree", "docker-compose.yml", "my-worktree");
        assert_eq!(state.container_name, "gwt-my-worktree");
        assert_eq!(state.docker_file_type, "docker-compose.yml");
        assert_eq!(state.worktree_name, "my-worktree");
    }
}
