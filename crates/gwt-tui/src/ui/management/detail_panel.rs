use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use super::ManagementState;

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

/// Build a detail row with a label and value.
fn detail_row<'a>(label: &'a str, value: Span<'a>) -> Line<'a> {
    Line::from(vec![
        Span::styled(label, Style::new().fg(Color::DarkGray)),
        value,
    ])
}

/// Render the detail panel for the selected agent.
pub fn render(buf: &mut Buffer, area: Rect, state: &ManagementState) {
    let block = Block::default()
        .title(" Agent Details ")
        .borders(Borders::ALL);

    let Some(agent) = state.selected_agent() else {
        let paragraph = Paragraph::new("No agent selected")
            .style(Style::new().fg(Color::DarkGray))
            .block(block);
        paragraph.render(area, buf);
        return;
    };

    let color = agent.status.color();
    let status_str = match agent.uptime {
        Some(ref uptime) => format!("{} ({})", agent.status, format_uptime(uptime)),
        None => agent.status.to_string(),
    };

    let mut lines = vec![
        detail_row("Name:      ", Span::raw(&agent.agent_name)),
        detail_row("Type:      ", Span::raw(&agent.agent_type)),
        detail_row(
            "Branch:    ",
            Span::raw(agent.branch.as_deref().unwrap_or("-")),
        ),
        detail_row(
            "Status:    ",
            Span::styled(status_str, Style::new().fg(color)),
        ),
    ];

    if let Some(ref spec_id) = agent.spec_id {
        lines.push(detail_row("SPEC:      ", Span::raw(spec_id.as_str())));
    }

    if let Some(ref pr_url) = agent.pr_url {
        lines.push(detail_row("PR:        ", Span::raw(pr_url.as_str())));
    }

    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("[k]", Style::new().fg(Color::Yellow)),
        Span::raw(" Kill  "),
        Span::styled("[r]", Style::new().fg(Color::Yellow)),
        Span::raw(" Restart  "),
        Span::styled("[Enter]", Style::new().fg(Color::Yellow)),
        Span::raw(" Focus"),
    ]));

    Paragraph::new(lines).block(block).render(area, buf);
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

    fn collect_buf_content(buf: &Buffer, width: u16, height: u16) -> String {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect()
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
        let content = collect_buf_content(&buf, 50, 12);
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
        let content = collect_buf_content(&buf, 50, 14);
        assert!(content.contains("SPEC-1776"));
        assert!(content.contains("#42"));
    }
}
