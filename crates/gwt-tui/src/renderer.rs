//! Renderer — converts vt100 screen cells to ratatui Buffer.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

/// Map a vt100 color to a ratatui color.
pub fn map_vt_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Map vt100 cell attributes to ratatui Modifier.
pub fn map_vt_attrs(bold: bool, italic: bool, underline: bool, inverse: bool) -> Modifier {
    let mut mods = Modifier::empty();
    if bold {
        mods |= Modifier::BOLD;
    }
    if italic {
        mods |= Modifier::ITALIC;
    }
    if underline {
        mods |= Modifier::UNDERLINED;
    }
    if inverse {
        mods |= Modifier::REVERSED;
    }
    mods
}

/// Detect URLs in text, returning (start, end) byte positions.
///
/// Matches `http://` and `https://` URLs, stopping at whitespace and
/// common delimiter characters (`<`, `>`, `"`, `{`, `}`, `|`, `\\`, `^`,
/// `` ` ``, `[`, `]`).
pub fn detect_urls(text: &str) -> Vec<(usize, usize)> {
    let mut results = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Look for "http://" or "https://"
        let remaining = &text[i..];
        let scheme_len = if remaining.starts_with("https://") {
            8
        } else if remaining.starts_with("http://") {
            7
        } else {
            i += 1;
            continue;
        };

        let start = i;
        let mut end = i + scheme_len;

        // Need at least one character after the scheme
        if end >= len || is_url_delimiter(bytes[end]) {
            i += 1;
            continue;
        }

        // Extend to end of URL
        while end < len && !is_url_delimiter(bytes[end]) {
            end += 1;
        }

        // Strip trailing punctuation that is unlikely part of the URL
        while end > start + scheme_len {
            let last = bytes[end - 1];
            if matches!(last, b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b'\'') {
                end -= 1;
            } else {
                break;
            }
        }

        if end > start + scheme_len {
            results.push((start, end));
        }
        i = end;
    }

    results
}

/// Check if a byte is a URL delimiter (whitespace or special chars).
fn is_url_delimiter(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'\t'
            | b'\n'
            | b'\r'
            | b'<'
            | b'>'
            | b'"'
            | b'{'
            | b'}'
            | b'|'
            | b'\\'
            | b'^'
            | b'`'
            | b'['
            | b']'
    )
}

/// A detected URL region with screen coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlRegion {
    pub url: String,
    pub row: u16,
    pub start_col: u16,
    pub end_col: u16,
}

