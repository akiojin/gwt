//! gwt-tui entry point.
//!
//! Initializes the terminal, creates the Model, and runs the event loop.

use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    Command,
};

use ratatui::{backend::CrosstermBackend, Terminal};

use gwt_git::RepoType;
use gwt_notification::{Notification, Severity};
use gwt_tui::{
    app, event,
    input::keybind::KeybindRegistry,
    message::Message,
    model::{ActiveLayer, Model},
};

const PTY_OUTPUT_POLL_SLICE: Duration = Duration::from_millis(10);
const MAX_MOUSE_SCROLL_BURST_MESSAGES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisableAlternateScrollMode;

impl Command for DisableAlternateScrollMode {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        // Terminal.app can translate trackpad scrolling in the alternate screen
        // into cursor keys unless alternate-scroll mode is explicitly disabled.
        f.write_str("\u{1b}[?1007l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }
}

fn drain_pty_output_into_model(model: &mut Model) -> bool {
    let mut drained = false;
    for (session_id, data) in model.drain_pty_output() {
        app::update(model, Message::PtyOutput(session_id, data));
        drained = true;
    }
    drained
}

fn is_mouse_scroll_message(msg: &Message) -> bool {
    matches!(
        msg,
        Message::MouseInput(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp
                | crossterm::event::MouseEventKind::ScrollDown,
            ..
        })
    )
}

fn poll_immediate_message_for_scroll_burst(
    deadline: Instant,
    input_normalizer: &mut event::InputNormalizer,
    terminal_focused: bool,
) -> Option<Message> {
    loop {
        let now = Instant::now();
        if let Some(msg) = input_normalizer.pop_pending(now) {
            return Some(msg);
        }

        let raw = event::poll_event_slice(deadline, Duration::ZERO)?;
        let now = Instant::now();
        let Some(msg) = input_normalizer.normalize(raw, now, terminal_focused) else {
            continue;
        };
        return Some(msg);
    }
}

fn drain_mouse_scroll_burst<F>(
    first: Message,
    pending_messages: &mut VecDeque<Message>,
    mut next_message: F,
) -> Vec<Message>
where
    F: FnMut() -> Option<Message>,
{
    let mut burst = vec![first];
    if !is_mouse_scroll_message(&burst[0]) {
        return burst;
    }

    while burst.len() < MAX_MOUSE_SCROLL_BURST_MESSAGES {
        let Some(next) = next_message() else {
            break;
        };
        if is_mouse_scroll_message(&next) {
            burst.push(next);
        } else {
            pending_messages.push_back(next);
            break;
        }
    }

    burst
}

fn enter_terminal(writer: &mut impl io::Write) -> io::Result<()> {
    execute!(
        writer,
        EnterAlternateScreen,
        DisableAlternateScrollMode,
        EnableMouseCapture,
        EnableBracketedPaste,
    )
}

fn leave_terminal(writer: &mut impl io::Write) -> io::Result<()> {
    execute!(
        writer,
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste,
    )
}

#[cfg(test)]
fn terminal_enter_commands_ansi() -> String {
    let mut ansi = String::new();
    EnterAlternateScreen
        .write_ansi(&mut ansi)
        .expect("enter alternate screen ansi");
    DisableAlternateScrollMode
        .write_ansi(&mut ansi)
        .expect("disable alternate scroll ansi");
    EnableMouseCapture
        .write_ansi(&mut ansi)
        .expect("enable mouse capture ansi");
    EnableBracketedPaste
        .write_ansi(&mut ansi)
        .expect("enable bracketed paste ansi");
    ansi
}

#[cfg(test)]
fn terminal_leave_commands_ansi() -> String {
    let mut ansi = String::new();
    LeaveAlternateScreen
        .write_ansi(&mut ansi)
        .expect("leave alternate screen ansi");
    DisableMouseCapture
        .write_ansi(&mut ansi)
        .expect("disable mouse capture ansi");
    DisableBracketedPaste
        .write_ansi(&mut ansi)
        .expect("disable bracketed paste ansi");
    ansi
}

