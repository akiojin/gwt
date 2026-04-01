//! Lightweight Markdown rendering helpers for TUI detail panes.

use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Widget, Wrap};

/// Render a compact subset of Markdown into the target area.
pub fn render_markdown(buf: &mut Buffer, area: Rect, markdown: &str, scroll: usize) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let lines = markdown_lines(markdown);
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll.min(u16::MAX as usize) as u16, 0));
    Widget::render(paragraph, area, buf);
}

fn markdown_lines(markdown: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;

    for raw_line in markdown.lines() {
        let trimmed = raw_line.trim_end();

        if trimmed.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            lines.push(Line::from(Span::styled(
                trimmed.to_string(),
                Style::default().fg(Color::Yellow),
            )));
            continue;
        }

        let line = if let Some(text) = trimmed.strip_prefix("# ") {
            Line::from(inline_spans(
                text,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        } else if let Some(text) = trimmed.strip_prefix("## ") {
            Line::from(inline_spans(
                text,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ))
        } else if let Some(text) = trimmed.strip_prefix("### ") {
            Line::from(inline_spans(
                text,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ))
        } else if let Some(text) = trimmed.strip_prefix("> ") {
            let mut spans = vec![Span::styled("│ ", Style::default().fg(Color::DarkGray))];
            spans.extend(inline_spans(text, Style::default().fg(Color::Gray)));
            Line::from(spans)
        } else if let Some(text) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let mut spans = vec![Span::styled("• ", Style::default().fg(Color::Cyan))];
            spans.extend(inline_spans(text, Style::default()));
            Line::from(spans)
        } else if is_thematic_break(trimmed) {
            Line::from(Span::styled(
                "─".repeat(32),
                Style::default().fg(Color::DarkGray),
            ))
        } else if trimmed.is_empty() {
            Line::from("")
        } else {
            Line::from(inline_spans(trimmed, Style::default()))
        };

        lines.push(line);
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}

fn inline_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remainder = text;
    let mut in_code = false;

    while let Some(idx) = remainder.find('`') {
        let (head, tail) = remainder.split_at(idx);
        if !head.is_empty() {
            spans.push(Span::styled(
                head.to_string(),
                style_for_segment(base_style, in_code),
            ));
        }

        remainder = &tail[1..];
        in_code = !in_code;
    }

    if !remainder.is_empty() {
        spans.push(Span::styled(
            remainder.to_string(),
            style_for_segment(base_style, in_code),
        ));
    }

    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base_style));
    }

    spans
}

fn style_for_segment(base_style: Style, in_code: bool) -> Style {
    if in_code {
        base_style
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
            .bg(Color::Rgb(40, 40, 40))
    } else {
        base_style
    }
}

fn is_thematic_break(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '-' || ch == '*') && trimmed.len() >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(buf: &Buffer, area: Rect, row: u16) -> String {
        (area.x..area.right())
            .map(|x| buf[(x, area.y + row)].symbol())
            .collect::<String>()
    }

    #[test]
    fn render_markdown_strips_heading_marker() {
        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);

        render_markdown(&mut buf, area, "# Title\n\n- Item", 0);

        assert_eq!(line_text(&buf, area, 0).trim(), "Title");
        assert!(line_text(&buf, area, 2).contains("• Item"));
    }
}
