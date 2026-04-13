//! Help overlay for displaying registered keybindings.

use crate::input::keybind::{Keybinding, KeybindingCategory};
use crate::theme;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

/// Render the help overlay with grouped keybindings.
pub fn render(bindings: &[Keybinding], frame: &mut Frame, area: Rect) {
    let width = (area.width * 70 / 100).max(52);
    let height = (area.height * 75 / 100).max(12);
    let overlay = super::centered_rect(width, height, area);
    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Help")
        .border_type(theme::border::modal())
        .border_style(Style::default().fg(theme::color::FOCUS));
    let inner = block.inner(overlay);
    frame.render_widget(block, overlay);

    let category_order = [
        KeybindingCategory::Global,
        KeybindingCategory::Sessions,
        KeybindingCategory::Management,
        KeybindingCategory::Input,
    ];

    let mut lines = vec![Line::from(Span::styled(
        "Press Esc or Ctrl+G,? to close",
        theme::style::muted_text(),
    ))];

    for category in category_order {
        let category_bindings: Vec<&Keybinding> = bindings
            .iter()
            .filter(|binding| binding.category == category)
            .collect();
        if category_bindings.is_empty() {
            continue;
        }

        lines.push(Line::from(""));
        lines.push(theme::section_divider(category.label(), inner.width));

        for binding in category_bindings {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<12}", binding.keys),
                    Style::default().fg(theme::color::FOCUS),
                ),
                Span::raw(" "),
                Span::styled(
                    binding.description.clone(),
                    Style::default().fg(theme::color::TEXT_PRIMARY),
                ),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default())
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::keybind::KeybindRegistry;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn render_lists_registered_keybindings() {
        let backend = TestBackend::new(160, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let registry = KeybindRegistry::new();
        terminal
            .draw(|frame| render(registry.all_bindings(), frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Help"));
        assert!(text.contains("Ctrl+G, g"));
        assert!(text.contains("Ctrl+G, ?"));
        assert!(text.contains("Copy selected terminal text"));
        assert!(text.contains("Global"));
        assert!(text.contains("Sessions"));
    }

    #[test]
    fn render_omits_unregistered_sequences() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let registry = KeybindRegistry::new();
        terminal
            .draw(|frame| render(registry.all_bindings(), frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(!text.contains("Ctrl+Shift+P"));
        assert!(!text.contains("Alt+Enter"));
    }
}
