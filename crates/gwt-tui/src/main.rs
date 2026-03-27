//! gwt-tui: TUI frontend for Git Worktree Manager

mod app;
mod event;
mod state;

use std::io;

use crossterm::{
    event::{self as ct_event, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::CrosstermBackend, Terminal};

use crate::{
    app::App,
    event::{pty_output_channel, TuiEvent},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Determine repo root from current working directory
    let repo_root = std::env::current_dir().unwrap_or_default();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    let (rows, cols) = (size.height, size.width);

    // Create PTY output channel
    let (pty_tx, mut pty_rx) = pty_output_channel();

    // Create app
    let mut app = App::new(repo_root, pty_tx, rows, cols);

    // Main event loop
    let result = run_event_loop(&mut app, &mut terminal, &mut pty_rx);

    // Cleanup terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_event_loop(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    pty_rx: &mut event::PtyOutputReceiver,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        // Render
        terminal.draw(|frame| {
            render(app, frame);
        })?;

        // Collect event
        let event = poll_event(pty_rx)?;

        match event {
            TuiEvent::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                let action = app.map_key(key);
                app.handle_action(action);
            }
            TuiEvent::Resize(cols, rows) => {
                app.handle_resize(cols, rows);
            }
            TuiEvent::PtyOutput { pane_id, data } => {
                app.handle_pty_output(&pane_id, &data);
            }
            TuiEvent::Tick => {}
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Poll for the next event: crossterm input or PTY output.
fn poll_event(
    pty_rx: &mut event::PtyOutputReceiver,
) -> Result<TuiEvent, Box<dyn std::error::Error>> {
    // Check for PTY output first (non-blocking)
    if let Ok((pane_id, data)) = pty_rx.try_recv() {
        return Ok(TuiEvent::PtyOutput { pane_id, data });
    }

    // Poll crossterm events with a short timeout
    if ct_event::poll(std::time::Duration::from_millis(16))? {
        match ct_event::read()? {
            Event::Key(key) => return Ok(TuiEvent::Key(key)),
            Event::Resize(cols, rows) => return Ok(TuiEvent::Resize(cols, rows)),
            _ => {}
        }
    }

    Ok(TuiEvent::Tick)
}

/// Render the TUI frame.
fn render(app: &App, frame: &mut ratatui::Frame) {
    use ratatui::{
        layout::{Constraint, Direction, Layout},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Paragraph, Tabs},
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(frame.area());

    // Tab bar
    let tab_titles: Vec<Line> = app
        .state
        .tabs
        .iter()
        .map(|t| Line::from(Span::raw(&t.name)))
        .collect();

    if !tab_titles.is_empty() {
        let tabs = Tabs::new(tab_titles)
            .select(app.state.active_tab)
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_widget(tabs, chunks[0]);
    } else {
        let hint = Paragraph::new("Press Ctrl-T to open a shell tab")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, chunks[0]);
    }

    // Terminal content area
    let rows = app.active_screen_rows();
    let lines: Vec<Line> = rows.iter().map(|r| Line::from(r.as_str())).collect();

    let mode_indicator = match app.state.mode {
        state::AppMode::ScrollMode => " [SCROLL] ",
        state::AppMode::Management => " [MGMT] ",
        state::AppMode::Normal => "",
    };

    let block = Block::default()
        .borders(Borders::NONE)
        .title(mode_indicator);

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, chunks[1]);
}
