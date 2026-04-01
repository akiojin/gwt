//! Status bar widget: bottom bar showing context and key hints

use ratatui::prelude::*;

use crate::model::{ActiveLayer, Model};

/// Render the status bar.
pub fn render(model: &Model, buf: &mut Buffer, area: Rect) {
    // Fill background
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_style(Style::default().bg(Color::DarkGray));
            cell.set_char(' ');
        }
    }

    let left = match model.active_layer {
        ActiveLayer::Main => {
            if model.session_tabs.is_empty() {
                " No sessions".to_string()
            } else {
                let tab = &model.session_tabs[model.active_session];
                let branch = tab.branch.as_deref().unwrap_or("");
                if branch.is_empty() {
                    format!(" {}", tab.name)
                } else {
                    format!(" {} | {}", tab.name, branch)
                }
            }
        }
        ActiveLayer::Management => {
            let tab_name = model.management_tab.label();
            let running = model.running_session_count();
            if running > 0 {
                format!(" {tab_name} | {running} running")
            } else {
                format!(" {tab_name}")
            }
        }
    };

    let hints = match model.active_layer {
        ActiveLayer::Main if model.session_tabs.is_empty() => {
            " Enter on Branches: Agent | Ctrl+G,c: Shell | Ctrl+G,Ctrl+G: Manage "
        }
        ActiveLayer::Main => {
            " Wheel: Scroll | PgUp/PgDn: History | Drag: Copy | Ctrl+G,Ctrl+G: Manage | Ctrl+G,x: Close "
        }
        ActiveLayer::Management => " Tab: Switch | Ctrl+G,Ctrl+G: Terminal | Ctrl+C×2: Quit ",
    };

    let preferred_left_width = left.chars().count().min(usize::from(area.width)) as u16;
    let left_width = preferred_left_width;
    let right_width = area.width.saturating_sub(left_width);

    let left_span = Span::styled(left, Style::default().fg(Color::White).bg(Color::DarkGray));
    buf.set_span(area.x, area.y, &left_span, left_width);

    if right_width > 0 {
        let right_span = Span::styled(hints, Style::default().fg(Color::Gray).bg(Color::DarkGray));
        let right_x = area.x + left_width;
        buf.set_span(right_x, area.y, &right_span, right_width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ManagementTab, Model};
    use std::path::PathBuf;

    fn test_model() -> Model {
        Model::new(PathBuf::from("/tmp/test-repo"))
    }

    fn buf_row_text(buf: &Buffer, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| {
                buf.cell((x, y))
                    .map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))
            })
            .collect()
    }

    #[test]
    fn render_management_layer_smoke() {
        let model = test_model();
        assert_eq!(model.active_layer, ActiveLayer::Management);
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        render(&model, &mut buf, area);
        let text = buf_row_text(&buf, 0, 120);
        // Management tab label should appear
        assert!(
            text.contains("Branches"),
            "Expected 'Branches' in: {text:?}"
        );
        // Hint text should appear
        assert!(
            text.contains("Tab: Switch"),
            "Expected hint text in: {text:?}"
        );
    }

    #[test]
    fn render_main_layer_no_sessions_smoke() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        render(&model, &mut buf, area);
        let text = buf_row_text(&buf, 0, 120);
        assert!(
            text.contains("Ctrl+G,c: Shell"),
            "Expected session creation hint in: {text:?}"
        );
    }

    #[test]
    fn render_main_layer_with_session_no_branch() {
        let mut model = test_model();
        use crate::model::{SessionStatus, SessionTab, SessionTabType};
        use gwt_core::terminal::AgentColor;
        model.add_session(SessionTab {
            pane_id: "p1".into(),
            name: "Shell #1".into(),
            tab_type: SessionTabType::Shell,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: None,
            spec_id: None,
        });
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        render(&model, &mut buf, area);
        let text = buf_row_text(&buf, 0, 120);
        assert!(text.contains("Shell #1"), "Expected tab name in: {text:?}");
        assert!(!text.contains("LIVE"), "Unexpected LIVE label in: {text:?}");
        assert!(
            !text.contains("SCROLLED"),
            "Unexpected SCROLLED label in: {text:?}"
        );
    }

    #[test]
    fn render_main_layer_with_session_and_branch() {
        let mut model = test_model();
        use crate::model::{SessionStatus, SessionTab, SessionTabType};
        use gwt_core::terminal::AgentColor;
        model.add_session(SessionTab {
            pane_id: "p2".into(),
            name: "Agent #1".into(),
            tab_type: SessionTabType::Agent,
            color: AgentColor::Blue,
            status: SessionStatus::Running,
            branch: Some("feature/test".into()),
            spec_id: None,
        });
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        render(&model, &mut buf, area);
        let text = buf_row_text(&buf, 0, 120);
        assert!(text.contains("Agent #1"), "Expected tab name in: {text:?}");
        assert!(
            text.contains("feature/test"),
            "Expected branch in: {text:?}"
        );
        assert!(!text.contains("LIVE"), "Unexpected LIVE label in: {text:?}");
        assert!(
            !text.contains("SCROLLED"),
            "Unexpected SCROLLED label in: {text:?}"
        );
    }

    #[test]
    fn render_management_with_running_agents() {
        let mut model = test_model();
        use crate::model::{SessionStatus, SessionTab, SessionTabType};
        use gwt_core::terminal::AgentColor;
        model.session_tabs.push(SessionTab {
            pane_id: "p1".into(),
            name: "Agent #1".into(),
            tab_type: SessionTabType::Agent,
            color: AgentColor::Blue,
            status: SessionStatus::Running,
            branch: Some("feature/test".into()),
            spec_id: None,
        });
        model.session_tabs.push(SessionTab {
            pane_id: "p2".into(),
            name: "Agent #2".into(),
            tab_type: SessionTabType::Agent,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: None,
            spec_id: None,
        });
        // Stay in management layer
        model.active_layer = ActiveLayer::Management;
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        render(&model, &mut buf, area);
        let text = buf_row_text(&buf, 0, 120);
        assert!(
            text.contains("2 running"),
            "Expected '2 running' in: {text:?}"
        );
    }

    #[test]
    fn render_management_no_running_agents() {
        let model = test_model();
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        render(&model, &mut buf, area);
        let text = buf_row_text(&buf, 0, 120);
        assert!(
            !text.contains("running"),
            "Expected no 'running' in: {text:?}"
        );
    }

    #[test]
    fn render_all_management_tabs_no_panic() {
        let mut model = test_model();
        let area = Rect::new(0, 0, 120, 1);
        for tab in ManagementTab::ALL {
            model.management_tab = tab;
            model.active_layer = ActiveLayer::Management;
            let mut buf = Buffer::empty(area);
            render(&model, &mut buf, area);
            let text = buf_row_text(&buf, 0, 120);
            assert!(
                text.contains(tab.label()),
                "Expected '{}' in: {text:?}",
                tab.label()
            );
        }
    }
}
