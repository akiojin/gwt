//! Progress modal overlay widget

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::model::ProgressState;

/// Render a centered progress modal overlay.
pub fn render(buf: &mut Buffer, area: Rect, state: &ProgressState) {
    let modal_width = 50.min(area.width.saturating_sub(4));
    let modal_height = 5.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
    let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect::new(x, y, modal_width, modal_height);

    // Clear background
    Clear.render(modal_area, buf);

    let detail = state.detail.as_deref().unwrap_or("");
    let percent_str = state
        .percent
        .map(|p| format!(" ({p}%)"))
        .unwrap_or_default();

    let text = format!("{}{}\n{}", state.title, percent_str, detail);
    let para = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(" Progress "),
        )
        .wrap(Wrap { trim: true });

    ratatui::widgets::Widget::render(para, modal_area, buf);
}
