//! Screen modules — one per management tab, plus overlays.

pub mod branches;
pub mod confirm;
pub mod docker_progress;
pub mod error;
pub mod git_view;
pub mod issues;
pub mod logs;
pub mod port_select;
pub mod pr_dashboard;
pub mod profiles;
pub mod service_select;
pub mod settings;
pub mod specs;
pub mod versions;
pub mod wizard;

use ratatui::prelude::*;
use ratatui::widgets::*;

/// Clamp a selection index to [0, len-1]. Sets to 0 if len is 0.
pub fn clamp_index(selected: &mut usize, len: usize) {
    if len == 0 {
        *selected = 0;
    } else if *selected >= len {
        *selected = len - 1;
    }
}

/// Move selection up with wrapping.
pub fn move_up(selected: &mut usize, len: usize) {
    if len == 0 {
        return;
    }
    *selected = if *selected == 0 {
        len - 1
    } else {
        *selected - 1
    };
}

/// Move selection down with wrapping.
pub fn move_down(selected: &mut usize, len: usize) {
    if len == 0 {
        return;
    }
    *selected = (*selected + 1) % len;
}

/// Style for a list item: highlighted if selected, default otherwise.
pub fn list_item_style(is_selected: bool) -> Style {
    if is_selected {
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    }
}

/// Calculate a centered rect within an area.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// Render an empty list placeholder.
pub fn render_empty_list(frame: &mut Frame, area: Rect, has_data: bool, noun: &str) {
    let msg = if has_data {
        format!("No matching {}", noun)
    } else {
        format!("No {} loaded", noun)
    };
    let p = Paragraph::new(msg)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, area);
}
