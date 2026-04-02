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
    let raw_lines: Vec<&str> = markdown.lines().collect();
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut i = 0;

    while i < raw_lines.len() {
        let trimmed = raw_lines[i].trim_end();

        if trimmed.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            i += 1;
            continue;
        }

        if in_code_block {
            lines.push(Line::from(Span::styled(
                trimmed.to_string(),
                Style::default().fg(Color::Yellow),
            )));
            i += 1;
            continue;
        }

        // Table block detection: collect consecutive `| ... |` lines
        if is_table_row(trimmed) {
            let table_start = i;
            let mut table_rows: Vec<&str> = Vec::new();
            while i < raw_lines.len() && is_table_row(raw_lines[i].trim_end()) {
                table_rows.push(raw_lines[i].trim_end());
                i += 1;
            }
            lines.extend(render_table_block(&table_rows, table_start));
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
        i += 1;
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.ends_with('|') && t.len() >= 3
}

fn is_separator_row(line: &str) -> bool {
    let t = line.trim();
    if !is_table_row(t) {
        return false;
    }
    t[1..t.len() - 1].split('|').all(|cell| {
        let c = cell.trim();
        !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')
    })
}

fn parse_table_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let inner = &t[1..t.len() - 1]; // strip leading/trailing `|`
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

fn render_table_block(rows: &[&str], _start: usize) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return Vec::new();
    }

    // Parse all rows into cells, identify separator rows
    let parsed: Vec<(Vec<String>, bool)> = rows
        .iter()
        .map(|r| (parse_table_cells(r), is_separator_row(r)))
        .collect();

    // Calculate column widths
    let col_count = parsed
        .iter()
        .map(|(cells, _)| cells.len())
        .max()
        .unwrap_or(0);
    let mut col_widths = vec![0usize; col_count];
    for (cells, is_sep) in &parsed {
        if *is_sep {
            continue;
        }
        for (j, cell) in cells.iter().enumerate() {
            if j < col_widths.len() {
                col_widths[j] = col_widths[j].max(strip_inline_markers(cell).len());
            }
        }
    }

    let sep_style = Style::default().fg(Color::DarkGray);
    let header_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    // Determine which row indices are headers (rows before the first separator)
    let first_sep = parsed.iter().position(|(_, is_sep)| *is_sep);
    let header_end = first_sep.unwrap_or(0);

    let mut lines = Vec::new();
    for (idx, (cells, is_sep)) in parsed.iter().enumerate() {
        if *is_sep {
            // Render separator as ├─┼─┤
            let mut sep_parts: Vec<String> = Vec::new();
            for (j, w) in col_widths.iter().enumerate() {
                if j > 0 {
                    sep_parts.push("─┼─".to_string());
                }
                sep_parts.push("─".repeat(*w));
            }
            lines.push(Line::from(Span::styled(
                format!("─┤{}├─", sep_parts.join("")),
                sep_style,
            )));
            continue;
        }

        let is_header = idx < header_end;
        let cell_style = if is_header {
            header_style
        } else {
            Style::default()
        };

        let mut spans: Vec<Span<'static>> = Vec::new();
        for (j, w) in col_widths.iter().enumerate() {
            if j > 0 {
                spans.push(Span::styled(" │ ", sep_style));
            }
            let raw = cells.get(j).map(|s| s.as_str()).unwrap_or("");
            let display_len = strip_inline_markers(raw).len();
            let padding = w.saturating_sub(display_len);
            let mut cell_spans = inline_spans(raw, cell_style);
            if padding > 0 {
                cell_spans.push(Span::styled(" ".repeat(padding), cell_style));
            }
            spans.extend(cell_spans);
        }
        lines.push(Line::from(spans));
    }

    lines
}

fn inline_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remainder: &str = text;

    while !remainder.is_empty() {
        if remainder.starts_with("**") {
            // Bold: find closing **
            if let Some(end) = remainder[2..].find("**") {
                let content = &remainder[2..2 + end];
                spans.push(Span::styled(
                    content.to_string(),
                    base_style.add_modifier(Modifier::BOLD),
                ));
                remainder = &remainder[2 + end + 2..];
                continue;
            }
        }

        if remainder.starts_with('`') {
            // Inline code: find closing `
            if let Some(end) = remainder[1..].find('`') {
                let content = &remainder[1..1 + end];
                spans.push(Span::styled(
                    content.to_string(),
                    base_style
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                        .bg(Color::Rgb(40, 40, 40)),
                ));
                remainder = &remainder[1 + end + 1..];
                continue;
            }
        }

        if remainder.starts_with('*') && !remainder.starts_with("**") {
            // Italic: find closing * (not preceded by another *)
            if let Some(end) = remainder[1..].find('*') {
                if end > 0 {
                    let content = &remainder[1..1 + end];
                    spans.push(Span::styled(
                        content.to_string(),
                        base_style.add_modifier(Modifier::ITALIC),
                    ));
                    remainder = &remainder[1 + end + 1..];
                    continue;
                }
            }
        }

        // Plain text: consume until next openable marker or end.
        // A marker is "openable" only if its closing pair exists.
        let mut end = remainder.len();
        for pos in 1..remainder.len() {
            let tail = &remainder[pos..];
            if tail.starts_with("**") && tail[2..].contains("**") {
                end = pos;
                break;
            }
            if tail.starts_with('`') && tail[1..].contains('`') {
                end = pos;
                break;
            }
            if tail.starts_with('*') && !tail.starts_with("**") && tail[1..].contains('*') {
                end = pos;
                break;
            }
        }
        spans.push(Span::styled(remainder[..end].to_string(), base_style));
        remainder = &remainder[end..];
    }

    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base_style));
    }

    spans
}

