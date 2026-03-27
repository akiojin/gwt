use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use super::{AgentStatus, ManagementState};

/// Map an `AgentStatus` to a display color.
pub fn status_color(status: &AgentStatus) -> Color {
    match status {
        AgentStatus::Running => Color::Green,
        AgentStatus::Idle => Color::Yellow,
        AgentStatus::Completed(_) => Color::Cyan,
        AgentStatus::Error(_) => Color::Red,
    }
}

/// Short text label for an agent status.
fn status_text(status: &AgentStatus) -> &str {
    match status {
        AgentStatus::Running => "running",
        AgentStatus::Idle => "idle",
        AgentStatus::Completed(_) => "done",
        AgentStatus::Error(_) => "error",
    }
}

/// Render the agent list into the given area.
pub fn render(buf: &mut Buffer, area: Rect, state: &ManagementState) {
    let items: Vec<ListItem> = state
        .agents
        .iter()
        .map(|agent| {
            let branch_text = agent
                .branch
                .as_deref()
                .unwrap_or("-");
            let branch_display: String = if branch_text.chars().count() > 16 {
                let truncated: String = branch_text.chars().take(13).collect();
                format!("{truncated}...")
            } else {
                branch_text.to_string()
            };

            let color = status_color(&agent.status);

            let line = Line::from(vec![
                Span::styled("▌ ", Style::default().fg(color)),
                Span::styled(
                    format!("{:<14}", agent.agent_name),
                    Style::default(),
                ),
                Span::raw(format!("{:<17}", branch_display)),
                Span::styled(status_text(&agent.status), Style::default().fg(color)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let block = Block::default()
        .title(" Agents ")
        .borders(Borders::ALL);

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::REVERSED),
        )
        .highlight_symbol("> ");

    let mut list_state = ListState::default();
    if !state.agents.is_empty() {
        list_state.select(Some(state.selected_index));
    }

    ratatui::widgets::StatefulWidget::render(list, area, buf, &mut list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::management::{AgentEntry, AgentStatus, ManagementState};

    fn make_entry(name: &str, branch: Option<&str>, status: AgentStatus) -> AgentEntry {
        AgentEntry {
            pane_id: format!("pane-{}", name),
            agent_name: name.to_string(),
            agent_type: "claude".to_string(),
            branch: branch.map(|s| s.to_string()),
            status,
            uptime: None,
            pr_url: None,
            spec_id: None,
        }
    }

    #[test]
    fn test_render_empty_agent_list() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
        let state = ManagementState::default();
        render(&mut buf, Rect::new(0, 0, 40, 10), &state);
        // Should render the block title without panic
        let top_row: String = (0..40)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(top_row.contains("Agents"));
    }

    #[test]
    fn test_render_agents_with_selection() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 10));
        let state = ManagementState {
            agents: vec![
                make_entry("Claude Code", Some("feat/xyz"), AgentStatus::Running),
                make_entry("Codex CLI", Some("main"), AgentStatus::Idle),
            ],
            selected_index: 0,
            ..Default::default()
        };
        render(&mut buf, Rect::new(0, 0, 60, 10), &state);
        // Check that agent names appear in the rendered buffer
        let row1: String = (0..60)
            .map(|x| buf.cell((x, 1)).unwrap().symbol().to_string())
            .collect();
        assert!(row1.contains("Claude Code"));
    }

    #[test]
    fn test_status_color_mapping() {
        assert_eq!(status_color(&AgentStatus::Running), Color::Green);
        assert_eq!(status_color(&AgentStatus::Idle), Color::Yellow);
        assert_eq!(status_color(&AgentStatus::Completed(0)), Color::Cyan);
        assert_eq!(
            status_color(&AgentStatus::Error("fail".into())),
            Color::Red
        );
    }
}
