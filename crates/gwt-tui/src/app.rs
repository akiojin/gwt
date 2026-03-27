use std::{io, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

/// Run the main event loop.
pub fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut should_quit = false;

    while !should_quit {
        terminal.draw(render)?;

        // Poll for events with 50ms timeout.
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                // Ctrl+C or 'q' to quit (temporary, will be replaced by Ctrl+G system).
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    should_quit = true;
                }
                if key.code == KeyCode::Char('q') {
                    should_quit = true;
                }
            }
        }
    }

    Ok(())
}

fn render(frame: &mut Frame) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(1), // tab bar
        Constraint::Min(1),   // terminal area
        Constraint::Length(1), // status bar
    ])
    .split(area);

    // Tab bar
    let tab_bar = Line::from(vec![
        Span::styled(
            " gwt ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" [1] shell ", Style::default().fg(Color::White).bg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(tab_bar), chunks[0]);

    // Terminal area (placeholder)
    let terminal_block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default().bg(Color::Black));
    let terminal_content = Paragraph::new("gwt TUI — Phase 0 scaffold\n\nPress 'q' or Ctrl+C to quit.")
        .style(Style::default().fg(Color::White))
        .block(terminal_block);
    frame.render_widget(terminal_content, chunks[1]);

    // Status bar
    let status = Line::from(vec![
        Span::styled(
            " Ctrl+G ",
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ),
        Span::raw(" for commands  "),
        Span::styled("shell", Style::default().fg(Color::Green)),
        Span::raw("  "),
    ]);
    frame.render_widget(Paragraph::new(status), chunks[2]);
}