/// Strip inline markers (`**`, `*`, `` ` ``) to get display-width text.
fn strip_inline_markers(text: &str) -> String {
    let mut result = String::new();
    let mut remainder: &str = text;

    while !remainder.is_empty() {
        if remainder.starts_with("**") {
            if let Some(end) = remainder[2..].find("**") {
                result.push_str(&remainder[2..2 + end]);
                remainder = &remainder[2 + end + 2..];
                continue;
            }
        }
        if remainder.starts_with('`') {
            if let Some(end) = remainder[1..].find('`') {
                result.push_str(&remainder[1..1 + end]);
                remainder = &remainder[1 + end + 1..];
                continue;
            }
        }
        if remainder.starts_with('*') && !remainder.starts_with("**") {
            if let Some(end) = remainder[1..].find('*') {
                if end > 0 {
                    result.push_str(&remainder[1..1 + end]);
                    remainder = &remainder[1 + end + 1..];
                    continue;
                }
            }
        }
        // Consume one char (handles unclosed markers as literals)
        let ch_len = remainder.chars().next().map_or(0, |c| c.len_utf8());
        result.push_str(&remainder[..ch_len]);
        remainder = &remainder[ch_len..];
    }

    result
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

    #[test]
    fn render_markdown_table() {
        let md = "| Name | Age |\n| --- | --- |\n| Alice | 30 |\n| Bob | 25 |";
        let area = Rect::new(0, 0, 60, 5);
        let mut buf = Buffer::empty(area);

        render_markdown(&mut buf, area, md, 0);

        let row0 = line_text(&buf, area, 0);
        let row1 = line_text(&buf, area, 1);
        let row2 = line_text(&buf, area, 2);

        // Header row contains column names
        assert!(
            row0.contains("Name"),
            "header should contain 'Name': {row0}"
        );
        assert!(row0.contains("Age"), "header should contain 'Age': {row0}");

        // Separator row contains box-drawing chars
        assert!(row1.contains('─'), "separator should contain '─': {row1}");

        // Data row contains values
        assert!(
            row2.contains("Alice"),
            "data row should contain 'Alice': {row2}"
        );
        assert!(row2.contains("30"), "data row should contain '30': {row2}");
    }

    #[test]
    fn is_table_row_detects_pipe_lines() {
        assert!(super::is_table_row("| a | b |"));
        assert!(super::is_table_row("| --- | --- |"));
        assert!(!super::is_table_row("not a table"));
        assert!(!super::is_table_row("| only start"));
        assert!(!super::is_table_row("only end |"));
    }

    #[test]
    fn is_separator_row_detects_dashes() {
        assert!(super::is_separator_row("| --- | --- |"));
        assert!(super::is_separator_row("| :---: | ---: |"));
        assert!(!super::is_separator_row("| Name | Age |"));
    }

    #[test]
    fn parse_table_cells_splits_correctly() {
        let cells = super::parse_table_cells("| Name | Age | City |");
        assert_eq!(cells, vec!["Name", "Age", "City"]);
    }

    #[test]
    fn inline_spans_bold() {
        let spans = super::inline_spans("hello **world**", Style::default());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), "hello ");
        assert_eq!(spans[1].content.as_ref(), "world");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn inline_spans_italic() {
        let spans = super::inline_spans("hello *world*", Style::default());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), "hello ");
        assert_eq!(spans[1].content.as_ref(), "world");
        assert!(spans[1].style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn inline_spans_mixed() {
        let spans = super::inline_spans("**bold** and `code`", Style::default());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "bold");
        assert_eq!(spans[1].content.as_ref(), " and ");
        assert_eq!(spans[2].content.as_ref(), "code");
    }

    #[test]
    fn inline_spans_unclosed_bold() {
        let spans = super::inline_spans("**unclosed", Style::default());
        // No closing ** → treated as literal
        assert_eq!(spans[0].content.as_ref(), "**unclosed");
    }

    #[test]
    fn strip_inline_markers_removes_formatting() {
        assert_eq!(super::strip_inline_markers("**bold**"), "bold");
        assert_eq!(super::strip_inline_markers("*italic*"), "italic");
        assert_eq!(super::strip_inline_markers("`code`"), "code");
        assert_eq!(
            super::strip_inline_markers("hello **world**"),
            "hello world"
        );
        assert_eq!(super::strip_inline_markers("plain text"), "plain text");
    }

    #[test]
    fn render_bullet_with_bold() {
        let md = "- **bold item**";
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);

        render_markdown(&mut buf, area, md, 0);

        let row = line_text(&buf, area, 0);
        assert!(
            row.contains("bold item"),
            "bullet should contain 'bold item' without **: {row}"
        );
        assert!(!row.contains("**"), "bullet should not contain '**': {row}");
    }
}