/// Render a vt100 screen into a ratatui Buffer at the given area.
///
/// Returns detected URL regions with their screen coordinates so that
/// callers can implement click-to-open or hover highlighting.
pub fn render_vt_screen(screen: &vt100::Screen, buf: &mut Buffer, area: Rect) -> Vec<UrlRegion> {
    let rows = area.height.min(screen.size().0);
    let cols = area.width.min(screen.size().1);
    let mut url_regions: Vec<UrlRegion> = Vec::new();

    for row in 0..rows {
        // Build the visible text for this row to detect URLs.
        let mut row_text = String::with_capacity(cols as usize);
        // Track the mapping from byte offset to column index.
        let mut byte_to_col: Vec<u16> = Vec::with_capacity(cols as usize);

        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                let ch = cell.contents().chars().next().unwrap_or(' ');
                let byte_start = row_text.len();
                row_text.push(ch);
                let byte_end = row_text.len();
                // Map each byte of this character to the column.
                for _ in byte_start..byte_end {
                    byte_to_col.push(col);
                }
            } else {
                let byte_start = row_text.len();
                row_text.push(' ');
                let byte_end = row_text.len();
                for _ in byte_start..byte_end {
                    byte_to_col.push(col);
                }
            }
        }

        // Detect URLs in the row text.
        let urls = detect_urls(&row_text);

        // Build a set of columns that belong to a URL for fast lookup.
        let mut url_cols = std::collections::HashSet::new();
        for &(start, end) in &urls {
            let start_col_idx = byte_to_col.get(start).copied().unwrap_or(0);
            // end is exclusive byte position; map the last included byte.
            let end_col_idx = if end > 0 {
                byte_to_col.get(end - 1).copied().unwrap_or(0)
            } else {
                0
            };
            let url_str = &row_text[start..end];
            url_regions.push(UrlRegion {
                url: url_str.to_string(),
                row,
                start_col: start_col_idx,
                end_col: end_col_idx,
            });
            for c in start_col_idx..=end_col_idx {
                url_cols.insert(c);
            }
        }

        // Render cells into the buffer.
        for col in 0..cols {
            let cell = screen.cell(row, col);
            if let Some(cell) = cell {
                let x = area.x + col;
                let y = area.y + row;

                if x < buf.area().right() && y < buf.area().bottom() {
                    let buf_cell = &mut buf[(x, y)];
                    buf_cell.set_char(cell.contents().chars().next().unwrap_or(' '));

                    let is_url = url_cols.contains(&col);
                    let fg = if is_url {
                        Some(Color::Cyan)
                    } else {
                        Some(map_vt_color(cell.fgcolor()))
                    };
                    let mut mods = map_vt_attrs(
                        cell.bold(),
                        cell.italic(),
                        cell.underline() || is_url,
                        cell.inverse(),
                    );
                    if is_url {
                        mods |= Modifier::UNDERLINED;
                    }

                    buf_cell.set_style(Style {
                        fg,
                        bg: Some(map_vt_color(cell.bgcolor())),
                        underline_color: None,
                        add_modifier: mods,
                        sub_modifier: Modifier::empty(),
                    });
                }
            }
        }
    }

    url_regions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_vt_color_default() {
        assert_eq!(map_vt_color(vt100::Color::Default), Color::Reset);
    }

    #[test]
    fn map_vt_color_indexed() {
        assert_eq!(map_vt_color(vt100::Color::Idx(42)), Color::Indexed(42));
    }

    #[test]
    fn map_vt_color_rgb() {
        assert_eq!(
            map_vt_color(vt100::Color::Rgb(10, 20, 30)),
            Color::Rgb(10, 20, 30)
        );
    }

    #[test]
    fn map_vt_attrs_none() {
        assert_eq!(map_vt_attrs(false, false, false, false), Modifier::empty());
    }

    #[test]
    fn map_vt_attrs_all() {
        let m = map_vt_attrs(true, true, true, true);
        assert!(m.contains(Modifier::BOLD));
        assert!(m.contains(Modifier::ITALIC));
        assert!(m.contains(Modifier::UNDERLINED));
        assert!(m.contains(Modifier::REVERSED));
    }

    #[test]
    fn render_vt_screen_basic() {
        let parser = vt100::Parser::new(2, 3, 0);
        let screen = parser.screen();
        let area = Rect::new(0, 0, 3, 2);
        let mut buf = Buffer::empty(area);
        let regions = render_vt_screen(screen, &mut buf, area);
        assert!(regions.is_empty());
    }

    // ---- URL detection tests ----

    #[test]
    fn detect_urls_simple_https() {
        let text = "Visit https://example.com for info";
        let urls = detect_urls(text);
        assert_eq!(urls.len(), 1);
        assert_eq!(&text[urls[0].0..urls[0].1], "https://example.com");
    }

    #[test]
    fn detect_urls_simple_http() {
        let text = "Visit http://example.com now";
        let urls = detect_urls(text);
        assert_eq!(urls.len(), 1);
        assert_eq!(&text[urls[0].0..urls[0].1], "http://example.com");
    }

    #[test]
    fn detect_urls_with_path() {
        let urls = detect_urls("See https://example.com/path/to/page");
        assert_eq!(urls, vec![(4, 36)]);
    }

    #[test]
    fn detect_urls_with_query() {
        let text = "Go to https://example.com/search?q=test&page=1 done";
        let urls = detect_urls(text);
        assert_eq!(urls.len(), 1);
        assert_eq!(
            &text[urls[0].0..urls[0].1],
            "https://example.com/search?q=test&page=1"
        );
    }

    #[test]
    fn detect_urls_multiple() {
        let text = "https://a.com and https://b.com";
        let urls = detect_urls(text);
        assert_eq!(urls.len(), 2);
        assert_eq!(&text[urls[0].0..urls[0].1], "https://a.com");
        assert_eq!(&text[urls[1].0..urls[1].1], "https://b.com");
    }

    #[test]
    fn detect_urls_no_urls() {
        let urls = detect_urls("No URLs here at all");
        assert!(urls.is_empty());
    }

    #[test]
    fn detect_urls_strips_trailing_punctuation() {
        let urls = detect_urls("Check https://example.com.");
        assert_eq!(urls.len(), 1);
        let (s, e) = urls[0];
        assert_eq!(&"Check https://example.com."[s..e], "https://example.com");
    }

    #[test]
    fn detect_urls_stops_at_angle_bracket() {
        let urls = detect_urls("<https://example.com>");
        assert_eq!(urls.len(), 1);
        let (s, e) = urls[0];
        assert_eq!(&"<https://example.com>"[s..e], "https://example.com");
    }

    #[test]
    fn detect_urls_empty_string() {
        assert!(detect_urls("").is_empty());
    }

    #[test]
    fn detect_urls_scheme_only_no_match() {
        // "https://" alone with nothing after should not match
        let urls = detect_urls("just https:// here");
        assert!(urls.is_empty());
    }

    #[test]
    fn detect_urls_with_fragment() {
        let text = "See https://example.com/page#section end";
        let urls = detect_urls(text);
        assert_eq!(urls.len(), 1);
        assert_eq!(
            &text[urls[0].0..urls[0].1],
            "https://example.com/page#section"
        );
    }

    // ---- URL region coordinate tracking tests (T002) ----

    #[test]
    fn url_region_tracking_single_url() {
        // Write "https://x.co" starting at column 0 on a 20-col screen.
        let mut parser = vt100::Parser::new(1, 20, 0);
        parser.process(b"https://x.co rest");
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        let regions = render_vt_screen(parser.screen(), &mut buf, area);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].url, "https://x.co");
        assert_eq!(regions[0].row, 0);
        assert_eq!(regions[0].start_col, 0);
        assert_eq!(regions[0].end_col, 11); // 12 chars, cols 0..11
    }

    #[test]
    fn url_region_tracking_offset_url() {
        // "hi https://a.io bye" — URL starts at column 3.
        let mut parser = vt100::Parser::new(1, 30, 0);
        parser.process(b"hi https://a.io bye");
        let area = Rect::new(0, 0, 30, 1);
        let mut buf = Buffer::empty(area);
        let regions = render_vt_screen(parser.screen(), &mut buf, area);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].url, "https://a.io");
        assert_eq!(regions[0].start_col, 3);
    }

    #[test]
    fn url_region_tracking_multiple_rows() {
        let mut parser = vt100::Parser::new(2, 40, 0);
        parser.process(b"https://a.com\r\nhttps://b.com");
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        let regions = render_vt_screen(parser.screen(), &mut buf, area);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].row, 0);
        assert_eq!(regions[0].url, "https://a.com");
        assert_eq!(regions[1].row, 1);
        assert_eq!(regions[1].url, "https://b.com");
    }

    #[test]
    fn url_cells_styled_cyan_underline() {
        let mut parser = vt100::Parser::new(1, 30, 0);
        parser.process(b"go https://x.co end");
        let area = Rect::new(0, 0, 30, 1);
        let mut buf = Buffer::empty(area);
        render_vt_screen(parser.screen(), &mut buf, area);

        // Column 3 is the 'h' of "https://x.co"
        let cell = &buf[(3, 0)];
        assert_eq!(cell.fg, Color::Cyan);
        assert!(cell.modifier.contains(Modifier::UNDERLINED));

        // Column 0 is 'g' — should NOT be cyan
        let plain = &buf[(0, 0)];
        assert_ne!(plain.fg, Color::Cyan);
    }

    // ---- Alt-screen buffer tests (T011, T012) ----

    #[test]
    fn alt_screen_enter_exit_preserves_main() {
        let mut parser = vt100::Parser::new(4, 20, 0);
        // Write on main screen
        parser.process(b"main content");
        // Enter alt screen (DECSET 1049)
        parser.process(b"\x1b[?1049h");
        // Write on alt screen
        parser.process(b"alt content");
        // Verify alt-screen content is visible
        let alt_row = parser.screen().contents_between(0, 0, 1, 20);
        assert!(
            alt_row.contains("alt content"),
            "alt screen should show alt content"
        );
        // Exit alt screen (DECRST 1049)
        parser.process(b"\x1b[?1049l");
        // Main screen content should be restored
        let main_row = parser.screen().contents_between(0, 0, 1, 20);
        assert!(
            main_row.contains("main content"),
            "main screen should be restored after alt-screen exit"
        );
    }

    #[test]
    fn alt_screen_cursor_position_restores() {
        let mut parser = vt100::Parser::new(10, 40, 0);
        // Move cursor to row 3, col 5 (1-based in ANSI: row 4, col 6)
        parser.process(b"\x1b[4;6H");
        let (pre_row, pre_col) = parser.screen().cursor_position();
        assert_eq!(pre_row, 3);
        assert_eq!(pre_col, 5);

        // Enter alt screen
        parser.process(b"\x1b[?1049h");
        // Move cursor elsewhere
        parser.process(b"\x1b[1;1H");
        let (alt_row, alt_col) = parser.screen().cursor_position();
        assert_eq!(alt_row, 0);
        assert_eq!(alt_col, 0);

        // Exit alt screen
        parser.process(b"\x1b[?1049l");
        let (post_row, post_col) = parser.screen().cursor_position();
        assert_eq!(post_row, 3, "cursor row should be restored");
        assert_eq!(post_col, 5, "cursor col should be restored");
    }
}