#[cfg(not(tarpaulin_include))]
fn main() -> io::Result<()> {
    // Install a panic hook that restores the terminal before printing the
    // backtrace.  Without this, panics leave the terminal in raw/alt-screen
    // mode and the error message is invisible.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = leave_terminal(&mut io::stdout());
        default_hook(info);
    }));

    // Parse CLI args: optional repo path
    let repo_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Initialize terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    enter_terminal(&mut stdout)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the app
    let result = run_app(&mut terminal, repo_path);

    // Restore terminal
    disable_raw_mode()?;
    leave_terminal(terminal.backend_mut())?;
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
        RepoType::Bare {
            develop_worktree: Some(wt),
        } => Model::new(wt),
        RepoType::Bare {
            develop_worktree: None,
        } => Model::new_initialization(repo_path, true),
        RepoType::NonRepo => Model::new_initialization(repo_path, false),
    };
    if model.active_layer != ActiveLayer::Initialization {
        let session_state_path = Model::session_state_path(model.repo_path());
        if let Some(warning) = restore_startup_session_state_with(&mut model, &session_state_path) {
            app::update(
                &mut model,
                Message::Notify(
                    Notification::new(Severity::Warn, "session", "Session restore fell back")
                        .with_detail(warning),
                ),
            );
        }
        let size = terminal.size()?;
        sync_startup_terminal_size(&mut model, size.width, size.height);
    }
    // Load initial data (branches, specs, tags) — best-effort
    if model.active_layer != ActiveLayer::Initialization {
        app::load_initial_data(&mut model);
    }

    // Spawn PTY for the default shell-0 session.
    if model.active_layer != ActiveLayer::Initialization {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let (cols, rows) = app::session_content_size(&model);
        // Resize default VtState to match actual pane area.
        if let Some(s) = model.active_session_tab_mut() {
            s.vt.resize(rows, cols);
        }
        let config = gwt_terminal::pty::SpawnConfig {
            command: shell,
            args: vec![],
            cols,
            rows,
            env: std::collections::HashMap::new(),
            cwd: Some(model.repo_path().to_path_buf()),
        };
        if let Err(e) = app::spawn_pty_for_session(&mut model, "shell-0", config) {
            app::update(
                &mut model,
                Message::Notify(Notification::new(
                    Severity::Error,
                    "pty",
                    format!("Default shell spawn failed: {e}"),
                )),
            );
        }
    }

    let mut keybinds = KeybindRegistry::new();
    let mut input_normalizer = event::InputNormalizer::default();
    let mut pending_messages = VecDeque::new();

    loop {
        drain_pty_output_into_model(&mut model);

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
        loop {
            if let Some(msg) = input_normalizer.pop_pending(std::time::Instant::now()) {
                app::update(&mut model, msg);
                break;
            }

            if drain_pty_output_into_model(&mut model) {
                break;
            }

            let Some(msg) = pending_messages
                .pop_front()
                .or_else(|| event::poll_event_slice(deadline, PTY_OUTPUT_POLL_SLICE))
            else {
                continue;
            };

            let terminal_focused = model.active_layer != ActiveLayer::Initialization
                && model.active_focus == gwt_tui::model::FocusPane::Terminal;
            let Some(msg) =
                input_normalizer.normalize(msg, std::time::Instant::now(), terminal_focused)
            else {
                continue;
            };

            let burst = drain_mouse_scroll_burst(msg, &mut pending_messages, || {
                poll_immediate_message_for_scroll_burst(
                    deadline,
                    &mut input_normalizer,
                    terminal_focused,
                )
            });

            for msg in burst {
                // Route key events through keybind registry
                // (skip keybind processing when in Initialization layer)
                let msg = match msg {
                    Message::KeyInput(key) if model.active_layer != ActiveLayer::Initialization => {
                        let terminal_focused =
                            model.active_focus == gwt_tui::model::FocusPane::Terminal;
                        keybinds
                            .process_key_with_focus(key, terminal_focused)
                            .unwrap_or(Message::KeyInput(key))
                    }
                    other => other,
                };

                // Update: process message
                app::update(&mut model, msg);
                if model.quit {
                    break;
                }
            }
            break;
        }
    }

    // Kill all live PTY processes on shutdown.
    model.kill_all_pty();

    if model.active_layer != ActiveLayer::Initialization {
        let session_state_path = Model::session_state_path(model.repo_path());
        if let Err(err) = persist_session_state_for_shutdown_with(&model, &session_state_path) {
            eprintln!("Warning: failed to save session state: {err}");
        }
    }

    Ok(())
}

fn restore_startup_session_state_with(model: &mut Model, path: &Path) -> Option<String> {
    model.restore_session_state_from_path(path)
}

fn persist_session_state_for_shutdown_with(model: &Model, path: &Path) -> Result<(), String> {
    model.save_session_state(path)
}

