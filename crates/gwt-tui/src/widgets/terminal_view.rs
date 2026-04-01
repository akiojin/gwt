//! Terminal view widget: renders VT100 parser screen into ratatui buffer
//!
//! Phase 2: will use renderer.rs for full VT100 → ratatui conversion.

use ratatui::prelude::*;

/// Render a VT100 parser screen into the given buffer area.
pub fn render(_buf: &mut Buffer, _area: Rect, _parser: Option<&vt100::Parser>) {
    // Phase 2: delegate to renderer module for VT100 cell conversion
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::prelude::*;

    #[test]
    fn render_none_smoke() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, None);
        // No-op, no panic
    }

    #[test]
    fn render_with_parser_smoke() {
        let parser = vt100::Parser::new(24, 80, 0);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, Some(&parser));
        // No-op (Phase 2 stub), no panic
    }

    #[test]
    fn render_zero_area_smoke() {
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, None);
        // No panic with zero-sized area
    }
}
