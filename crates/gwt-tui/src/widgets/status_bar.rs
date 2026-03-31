//! Status bar widget: bottom bar showing context and key hints

use ratatui::prelude::*;

use crate::model::{ActiveLayer, Model};

/// Render the status bar.
pub fn render(model: &Model, buf: &mut Buffer, area: Rect) {
    let layer_label = match model.active_layer {
        ActiveLayer::Main => "Main",
        ActiveLayer::Management => "Management",
    };

    let session_info = if model.session_tabs.is_empty() {
        String::new()
    } else {
        format!(
            " | Session {}/{}",
            model.active_session + 1,
            model.session_tabs.len()
        )
    };

    let hints = " Ctrl+G,Ctrl+G: Toggle Layer | Ctrl+G,?: Help";

    let left = format!(" [{layer_label}]{session_info}");
    let right_width = hints.len() as u16;
    let left_width = area.width.saturating_sub(right_width);

    let left_span = Span::styled(left, Style::default().fg(Color::White).bg(Color::DarkGray));
    buf.set_span(area.x, area.y, &left_span, left_width);

    let right_span = Span::styled(hints, Style::default().fg(Color::Gray).bg(Color::DarkGray));
    let right_x = area.x + left_width;
    buf.set_span(right_x, area.y, &right_span, right_width);

    // Fill any gap with background
    for x in (area.x + left_span.width() as u16)..right_x {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_style(Style::default().bg(Color::DarkGray));
        }
    }
}