fn sync_startup_terminal_size(model: &mut Model, width: u16, height: u16) {
    app::update(model, Message::Resize(width, height));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers, MouseEvent, MouseEventKind};
    use gwt_tui::app;
    use gwt_tui::model::{ManagementTab, SessionLayout};

    fn scroll_message(kind: MouseEventKind) -> Message {
        Message::MouseInput(MouseEvent {
            kind,
            column: 40,
            row: 12,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn restore_startup_session_state_with_applies_saved_layout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.toml");

        let mut original = Model::new(PathBuf::from("/tmp/repo"));
        original.session_layout = SessionLayout::Grid;
        original.active_layer = ActiveLayer::Main;
        original.management_tab = ManagementTab::Logs;
        original
            .save_session_state(&path)
            .expect("save original session state");

        let mut restored = Model::new(PathBuf::from("/tmp/repo"));
        let warning = restore_startup_session_state_with(&mut restored, &path);

        assert!(warning.is_none());
        assert_eq!(restored.session_layout, SessionLayout::Grid);
        assert_eq!(restored.active_layer, ActiveLayer::Main);
        assert_eq!(restored.management_tab, ManagementTab::Logs);
    }

    #[test]
    fn restore_startup_session_state_with_returns_warning_for_corrupted_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "display_mode = [").expect("write corrupted state");

        let mut restored = Model::new(PathBuf::from("/tmp/repo"));
        let warning = restore_startup_session_state_with(&mut restored, &path);

        assert!(warning.is_some());
    }

    #[test]
    fn persist_session_state_for_shutdown_with_creates_state_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("session.toml");
        let model = Model::new(PathBuf::from("/tmp/repo"));

        persist_session_state_for_shutdown_with(&model, &path)
            .expect("persist session state for shutdown");

        assert!(path.exists());
    }

    #[test]
    fn sync_startup_terminal_size_updates_model_and_active_vt_before_spawn() {
        let mut model = Model::new(PathBuf::from("/tmp/repo"));

        sync_startup_terminal_size(&mut model, 120, 40);

        assert_eq!(model.terminal_size(), (120, 40));
        let (cols, rows) = app::session_content_size(&model);
        let active = model.active_session_tab().expect("active session");
        assert_eq!(active.vt.cols(), cols);
        assert_eq!(active.vt.rows(), rows);
    }

    #[test]
    fn drain_pty_output_into_model_applies_output_without_tick() {
        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        assert!(!model
            .active_session_tab()
            .expect("active session")
            .vt
            .screen()
            .contents()
            .contains("ready"));

        app::spawn_pty_for_session(
            &mut model,
            "shell-0",
            gwt_terminal::pty::SpawnConfig {
                command: "/bin/echo".to_string(),
                args: vec!["ready".to_string()],
                cols: 80,
                rows: 24,
                env: std::collections::HashMap::new(),
                cwd: None,
            },
        )
        .expect("spawn echo pty");

        let mut drained = false;
        for _ in 0..20 {
            if drain_pty_output_into_model(&mut model) {
                drained = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            drained,
            "queued pty output should drain without requiring Tick"
        );
        assert!(model
            .active_session_tab()
            .expect("active session")
            .vt
            .screen()
            .contents()
            .contains("ready"));
    }

    #[test]
    fn terminal_enter_commands_enable_bracketed_paste() {
        let ansi = terminal_enter_commands_ansi();
        assert!(ansi.contains("\u{1b}[?2004h"));
    }

    #[test]
    fn drain_mouse_scroll_burst_collects_consecutive_scroll_messages() {
        let first = scroll_message(MouseEventKind::ScrollUp);
        let mut pending_messages = VecDeque::new();
        let mut queued = VecDeque::from([
            scroll_message(MouseEventKind::ScrollUp),
            scroll_message(MouseEventKind::ScrollDown),
            Message::KeyInput(crossterm::event::KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )),
        ]);

        let burst = drain_mouse_scroll_burst(first, &mut pending_messages, || queued.pop_front());

        assert_eq!(burst.len(), 3);
        assert!(burst.iter().all(is_mouse_scroll_message));
        assert!(matches!(
            pending_messages.pop_front(),
            Some(Message::KeyInput(key)) if key.code == KeyCode::Esc
        ));
        assert!(queued.is_empty());
    }

    #[test]
    fn drain_mouse_scroll_burst_preserves_non_scroll_first_message() {
        let first = Message::Tick;
        let mut pending_messages = VecDeque::new();
        let mut queued = VecDeque::from([scroll_message(MouseEventKind::ScrollUp)]);

        let burst = drain_mouse_scroll_burst(first, &mut pending_messages, || queued.pop_front());

        assert!(matches!(burst.as_slice(), [Message::Tick]));
        assert!(pending_messages.is_empty());
        assert_eq!(queued.len(), 1);
        assert!(matches!(
            queued.pop_front(),
            Some(Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..
            }))
        ));
    }

    #[test]
    fn terminal_enter_commands_disable_alternate_scroll_mode() {
        let ansi = terminal_enter_commands_ansi();
        assert!(
            ansi.contains("\u{1b}[?1007l"),
            "terminal startup should disable alternate-scroll mode so Terminal.app delivers wheel events to gwt"
        );
    }

    #[test]
    fn terminal_leave_commands_disable_bracketed_paste() {
        let ansi = terminal_leave_commands_ansi();
        assert!(ansi.contains("\u{1b}[?2004l"));
    }
}
