use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use super::ManagementState;
use crate::ui::management::agent_list::status_color;

/// Format a duration as "Xh Ym Zs" or "Ym Zs" or "Zs".
fn format_uptime(d: &std::time::Duration) -> String {
    let total_secs = d.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

/// Render the detail panel for the selected agent.
pub fn render(buf: &mut Buffer, area: Rect, state: &ManagementState) {
    let block = Block::default()
        .title(" Agent Details ")
        .borders(Borders::ALL);

    if let Some(agent) = state.selected_agent() {
        let color = status_color(&agent.status);
        let status_str = if let Some(ref uptime) = agent.uptime {
            format!("{} ({})", agent.status, format_uptime(uptime))
        } else {
            agent.status.to_string()
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled("Name:      ", Style::default().fg(Color::DarkGray)),
                Span::raw(&agent.agent_name),
            ]),
            Line::from(vec![
                Span::styled("Type:      ", Style::default().fg(Color::DarkGray)),
                Span::raw(&agent.agent_type),
            ]),
            Line::from(vec![
                Span::styled("Branch:    ", Style::default().fg(Color::DarkGray)),
                Span::raw(agent.branch.as_deref().unwrap_or("-")),
            ]),
            Line::from(vec![
                Span::styled("Status:    ", Style::default().fg(Color::DarkGray)),
                Span::styled(status_str, Style::default().fg(color)),
            ]),
        ];

        if let Some(ref spec_id) = agent.spec_id {
            lines.push(Line::from(vec![
                Span::styled("SPEC:      ", Style::default().fg(Color::DarkGray)),
                Span::raw(spec_id.as_str()),
            ]));
        }

        if let Some(ref pr_url) = agent.pr_url {
            lines.push(Line::from(vec![
                Span::styled("PR:        ", Style::default().fg(Color::DarkGray)),
                Span::raw(pr_url.as_str()),
            ]));
        }

        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled("[k]", Style::default().fg(Color::Yellow)),
            Span::raw(" Kill  "),
            Span::styled("[r]", Style::default().fg(Color::Yellow)),
            Span::raw(" Restart  "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Focus"),
        ]));

        let paragraph = Paragraph::new(lines).block(block);
        paragraph.render(area, buf);
    } else {
        let paragraph = Paragraph::new("No agent selected")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        paragraph.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::management::{AgentEntry, AgentStatus, ManagementState};

    fn make_entry_full() -> AgentEntry {
        AgentEntry {
            pane_id: "pane-1".to_string(),
            agent_name: "Claude Code".to_string(),
            agent_type: "claude".to_string(),
            branch: Some("feature/xyz".to_string()),
            status: AgentStatus::Running,
            uptime: Some(std::time::Duration::from_secs(754)),
            pr_url: Some("#42".to_string()),
            spec_id: Some("SPEC-1776".to_string()),
        }
    }

    #[test]
    fn test_render_no_selection() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 10));
        let state = ManagementState::default();
        render(&mut buf, Rect::new(0, 0, 50, 10), &state);
        let row1: String = (0..50)
            .map(|x| buf.cell((x, 1)).unwrap().symbol().to_string())
            .collect();
        assert!(row1.contains("No agent selected"));
    }

    #[test]
    fn test_render_agent_details() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 12));
        let state = ManagementState {
            agents: vec![AgentEntry {
                pane_id: "pane-1".to_string(),
                agent_name: "Claude Code".to_string(),
                agent_type: "claude".to_string(),
                branch: Some("feature/xyz".to_string()),
                status: AgentStatus::Running,
                uptime: None,
                pr_url: None,
                spec_id: None,
            }],
            selected_index: 0,
            ..Default::default()
        };
        render(&mut buf, Rect::new(0, 0, 50, 12), &state);
        let content: String = (0..12)
            .flat_map(|y| (0..50).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(content.contains("Claude Code"));
        assert!(content.contains("claude"));
        assert!(content.contains("Running"));
    }

    #[test]
    fn test_render_with_pr_and_spec() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 14));
        let state = ManagementState {
            agents: vec![make_entry_full()],
            selected_index: 0,
            ..Default::default()
        };
        render(&mut buf, Rect::new(0, 0, 50, 14), &state);
        let content: String = (0..14)
            .flat_map(|y| (0..50).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(content.contains("SPEC-1776"));
        assert!(content.contains("#42"));
    }
}
