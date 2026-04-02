//! Tab bar widget: renders the 2-layer tab bar (Main sessions / Management tabs)

use ratatui::prelude::*;
use ratatui::widgets::Tabs;

use crate::model::{ActiveLayer, ManagementTab, Model, SessionLayoutMode};

/// Unicode indicator for active layer
const MAIN_ICON: &str = "\u{25B6}"; // Right-pointing triangle
const MGMT_ICON: &str = "\u{2630}"; // Trigram (hamburger)

/// Render the tab bar for the current layer.
pub fn render(model: &Model, buf: &mut Buffer, area: Rect) {
    // Fill background for tab bar
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_style(Style::default().bg(Color::DarkGray));
            cell.set_char(' ');
        }
    }

    match model.active_layer {
        ActiveLayer::Initialization => {} // No tab bar during initialization
        ActiveLayer::Main => render_main_tabs(model, buf, area),
        ActiveLayer::Management => render_management_tabs(model, buf, area),
    }
}

fn render_main_tabs(model: &Model, buf: &mut Buffer, area: Rect) {
    let prefix = Span::styled(
        format!(" {MAIN_ICON} "),
        Style::default().fg(Color::Yellow).bg(Color::DarkGray),
    );
    buf.set_span(area.x, area.y, &prefix, 4);

    let tabs_area = Rect {
        x: area.x + 4,
        y: area.y,
        width: area.width.saturating_sub(4),
        height: area.height,
    };

    if model.session_tabs.is_empty() {
        let hint = Span::styled(
            "(no sessions — Enter on Branches: agent | Ctrl+G,c: shell)",
            Style::default().fg(Color::Gray).bg(Color::DarkGray),
        );
        buf.set_span(tabs_area.x, tabs_area.y, &hint, tabs_area.width);
        return;
    }

    if model.session_layout_mode == SessionLayoutMode::Grid {
        let focus_name = &model.session_tabs[model.active_session].name;
        let summary = Span::styled(
            format!(
                "{} sessions | focus: {} | Ctrl+G,z: maximize",
                model.session_tabs.len(),
                focus_name
            ),
            Style::default().fg(Color::Gray).bg(Color::DarkGray),
        );
        buf.set_span(tabs_area.x, tabs_area.y, &summary, tabs_area.width);
        return;
    }

    let titles: Vec<Line<'_>> = model
        .session_tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            if i == model.active_session {
                Line::from(Span::styled(
                    format!(" {} ", tab.name),
                    Style::default().fg(Color::Black).bg(Color::Yellow).bold(),
                ))
            } else {
                Line::from(Span::styled(
                    format!(" {} ", tab.name),
                    Style::default().fg(Color::White).bg(Color::DarkGray),
                ))
            }
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(model.active_session)
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Yellow).bold())
        .style(Style::default().bg(Color::DarkGray))
        .divider(Span::styled(" ", Style::default().bg(Color::DarkGray)));

    ratatui::widgets::Widget::render(tabs, tabs_area, buf);
}

fn render_management_tabs(model: &Model, buf: &mut Buffer, area: Rect) {
    let prefix = Span::styled(
        format!(" {MGMT_ICON} "),
        Style::default().fg(Color::Cyan).bg(Color::DarkGray),
    );
    buf.set_span(area.x, area.y, &prefix, 4);

    let tabs_area = Rect {
        x: area.x + 4,
        y: area.y,
        width: area.width.saturating_sub(4),
        height: area.height,
    };

    let titles: Vec<Line<'_>> = ManagementTab::ALL
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            if i == model.management_tab.index() {
                Line::from(Span::styled(
                    format!(" {} ", tab.label()),
                    Style::default().fg(Color::Black).bg(Color::Cyan).bold(),
                ))
            } else {
                Line::from(Span::styled(
                    format!(" {} ", tab.label()),
                    Style::default().fg(Color::White).bg(Color::DarkGray),
                ))
            }
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(model.management_tab.index())
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan).bold())
        .style(Style::default().bg(Color::DarkGray))
        .divider(Span::styled(" ", Style::default().bg(Color::DarkGray)));

    ratatui::widgets::Widget::render(tabs, tabs_area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SessionLayoutMode, SessionStatus, SessionTab, SessionTabType};
    use gwt_core::terminal::AgentColor;
    use std::path::PathBuf;

    #[test]
    fn management_tab_renders_without_panic() {
        let model = Model::new(PathBuf::from("/tmp/test"));
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 1));
        render(&model, &mut buf, Rect::new(0, 0, 80, 1));
    }

    #[test]
    fn main_tab_bar_shows_grid_summary_in_grid_mode() {
        let mut model = Model::new(PathBuf::from("/tmp/test"));
        model.active_layer = ActiveLayer::Main;
        model.session_layout_mode = SessionLayoutMode::Grid;
        model.session_tabs.push(SessionTab {
            pane_id: "p1".into(),
            name: "Agent #1".into(),
            tab_type: SessionTabType::Agent,
            color: AgentColor::Blue,
            status: SessionStatus::Running,
            branch: Some("feature/test".into()),
            spec_id: None,
        });
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        render(&model, &mut buf, area);
        let text: String = (0..80)
            .map(|x| {
                buf.cell((x, 0))
                    .map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))
            })
            .collect();
        assert!(text.contains("focus: Agent #1"), "got: {text:?}");
    }
}
