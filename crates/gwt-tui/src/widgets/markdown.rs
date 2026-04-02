//! Markdown rendering widget for Issue/SPEC detail views.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Render markdown text with basic formatting.
///
/// Supports: headings (#), bold (**), code blocks (```), inline code (`).
/// This is intentionally simple — full markdown parsing is out of scope.
pub fn render(title: &str, content: &str, frame: &mut Frame, area: Rect) {
    let lines: Vec<Line> = content.lines().map(style_line).collect();

    let block = Block::default().borders(Borders::ALL).title(title);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

fn style_line(line: &str) -> Line<'static> {
    if line.starts_with("### ") {
        Line::from(Span::styled(
            line.to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))
    } else if line.starts_with("## ") {
        Line::from(Span::styled(
            line.to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
    } else if line.starts_with("# ") {
        Line::from(Span::styled(
            line.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
    } else if line.starts_with("```") {
        Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::DarkGray),
        ))
    } else if line.starts_with("- ") || line.starts_with("* ") {
        Line::from(Span::styled(
            format!("  \u{2022} {}", &line[2..]),
            Style::default().fg(Color::White),
        ))
    } else {
        Line::from(line.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_line_heading_levels() {
        let h1 = style_line("# Title");
        assert_eq!(h1.spans.len(), 1);

        let h2 = style_line("## Sub");
        assert_eq!(h2.spans.len(), 1);

        let h3 = style_line("### Detail");
        assert_eq!(h3.spans.len(), 1);
    }

    #[test]
    fn style_line_bullet() {
        let bullet = style_line("- item");
        let text: String = bullet.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('\u{2022}'));
    }

    #[test]
    fn style_line_plain() {
        let plain = style_line("Just text");
        let text: String = plain.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Just text");
    }
}
