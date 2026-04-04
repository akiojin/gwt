//! Markdown rendering widget for Issue/SPEC detail views.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
    Frame,
};

/// Render markdown text with basic formatting.
///
/// Supports: headings (#), bold (**), code blocks (```), inline code (`).
/// This is intentionally simple — full markdown parsing is out of scope.
pub fn render(title: &str, content: &str, frame: &mut Frame, area: Rect) {
    render_with_prelude(title, "", content, frame, area);
}

/// Render optional plain-text prelude lines followed by markdown content.
pub fn render_with_prelude(
    title: &str,
    prelude: &str,
    content: &str,
    frame: &mut Frame,
    area: Rect,
) {
    let lines = render_lines(prelude, content);

    let block = Block::default().title(title);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

fn render_lines(prelude: &str, content: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if !prelude.is_empty() {
        lines.extend(prelude.lines().map(|line| Line::from(line.to_string())));
    }

    if !prelude.is_empty() && !content.is_empty() {
        lines.push(Line::from(String::new()));
    }

    lines.extend(content.lines().map(style_line));
    lines
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

    #[test]
    fn style_line_code_block_fence() {
        let line = style_line("```rust");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "```rust");
    }

    #[test]
    fn style_line_star_bullet() {
        let bullet = style_line("* item two");
        let text: String = bullet.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('\u{2022}'));
        assert!(text.contains("item two"));
    }

    #[test]
    fn render_empty_content() {
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render("Title", "", f, area);
            })
            .unwrap();
        // Should not panic on empty input
    }

    #[test]
    fn render_multiline_content() {
        let content = "# Heading\n## Sub\n- bullet\n```code```\nplain text";
        let backend = ratatui::backend::TestBackend::new(60, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render("MD", content, f, area);
            })
            .unwrap();
    }

    #[test]
    fn render_lines_with_prelude_inserts_separator_before_markdown() {
        let lines = render_lines("Prelude", "## Heading\n- item");
        let text: Vec<String> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.to_string())
                    .collect()
            })
            .collect();

        assert_eq!(text[0], "Prelude");
        assert!(text[1].is_empty());
        assert_eq!(text[2], "## Heading");
        assert!(text[3].contains('\u{2022}'));
    }
}
