mod app;
mod event;
mod input;
mod renderer;
mod state;
mod ui;

use std::io;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run app
    let result = run_app(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = result {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let block = ratatui::widgets::Block::default()
                .title(" gwt-tui ")
                .borders(ratatui::widgets::Borders::ALL);
            frame.render_widget(block, area);
        })?;

        if crossterm::event::poll(std::time::Duration::from_millis(100))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                if key.code == crossterm::event::KeyCode::Char('q')
                    && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                {
                    return Ok(());
                }
            }
        }
    }
}
