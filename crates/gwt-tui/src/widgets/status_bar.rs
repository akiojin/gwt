//! Status bar widget — footer with session info, branch, and help hint.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::input::voice;
use crate::model::{ActiveLayer, Model, SessionLayout};

/// Render the status bar.
pub fn render(model: &Model, frame: &mut Frame, area: Rect) {
    let session_name = model
        .active_session_tab()
        .map(|s| s.name.as_str())
        .unwrap_or("No session");

    let layout_icon = match model.session_layout {
        SessionLayout::Tab => "\u{25A3}",
        SessionLayout::Grid => "\u{25A6}",
    };

    let layer = match model.active_layer {
        ActiveLayer::Main => "Main",
        ActiveLayer::Management => "Mgmt",
    };

    let mut spans = vec![
        Span::styled(
            format!(" {layout_icon} {session_name} "),
            Style::default().fg(Color::White),
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
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ));
    }

    spans.push(Span::styled(
        format!(" {} ", model.repo_path.display()),
        Style::default().fg(Color::DarkGray),
    ));
    spans.push(Span::styled(
        " Ctrl+G,? Help ",
        Style::default().fg(Color::DarkGray),
    ));

    let status = Line::from(spans);

    let bar = Paragraph::new(status).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(bar, area);
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
