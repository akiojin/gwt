//! Agent/Shell pane screen
//!
//! Renders VT100 terminal output for an agent or shell session.

use ratatui::prelude::*;

use crate::model::SelectionPoint;

/// Render a terminal pane using the VT100 parser screen.
/// Returns the cursor position (x, y) relative to the frame if visible.
pub fn render(
    buf: &mut Buffer,
    area: Rect,
    parser: Option<&vt100::Parser>,
    selection: Option<(SelectionPoint, SelectionPoint)>,
) -> Option<(u16, u16)> {
    if let Some(parser) = parser {
        let screen = parser.screen();
        crate::renderer::render_vt100_screen(buf, area, screen);
        render_selection(buf, area, selection, 0);

        // Return cursor position if visible
        if !screen.hide_cursor() {
            let (row, col) = screen.cursor_position();
            let x = area.x + col;
            let y = area.y + row;
            if x < area.right() && y < area.bottom() {
                return Some((x, y));
            }
        }
        None
    } else {
        let text = ratatui::widgets::Paragraph::new("Starting...").alignment(Alignment::Center);
        ratatui::widgets::Widget::render(text, area, buf);
        None
    }
}

/// Render a large history parser using a manual viewport offset.
pub fn render_history(
    buf: &mut Buffer,
    area: Rect,
    parser: &vt100::Parser,
    view_top: u16,
    selection: Option<(SelectionPoint, SelectionPoint)>,
) -> Option<(u16, u16)> {
    let screen = parser.screen();
    let rows = area.height as usize;
    let cols = area.width as usize;

    for row in 0..rows {
        for col in 0..cols {
            let source_row = view_top.saturating_add(row as u16);
            let source_col = col as u16;
            let buf_x = area.x + col as u16;
            let buf_y = area.y + row as u16;

            if let Some(cell) = screen.cell(source_row, source_col) {
                if let Some(buf_cell) = buf.cell_mut((buf_x, buf_y)) {
                    let ch = cell.contents();
                    if ch.is_empty() {
                        buf_cell.set_char(' ');
                    } else {
                        buf_cell.set_symbol(&ch);
                    }
                    buf_cell.set_style(crate::renderer::vt100_to_ratatui_style(cell));
                }
            }
        }
    }

    render_selection(buf, area, selection, view_top);

    None
}

pub fn selected_text(parser: &vt100::Parser, start: SelectionPoint, end: SelectionPoint) -> String {
    let screen = parser.screen();
    let (_, cols) = screen.size();
    let (start, end) = normalize_selection(start, end);
    let end_col = end.col.saturating_add(1).min(cols);
    screen.contents_between(start.row, start.col, end.row, end_col)
}

fn normalize_selection(
    start: SelectionPoint,
    end: SelectionPoint,
) -> (SelectionPoint, SelectionPoint) {
    if (start.row, start.col) <= (end.row, end.col) {
        (start, end)
    } else {
        (end, start)
    }
}

fn render_selection(
    buf: &mut Buffer,
    area: Rect,
    selection: Option<(SelectionPoint, SelectionPoint)>,
    view_top: u16,
) {
    let Some((start, end)) = selection else {
        return;
    };
    let (start, end) = normalize_selection(start, end);

    if end.row < view_top {
        return;
    }

    let visible_start = start.row.max(view_top);
    let visible_end = end
        .row
        .min(view_top.saturating_add(area.height.saturating_sub(1)));

    for row in visible_start..=visible_end {
        let row_in_view = row.saturating_sub(view_top);
        let col_start = if row == start.row { start.col } else { 0 };
        let col_end = if row == end.row {
            end.col
        } else {
            area.width.saturating_sub(1)
        };
        for col in col_start..=col_end {
            let x = area.x + col;
            let y = area.y + row_in_view;
            if x < area.right() && y < area.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    let style = cell.style().add_modifier(Modifier::REVERSED);
                    cell.set_style(style);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    #[test]
    fn render_none_shows_starting_placeholder() {
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        let result = render(&mut buf, area, None, None);
        assert!(result.is_none());
        // "Starting..." should appear somewhere in the buffer
        let text: String = (0..area.width)
            .map(|x| {
                buf.cell((x, area.y))
                    .map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))
            })
            .collect();
        // The paragraph is center-aligned, so it may have padding
        assert!(
            text.contains("Starting..."),
            "Expected 'Starting...' in buffer, got: {:?}",
            text.trim()
        );
    }

    #[test]
    fn render_with_parser_no_cursor() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"Hello, world!");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let result = render(&mut buf, area, Some(&parser), None);
        // Cursor is visible by default in vt100, so we get Some
        assert!(result.is_some());
    }

    #[test]
    fn render_with_parser_hidden_cursor() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        // ESC[?25l hides cursor
        parser.process(b"\x1b[?25lHidden cursor");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let result = render(&mut buf, area, Some(&parser), None);
        assert!(result.is_none());
    }

    #[test]
    fn render_with_parser_cursor_position() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        // Move cursor to row 2, col 5 (1-based in ANSI)
        parser.process(b"\x1b[3;6HX");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let result = render(&mut buf, area, Some(&parser), None);
        if let Some((x, y)) = result {
            // Cursor is after the 'X' we wrote at (row=2, col=6) in 0-based
            assert!(x < area.right());
            assert!(y < area.bottom());
        }
    }

    #[test]
    fn render_with_parser_text_appears_in_buffer() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"TestOutput");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, Some(&parser), None);
        let row: String = (0..10)
            .map(|x| {
                buf.cell((x, 0))
                    .map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))
            })
            .collect();
        assert_eq!(row, "TestOutput");
    }

    #[test]
    fn selected_text_returns_single_line_range() {
        let mut parser = vt100::Parser::new(4, 20, 10);
        parser.process(b"hello world");
        let text = selected_text(
            &parser,
            SelectionPoint { row: 0, col: 0 },
            SelectionPoint { row: 0, col: 4 },
        );
        assert_eq!(text, "hello");
    }

    #[test]
    fn render_with_selection_reverses_selected_cells() {
        let mut parser = vt100::Parser::new(4, 20, 10);
        parser.process(b"hello");
        let area = Rect::new(0, 0, 20, 4);
        let mut buf = Buffer::empty(area);
        render(
            &mut buf,
            area,
            Some(&parser),
            Some((
                SelectionPoint { row: 0, col: 1 },
                SelectionPoint { row: 0, col: 3 },
            )),
        );
        assert!(buf[(1, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(2, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(3, 0)].modifier.contains(Modifier::REVERSED));
    }
}
