//! Tab bar widget: renders the 2-layer tab bar (Main sessions / Management tabs)

use ratatui::prelude::*;
use ratatui::widgets::Tabs;

use crate::model::{ActiveLayer, ManagementTab, Model};

/// Unicode indicator for active layer
const MAIN_ICON: &str = "\u{25B6}"; // Right-pointing triangle
const MGMT_ICON: &str = "\u{2630}"; // Trigram (hamburger)

/// Render the tab bar for the current layer.
pub fn render(model: &Model, buf: &mut Buffer, area: Rect) {
    match model.active_layer {
        ActiveLayer::Main => render_main_tabs(model, buf, area),
        ActiveLayer::Management => render_management_tabs(model, buf, area),
    }
}

fn render_main_tabs(model: &Model, buf: &mut Buffer, area: Rect) {
    if model.session_tabs.is_empty() {
        let line = Line::from(vec![
            Span::styled(
                format!(" {MAIN_ICON} Sessions "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(" (no sessions) ", Style::default().fg(Color::DarkGray)),
        ]);
        buf.set_line(area.x, area.y, &line, area.width);
        return;
    }

    let titles: Vec<Line<'_>> = model
        .session_tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let marker = if i == model.active_session {
                "*"
            } else {
                " "
            };
            Line::from(format!("{marker}{}{marker}", tab.name))
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(model.active_session)
        .highlight_style(Style::default().fg(Color::Yellow).bold())
        .divider(Span::raw("|"));

    ratatui::widgets::Widget::render(tabs, area, buf);
}

fn render_management_tabs(model: &Model, buf: &mut Buffer, area: Rect) {
    let titles: Vec<Line<'_>> = ManagementTab::ALL
        .iter()
        .map(|tab| Line::from(tab.label()))
        .collect();

    let tabs = Tabs::new(titles)
        .select(model.management_tab.index())
        .highlight_style(Style::default().fg(Color::Cyan).bold())
        .divider(Span::raw("|"));

    // Prefix with management icon
    let prefix = Span::styled(
        format!(" {MGMT_ICON} "),
        Style::default().fg(Color::Cyan),
    );
    buf.set_span(area.x, area.y, &prefix, 4);

    let tabs_area = Rect {
        x: area.x + 4,
        y: area.y,
        width: area.width.saturating_sub(4),
        height: area.height,
    };
    ratatui::widgets::Widget::render(tabs, tabs_area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn management_tab_renders_without_panic() {
        let model = Model::new(PathBuf::from("/tmp/test"));
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 1));
        render(&model, &mut buf, Rect::new(0, 0, 80, 1));
        // Should not panic; management tab bar rendered
    }
}
