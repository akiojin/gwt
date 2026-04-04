//! Status bar widget — footer with session info, branch, and help hint.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use gwt_notification::{Notification, Severity};

use crate::input::voice;
use crate::model::{ActiveLayer, Model, SessionLayout, SessionTabType};

/// Render the status bar.
pub fn render(model: &Model, frame: &mut Frame, area: Rect) {
    render_with_notification(model, model.current_notification.as_ref(), frame, area);
}

/// Render the status bar with an optional notification segment.
pub fn render_with_notification(
    model: &Model,
    notification: Option<&Notification>,
    frame: &mut Frame,
    area: Rect,
) {
    render_with_notification_and_hints(model, notification, None, frame, area);
}

/// Render the status bar with an optional notification segment and contextual hints.
pub fn render_with_notification_and_hints(
    model: &Model,
    notification: Option<&Notification>,
    hints: Option<&str>,
    frame: &mut Frame,
    area: Rect,
) {
    let session_name = model
        .active_session_tab()
        .map(|s| s.name.as_str())
        .unwrap_or("No session");
    let session_type = model
        .active_session_tab()
        .map(|s| session_type_label(&s.tab_type))
        .unwrap_or_else(|| "None".to_string());
    let branch_context = derive_branch_context(model).unwrap_or_else(|| "n/a".to_string());

    let layout_icon = match model.session_layout {
        SessionLayout::Tab => "\u{25A3}",
        SessionLayout::Grid => "\u{25A6}",
    };

    let layer = match model.active_layer {
        ActiveLayer::Initialization => "Init",
        ActiveLayer::Main => "Main",
        ActiveLayer::Management => "Mgmt",
    };

    let mut spans = vec![
        Span::styled(
            format!(" {layout_icon} {session_name} "),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!(" branch: {branch_context} "),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            format!(" type: {session_type} "),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(
            format!(" [{layer}] "),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    // Voice indicator (when active)
    if let Some(indicator) = voice::render_indicator(&model.voice) {
        spans.push(Span::styled(
            format!(" {indicator} "),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }

    if let Some(notification) = notification {
        spans.push(notification_span(notification));
    }

    spans.push(Span::styled(
        format!(" {} ", model.repo_path.display()),
        Style::default().fg(Color::DarkGray),
    ));
    if let Some(hints) = hints.filter(|value| !value.is_empty()) {
        spans.push(Span::styled(
            format!(" {hints} "),
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        spans.push(Span::styled(
            " Ctrl+G,? Help ",
            Style::default().fg(Color::DarkGray),
        ));
    }

    let status = Line::from(spans);

    let bar = Paragraph::new(status).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(bar, area);
}

fn derive_branch_context(model: &Model) -> Option<String> {
    if let Some(session) = model.active_session_tab() {
        if let Some(branch) = session.name.strip_prefix("Shell: ") {
            return Some(branch.to_string());
        }
    }

    model.branches.selected_branch().map(|branch| branch.name.clone())
}

fn session_type_label(tab_type: &SessionTabType) -> String {
    match tab_type {
        SessionTabType::Shell => "Shell".to_string(),
        SessionTabType::Agent { agent_id, .. } => match agent_id.as_str() {
            "claude" => "Claude".to_string(),
            "codex" => "Codex".to_string(),
            "gemini" => "Gemini CLI".to_string(),
            "opencode" => "OpenCode".to_string(),
            other => other.to_string(),
        },
    }
}

fn notification_span(notification: &Notification) -> Span<'static> {
    let style = match notification.severity {
        Severity::Debug => Style::default().fg(Color::DarkGray),
        Severity::Info => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        Severity::Warn => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        Severity::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    };

    let summary = match notification.detail.as_deref() {
        Some(detail) if !detail.is_empty() => format!(
            " {} {}: {} - {} ",
            notification.severity, notification.source, notification.message, detail
        ),
        _ => format!(
            " {} {}: {} ",
            notification.severity, notification.source, notification.message
        ),
    };

    Span::styled(summary, style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentColor, SessionTab, SessionTabType, VtState};
    use crate::screens::branches::BranchCategory;
    use crate::screens::branches::BranchItem;
    use gwt_notification::{Notification, Severity};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    #[test]
    fn render_status_bar_tab_layout() {
        let model = Model::new(PathBuf::from("/tmp/test"));
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&model, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("Shell"));
        assert!(text.contains("Mgmt"));
    }

    #[test]
    fn render_status_bar_grid_management() {
        let mut model = Model::new(PathBuf::from("/tmp/test"));
        model.session_layout = SessionLayout::Grid;
        model.active_layer = ActiveLayer::Management;
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&model, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("Mgmt"));
    }

    #[test]
    fn render_with_info_notification_shows_summary() {
        let model = Model::new(PathBuf::from("/tmp/test"));
        let notification = Notification::new(Severity::Info, "core", "Started");
        let backend = TestBackend::new(100, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_with_notification(&model, Some(&notification), f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("INFO"));
        assert!(text.contains("core"));
        assert!(text.contains("Started"));
    }

    #[test]
    fn render_with_warn_notification_shows_summary() {
        let model = Model::new(PathBuf::from("/tmp/test"));
        let notification = Notification::new(Severity::Warn, "git", "Detached HEAD");
        let backend = TestBackend::new(100, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_with_notification(&model, Some(&notification), f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("WARN"));
        assert!(text.contains("git"));
        assert!(text.contains("Detached HEAD"));
    }

    #[test]
    fn render_status_bar_shell_session_shows_branch_and_shell_type() {
        let mut model = Model::new(PathBuf::from("/tmp/test"));
        model.active_layer = ActiveLayer::Main;
        model.sessions[0] = SessionTab {
            id: "shell-0".to_string(),
            name: "Shell: feature/status-bar".to_string(),
            tab_type: SessionTabType::Shell,
            vt: VtState::new(24, 80),
        };

        let backend = TestBackend::new(140, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&model, f, area);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("branch: feature/status-bar"));
        assert!(text.contains("type: Shell"));
    }

    #[test]
    fn render_status_bar_agent_session_shows_agent_type_and_selected_branch_context() {
        let mut model = Model::new(PathBuf::from("/tmp/test"));
        model.active_layer = ActiveLayer::Management;
        model.sessions[0] = SessionTab {
            id: "agent-0".to_string(),
            name: "Codex".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "codex".to_string(),
                color: AgentColor::Blue,
            },
            vt: VtState::new(24, 80),
        };
        model.branches.branches = vec![BranchItem {
            name: "feature/agent-context".to_string(),
            is_head: false,
            is_local: true,
            category: BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-agent-context")),
        }];
        model.branches.selected = 0;

        let backend = TestBackend::new(160, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&model, f, area);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("branch: feature/agent-context"));
        assert!(text.contains("type: Codex"));
    }
}
