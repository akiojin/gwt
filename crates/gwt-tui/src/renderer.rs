//! Renderer — converts vt100 screen cells to ratatui Buffer.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

use crate::model::{TerminalCell, TerminalSelection};

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
        // Skip to the next valid UTF-8 char boundary.
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        // Look for "http://" or "https://"
        let remaining = &text[i..];
        let scheme_len = if remaining.starts_with("https://") {
            8
        } else if remaining.starts_with("http://") {
            7
        } else {
            i += remaining.chars().next().map_or(1, |c| c.len_utf8());
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

fn build_row_text(screen: &vt100::Screen, row: u16, cols: u16) -> (String, Vec<u16>) {
    let mut row_text = String::with_capacity(cols as usize);
    let mut byte_to_col: Vec<u16> = Vec::with_capacity(cols as usize);

    for col in 0..cols {
        let symbol = screen
            .cell(row, col)
            .map(|cell| renderable_cell_symbol(cell, col, cols))
            .unwrap_or_else(|| " ".to_string());
        let byte_start = row_text.len();
        row_text.push_str(&symbol);
        let byte_end = row_text.len();
        for _ in byte_start..byte_end {
            byte_to_col.push(col);
        }
    }

    (row_text, byte_to_col)
}

fn renderable_cell_symbol(cell: &vt100::Cell, col: u16, cols: u16) -> String {
    if cell.is_wide_continuation() {
        return " ".to_string();
    }

    let symbol = cell.contents();
    let symbol = if symbol.is_empty() {
        " ".to_string()
    } else {
        symbol
    };
    let symbol_width = if cell.is_wide() { 2 } else { 1 };
    if symbol_width > 1 && usize::from(col).saturating_add(symbol_width) > usize::from(cols) {
        " ".to_string()
    } else {
        symbol
    }
}

/// Collect detected URL regions for the visible screen area.
///
/// Wrapped rows are joined into a single logical line before URL detection so
/// multi-row URLs remain clickable across the full wrapped span.
pub fn collect_url_regions(screen: &vt100::Screen, area: Rect) -> Vec<UrlRegion> {
    let rows = area.height.min(screen.size().0);
    let cols = area.width.min(screen.size().1);
    let mut url_regions: Vec<UrlRegion> = Vec::new();
    let mut row = 0;

    while row < rows {
        let mut logical_text = String::new();
        let mut byte_to_coord: Vec<(u16, u16)> = Vec::new();

        loop {
            let current_row = row;
            let (row_text, byte_to_col) = build_row_text(screen, current_row, cols);
            logical_text.push_str(&row_text);
            byte_to_coord.extend(byte_to_col.into_iter().map(|col| (current_row, col)));

            row += 1;
            if row >= rows || !screen.row_wrapped(current_row) {
                break;
            }
        }

        for (start, end) in detect_urls(&logical_text) {
            let url = logical_text[start..end].to_string();
            let mut segment_row = None;
            let mut segment_start_col = 0;
            let mut segment_end_col = 0;

            for &(coord_row, coord_col) in byte_to_coord.iter().take(end).skip(start) {
                match segment_row {
                    Some(active_row) if active_row == coord_row => {
                        segment_end_col = coord_col;
                    }
                    Some(active_row) => {
                        url_regions.push(UrlRegion {
                            url: url.clone(),
                            row: active_row,
                            start_col: segment_start_col,
                            end_col: segment_end_col,
                        });
                        segment_row = Some(coord_row);
                        segment_start_col = coord_col;
                        segment_end_col = coord_col;
                    }
                    None => {
                        segment_row = Some(coord_row);
                        segment_start_col = coord_col;
                        segment_end_col = coord_col;
                    }
                }
            }

            if let Some(active_row) = segment_row {
                url_regions.push(UrlRegion {
                    url,
                    row: active_row,
                    start_col: segment_start_col,
                    end_col: segment_end_col,
                });
            }
        }
    }

    url_regions
}

/// Render a vt100 screen into a ratatui Buffer at the given area.
///
/// Returns detected URL regions with their screen coordinates so that
/// callers can implement click-to-open or hover highlighting.
pub fn render_vt_screen(screen: &vt100::Screen, buf: &mut Buffer, area: Rect) -> Vec<UrlRegion> {
    render_vt_screen_with_selection(screen, buf, area, None)
}

/// Render a vt100 screen into a ratatui Buffer with an optional selection overlay.
pub fn render_vt_screen_with_selection(
    screen: &vt100::Screen,
    buf: &mut Buffer,
    area: Rect,
    selection: Option<TerminalSelection>,
) -> Vec<UrlRegion> {
    let url_regions = collect_url_regions(screen, area);
    render_vt_screen_with_selection_and_urls(screen, buf, area, selection, &url_regions);
    url_regions
}

/// Render a vt100 screen into a ratatui Buffer with precomputed URL regions.
pub fn render_vt_screen_with_selection_and_urls(
    screen: &vt100::Screen,
    buf: &mut Buffer,
    area: Rect,
    selection: Option<TerminalSelection>,
    url_regions: &[UrlRegion],
) {
    let rows = area.height.min(screen.size().0);
    let cols = area.width.min(screen.size().1);
    let mut url_cells = std::collections::HashSet::new();
    for region in url_regions {
        for col in region.start_col..=region.end_col {
            url_cells.insert((region.row, col));
        }
    }

    for row in 0..rows {
        // Render cells into the buffer.
        for col in 0..cols {
            let cell = screen.cell(row, col);
            if let Some(cell) = cell {
                let x = area.x + col;
                let y = area.y + row;

                if x < buf.area().right() && y < buf.area().bottom() {
                    if cell.is_wide_continuation() {
                        buf[(x, y)].reset();
                        continue;
                    }

                    let covered_end = (col + if cell.is_wide() { 2 } else { 1 }).min(cols);
                    let buf_cell = &mut buf[(x, y)];
                    buf_cell.reset();
                    buf_cell.set_symbol(&renderable_cell_symbol(cell, col, cols));

                    let is_url = (col..covered_end)
                        .any(|covered_col| url_cells.contains(&(row, covered_col)));
                    let is_selected = selection
                        .map(|selection| {
                            (col..covered_end)
                                .any(|covered_col| selection_contains(selection, row, covered_col))
                        })
                        .unwrap_or(false);
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
                    if is_selected {
                        mods |= Modifier::REVERSED;
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
}

fn selection_contains(selection: TerminalSelection, row: u16, col: u16) -> bool {
    let (start, end) = normalize_selection(selection);
    if row < start.row || row > end.row {
        return false;
    }
    if start.row == end.row {
        return row == start.row && col >= start.col && col <= end.col;
    }
    if row == start.row {
        return col >= start.col;
    }
    if row == end.row {
        return col <= end.col;
    }
    true
}

fn normalize_selection(selection: TerminalSelection) -> (TerminalCell, TerminalCell) {
    let anchor = selection.anchor;
    let focus = selection.focus;
    if (anchor.row, anchor.col) <= (focus.row, focus.col) {
        (anchor, focus)
    } else {
        (focus, anchor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Cell;

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
    fn detect_urls_with_emoji_prefix() {
        // Emoji like 🔍 is 4 bytes — must not panic on byte-boundary checks.
        let text = "  🔍  Resolving https://example.com done";
        let urls = detect_urls(text);
        assert_eq!(urls.len(), 1);
        assert_eq!(&text[urls[0].0..urls[0].1], "https://example.com");
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
    fn url_region_tracking_wrapped_url_spans_multiple_rows() {
        let url = "https://example.com/docs";
        let mut parser = vt100::Parser::new(2, 12, 0);
        parser.process(url.as_bytes());
        let area = Rect::new(0, 0, 12, 2);
        let regions = collect_url_regions(parser.screen(), area);

        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].url, url);
        assert_eq!(regions[1].url, url);
        assert_eq!(regions[0].row, 0);
        assert_eq!(regions[1].row, 1);
        assert_eq!(regions[0].start_col, 0);
        assert_eq!(regions[1].start_col, 0);
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

    #[test]
    fn wrapped_url_cells_styled_cyan_underline_across_rows() {
        let mut parser = vt100::Parser::new(2, 12, 0);
        parser.process(b"https://example.com/docs");
        let area = Rect::new(0, 0, 12, 2);
        let mut buf = Buffer::empty(area);
        render_vt_screen(parser.screen(), &mut buf, area);

        let first_row = &buf[(0, 0)];
        let second_row = &buf[(0, 1)];
        assert_eq!(first_row.fg, Color::Cyan);
        assert_eq!(second_row.fg, Color::Cyan);
        assert!(first_row.modifier.contains(Modifier::UNDERLINED));
        assert!(second_row.modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn render_vt_screen_tracks_wrapped_url_segments() {
        let mut parser = vt100::Parser::new(2, 12, 0);
        parser.process(b"https://example.com/path");
        let area = Rect::new(0, 0, 12, 2);
        let mut buf = Buffer::empty(area);

        let regions = render_vt_screen(parser.screen(), &mut buf, area);

        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].url, "https://example.com/path");
        assert_eq!(regions[0].row, 0);
        assert_eq!(regions[0].start_col, 0);
        assert_eq!(regions[0].end_col, 11);
        assert_eq!(regions[1].url, "https://example.com/path");
        assert_eq!(regions[1].row, 1);
        assert_eq!(regions[1].start_col, 0);
        assert!(regions[1].end_col > 0);
        assert_eq!(buf[(0, 1)].fg, Color::Cyan);
        assert!(buf[(0, 1)].modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn render_vt_screen_preserves_full_emoji_grapheme() {
        let mut parser = vt100::Parser::new(1, 6, 0);
        parser.process("⭐️ ok".as_bytes());
        let area = Rect::new(0, 0, 6, 1);
        let mut buf = Buffer::empty(area);

        render_vt_screen(parser.screen(), &mut buf, area);

        assert_eq!(
            buf[(0, 0)].symbol(),
            "⭐️",
            "renderer should preserve the full grapheme cluster stored in the vt100 cell",
        );
    }

    #[test]
    fn render_vt_screen_drops_wide_char_when_visible_area_crops_trailing_cell() {
        let mut parser = vt100::Parser::new(1, 6, 0);
        parser.process("abcdあ".as_bytes());
        let area = Rect::new(0, 0, 5, 1);
        let mut buf = Buffer::empty(area);

        render_vt_screen(parser.screen(), &mut buf, area);

        assert_eq!(
            buf[(4, 0)].symbol(),
            " ",
            "renderer should avoid drawing a half-visible wide character at the right edge",
        );
    }

    #[test]
    fn render_vt_screen_keeps_wide_continuation_cells_hidden() {
        let mut parser = vt100::Parser::new(1, 3, 0);
        parser.process("\x1b[31mあ".as_bytes());
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::filled(area, ratatui::buffer::Cell::new("x"));

        render_vt_screen(parser.screen(), &mut buf, area);

        assert_eq!(buf[(0, 0)].symbol(), "あ");
        assert_eq!(
            buf[(1, 0)],
            ratatui::buffer::Cell::default(),
            "wide continuation cells should stay hidden so the backend does not overdraw the glyph",
        );
    }

    #[test]
    fn render_vt_screen_selection_on_wide_continuation_styles_leading_glyph() {
        let mut parser = vt100::Parser::new(1, 3, 0);
        parser.process("あ".as_bytes());
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);

        render_vt_screen_with_selection(
            parser.screen(),
            &mut buf,
            area,
            Some(TerminalSelection {
                anchor: TerminalCell { row: 0, col: 1 },
                focus: TerminalCell { row: 0, col: 1 },
            }),
        );

        assert!(
            buf[(0, 0)].modifier.contains(Modifier::REVERSED),
            "selecting the trailing half of a wide glyph should still style the visible glyph",
        );
    }

    #[test]
    fn render_vt_screen_emits_trailing_clear_for_cjk_wide_glyph_diff() {
        let area = Rect::new(0, 0, 3, 1);
        let mut previous = Buffer::empty(area);
        previous.set_string(0, 0, "abc", Style::default());

        let mut parser = vt100::Parser::new(1, 3, 0);
        parser.process("aあ".as_bytes());
        let mut next = previous.clone();
        render_vt_screen(parser.screen(), &mut next, area);

        let updates = previous.diff(&next);
        assert!(
            updates
                .iter()
                .any(|(x, y, cell)| (*x, *y, cell.symbol()) == (2, 0, " ")),
            "wide CJK glyph redraw should explicitly clear the trailing cell so full-screen updates do not leave stale text behind",
        );
    }

    #[test]
    fn render_vt_screen_orders_trailing_clear_before_wide_glyph_redraw() {
        let area = Rect::new(0, 0, 3, 1);
        let mut previous = Buffer::empty(area);
        previous.set_string(0, 0, "abc", Style::default());

        let mut parser = vt100::Parser::new(1, 3, 0);
        parser.process("aあ".as_bytes());
        let mut next = previous.clone();
        render_vt_screen(parser.screen(), &mut next, area);

        let updates = previous.diff(&next);
        assert_eq!(
            updates,
            [
                (2, 0, &Cell::new(" ")),
                (1, 0, &Cell::new("あ")),
            ],
            "wide glyph redraws should clear the trailing cell before redrawing the visible glyph so terminal backends do not leave a visual gap",
        );
    }

    #[test]
    fn render_vt_screen_with_selection_reverses_selected_cells() {
        let mut parser = vt100::Parser::new(1, 12, 0);
        parser.process(b"alpha beta");
        let area = Rect::new(0, 0, 12, 1);
        let mut buf = Buffer::empty(area);

        render_vt_screen_with_selection(
            parser.screen(),
            &mut buf,
            area,
            Some(TerminalSelection {
                anchor: TerminalCell { row: 0, col: 0 },
                focus: TerminalCell { row: 0, col: 4 },
            }),
        );

        assert!(buf[(0, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(4, 0)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(6, 0)].modifier.contains(Modifier::REVERSED));
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
