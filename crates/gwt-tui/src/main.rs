//! gwt-tui entry point.
//!
//! Initializes the terminal, creates the Model, and runs the event loop.

use std::{io, path::PathBuf};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use gwt_git::RepoType;
use gwt_tui::{
    app, event,
    input::keybind::KeybindRegistry,
    message::Message,
    model::{ActiveLayer, Model},
};

#[cfg(not(tarpaulin_include))]
fn main() -> io::Result<()> {
    // Parse CLI args: optional repo path
    let repo_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Initialize terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the app
    let result = run_app(&mut terminal, repo_path);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {e}");
    }

    Ok(())
}

#[cfg(not(tarpaulin_include))]
fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    repo_path: PathBuf,
) -> io::Result<()> {
    // Detect repo type and create appropriate model
    let mut model = match gwt_git::detect_repo_type(&repo_path) {
        RepoType::Normal(root) => Model::new(root),
        RepoType::Bare { develop_worktree: Some(wt) } => Model::new(wt),
        RepoType::Bare { develop_worktree: None } => Model::new_initialization(repo_path, true),
        RepoType::NonRepo => Model::new_initialization(repo_path, false),
    };
    let mut keybinds = KeybindRegistry::new();

    loop {
        // View: render
        terminal.draw(|frame| {
            app::view(&model, frame);
        })?;

        // Check quit
        if model.quit {
            break;
        }

        // Event: poll
        let deadline = event::next_tick_deadline();
        if let Some(msg) = event::poll_event(deadline) {
            // Route key events through keybind registry
            // (skip keybind processing when in Initialization layer)
            let msg = match msg {
                Message::KeyInput(key) if model.active_layer != ActiveLayer::Initialization => {
                    keybinds.process_key(key).unwrap_or(Message::KeyInput(key))
                }
                other => other,
            };

            // Update: process message
            app::update(&mut model, msg);
        }
    }

    Ok(())
}
