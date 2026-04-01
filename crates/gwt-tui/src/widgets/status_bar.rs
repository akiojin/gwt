//! Status bar widget: bottom bar showing context and key hints

use ratatui::prelude::*;

use crate::model::{ActiveLayer, ManagementTab, Model};

/// Render the status bar.
pub fn render(model: &Model, buf: &mut Buffer, area: Rect) {
    // Fill background
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_style(Style::default().bg(Color::DarkGray));
            cell.set_char(' ');
        }
    }

    let left = match model.active_layer {
        ActiveLayer::Main => {
            if model.session_tabs.is_empty() {
                " Ctrl+G,c: Shell | Ctrl+G,n: Agent".to_string()
            } else {
                let tab = &model.session_tabs[model.active_session];
                let branch = tab.branch.as_deref().unwrap_or("");
                if branch.is_empty() {
                    format!(" {}", tab.name)
                } else {
                    format!(" {} | {}", tab.name, branch)
                }
            }
        }
        ActiveLayer::Management => {
            let tab_name = model.management_tab.label();
            format!(" {tab_name}")
        }
    };

    let hints = match model.active_layer {
        ActiveLayer::Main => " Ctrl+G,Ctrl+G: Manage | Ctrl+G,x: Close | Ctrl+C×2: Quit ",
        ActiveLayer::Management => " Tab: Switch | Ctrl+G,Ctrl+G: Terminal | Ctrl+C×2: Quit ",
    };

    let right_width = hints.len() as u16;
    let left_width = area.width.saturating_sub(right_width);

    let left_span = Span::styled(left, Style::default().fg(Color::White).bg(Color::DarkGray));
    buf.set_span(area.x, area.y, &left_span, left_width);

    let right_span = Span::styled(hints, Style::default().fg(Color::Gray).bg(Color::DarkGray));
    let right_x = area.x + left_width;
    buf.set_span(right_x, area.y, &right_span, right_width);
}
