//! Screen modules — one per management tab, plus overlays.

pub mod branches;
pub mod cleanup_confirm;
pub mod cleanup_progress;
pub mod confirm;
pub mod discussion_resume;
pub mod docker_progress;
pub mod error;
pub mod git_view;
pub mod help;
pub mod initialization;
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

use crate::theme;

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
        theme::style::selected_item()
    } else {
        theme::style::text()
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

/// Build a tab title line for embedding in a Block title.
///
/// Active tab is yellow/bold, inactive tabs are gray, separated by │.
pub fn build_tab_title(labels: &[&str], active: usize) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("│", theme::style::tab_separator()));
        }
        if i == active {
            spans.push(Span::styled(
                format!(" {} ", label),
                theme::style::tab_active(),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {} ", label),
                theme::style::tab_inactive(),
            ));
        }
    }
    Line::from(spans)
}

/// Selection prefix: accent bar when selected, space otherwise.
pub fn selection_prefix(is_selected: bool) -> Span<'static> {
    if is_selected {
        Span::styled(
            theme::icon::LEFT_ACCENT,
            Style::default().fg(theme::color::ACTIVE),
        )
    } else {
        Span::raw(" ")
    }
}

/// Render an empty list placeholder (borderless).
pub fn render_empty_list(frame: &mut Frame, area: Rect, has_data: bool, noun: &str) {
    let msg = if has_data {
        format!("No matching {}", noun)
    } else {
        format!("No {} loaded", noun)
    };
    let p = Paragraph::new(msg)
        .block(Block::default())
        .style(theme::style::muted_text());
    frame.render_widget(p, area);
}
