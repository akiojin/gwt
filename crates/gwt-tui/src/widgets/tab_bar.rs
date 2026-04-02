//! Session tab bar widget.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Tabs,
    Frame,
};

use crate::model::{Model, SessionTabType};

/// Render the session tab bar.
pub fn render(model: &Model, frame: &mut Frame, area: Rect) {
    let titles: Vec<Line> = model
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let icon = match &s.tab_type {
                SessionTabType::Shell => "\u{25B6}",
                SessionTabType::Agent { .. } => "\u{2B50}",
            };
            let style = if i == model.active_session {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(Span::styled(format!(" {icon} {} ", s.name), style))
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(model.active_session)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(tabs, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentColor, SessionTab, VtState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    #[test]
    fn render_tab_bar_single_shell() {
        let model = Model::new(PathBuf::from("/tmp/test"));
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&model, f, area);
            })
            .unwrap();
    }

    #[test]
    fn render_tab_bar_with_agent_session() {
        let mut model = Model::new(PathBuf::from("/tmp/test"));
        model.sessions.push(SessionTab {
            id: "agent-0".to_string(),
            name: "Claude".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "claude".to_string(),
                color: AgentColor::Green,
            },
            vt: VtState::new(24, 80),
        });
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&model, f, area);
            })
            .unwrap();
    }
}
