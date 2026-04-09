//! Session tab bar widget.
//!
//! Session tabs are now rendered as Block titles (same pattern as management tabs).

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders},
    Frame,
};

use crate::model::Model;
use crate::theme;

/// Render the session tab bar as a bordered block with tab title.
pub fn render(model: &Model, frame: &mut Frame, area: Rect) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, s) in model.sessions.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("│"));
        }
        let icon = s.tab_type.icon();
        let label = format!(" {icon} {} ", s.name);
        if i == model.active_session {
            spans.push(Span::styled(label, theme::style::tab_active()));
        } else {
            spans.push(Span::styled(label, theme::style::tab_inactive()));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme::border::default())
        .title(Line::from(spans));
    frame.render_widget(block, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentColor, SessionTab, SessionTabType, VtState};
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
            created_at: std::time::Instant::now(),
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
