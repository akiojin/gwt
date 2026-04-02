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

/// Render a vt100 screen into a ratatui Buffer at the given area.
pub fn render_vt_screen(screen: &vt100::Screen, buf: &mut Buffer, area: Rect) {
    let rows = area.height.min(screen.size().0);
    let cols = area.width.min(screen.size().1);

    for row in 0..rows {
        for col in 0..cols {
            let cell = screen.cell(row, col);
            if let Some(cell) = cell {
                let x = area.x + col;
                let y = area.y + row;

                if x < buf.area().right() && y < buf.area().bottom() {
                    let buf_cell = &mut buf[(x, y)];
                    buf_cell.set_char(cell.contents().chars().next().unwrap_or(' '));
                    buf_cell.set_style(Style {
                        fg: Some(map_vt_color(cell.fgcolor())),
                        bg: Some(map_vt_color(cell.bgcolor())),
                        underline_color: None,
                        add_modifier: map_vt_attrs(
                            cell.bold(),
                            cell.italic(),
                            cell.underline(),
                            cell.inverse(),
                        ),
                        sub_modifier: Modifier::empty(),
                    });
                }
            }
        }
    }
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
        render_vt_screen(screen, &mut buf, area);
        // Should not panic — cells are blank
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
}
