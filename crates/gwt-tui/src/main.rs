//! gwt-tui entry point.
//!
//! Initializes the terminal, creates the Model, and runs the event loop.

use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

#[cfg(test)]
use crossterm::Command;
use ratatui::{backend::CrosstermBackend, Terminal};

use gwt_agent::reset_runtime_state_dir;
#[cfg(test)]
use gwt_agent::reset_runtime_state_dir_for_pid;
use gwt_core::logging::{
    init as init_logging, LogEvent as Notification, LogLevel as Severity, LoggingConfig,
};
use gwt_core::paths::gwt_logs_dir;
use gwt_git::RepoType;
use gwt_tui::{
    app, event,
    input::keybind::KeybindRegistry,
    message::Message,
    model::{ActiveLayer, Model},
};

const PTY_OUTPUT_POLL_SLICE: Duration = Duration::from_millis(10);

fn drain_pty_output_into_model(model: &mut Model) -> bool {
    let mut drained = false;
    for (session_id, data) in model.drain_pty_output() {
        app::update(model, Message::PtyOutput(session_id, data));
        drained = true;
    }
    drained
}

fn enter_terminal(writer: &mut impl io::Write) -> io::Result<()> {
    execute!(
        writer,
        EnterAlternateScreen,
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
    // SPEC-6 Phase 5: initialize `tracing_subscriber` with a non-blocking
    // JSONL file writer rolling daily, plus a UI forwarder layer. The
    // returned handles MUST be kept alive for the lifetime of main so
    // that the background writer thread stays up.
    let logging_config = LoggingConfig::new(gwt_logs_dir());
    let mut logging_handles = match init_logging(logging_config) {
        Ok(h) => Some(h),
        Err(err) => {
            eprintln!("warning: structured logging disabled: {err}");
            None
        }
    };

    // Install a panic hook that:
    //   1. restores the terminal (raw mode + alt screen) so panics are visible
    //   2. emits the panic info as a `tracing::error!` so that the final
    //      event lands in `gwt.log.YYYY-MM-DD`
    //   3. delegates to the previous hook (which prints the standard
    //      backtrace to stderr)
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = leave_terminal(&mut io::stdout());
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(
            target: "gwt_tui::panic",
            panic = %info,
            backtrace = %backtrace,
            "TUI panic"
        );
        default_hook(info);
    }));

    tracing::info!(
        target: "gwt_tui::main",
        version = env!("CARGO_PKG_VERSION"),
        "gwt-tui starting"
    );

    // Take the UI log receiver before we move into run_app — we will
    // bridge it into a std::sync::mpsc channel that the synchronous
    // event loop can drain.
    let logging_ui_rx = logging_handles.as_mut().and_then(|h| h.take_ui_rx());
    // Clone the reload handle so the Logs tab can cycle the global
    // log level live (SPEC-6 FR-011).
    let logging_reload_handle = logging_handles.as_ref().map(|h| h.reload_handle.clone());

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
    let result = run_app(
        &mut terminal,
        repo_path,
        logging_ui_rx,
        logging_reload_handle,
    );

    // Restore terminal
    disable_raw_mode()?;
    leave_terminal(terminal.backend_mut())?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        tracing::error!(target: "gwt_tui::main", error = %e, "gwt-tui exited with error");
        eprintln!("Error: {e}");
    }

    tracing::info!(target: "gwt_tui::main", "gwt-tui shutdown");

    // Dropping the logging handles flushes the non-blocking writer.
    drop(logging_handles.take());

    Ok(())
}

#[cfg(not(tarpaulin_include))]
fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    repo_path: PathBuf,
    mut logging_ui_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Notification>>,
    logging_reload_handle: Option<gwt_core::logging::ReloadHandle>,
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
    // SPEC-6 Phase 5: spawn the Logs-tab file watcher so the
    // `~/.gwt/logs/gwt.log.YYYY-MM-DD` JSONL stream flows into
    // `LogsState.entries`. Keeping the handle alive for the lifetime
    // of run_app is enough — the watcher owns its own thread.
    let (logs_tx, logs_rx) = std::sync::mpsc::channel();
    let _logs_watcher_handle =
        gwt_tui::logs_watcher::spawn(gwt_core::paths::gwt_logs_dir(), logs_tx);
    model.set_logs_watcher_rx(logs_rx);

    // Plumb the reload handle through so the Logs tab can cycle the
    // tracing level live (SPEC-6 FR-011).
    if let Some(handle) = logging_reload_handle {
        model.set_log_reload_handle(handle);
    }

    // SPEC-6 FR-015: bridge the tokio UnboundedReceiver<LogEvent>
    // from `logging::init` into a std::sync::mpsc channel so the
    // synchronous TUI loop can drain UI log events (warn/error from
    // any crate's `tracing::*!` call) without a tokio runtime.
    if let Some(mut ui_rx) = logging_ui_rx.take() {
        let (bridge_tx, bridge_rx) = std::sync::mpsc::channel::<Notification>();
        std::thread::Builder::new()
            .name("gwt-ui-log-bridge".into())
            .spawn(move || {
                while let Some(event) = ui_rx.blocking_recv() {
                    if bridge_tx.send(event).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn ui log bridge thread");
        model.set_ui_log_rx(bridge_rx);
    }

    if let Some(warning) = reset_startup_runtime_state_with(&gwt_core::paths::gwt_sessions_dir()) {
        app::update(
            &mut model,
            Message::Notify(
                Notification::new(Severity::Warn, "session", "Runtime reset failed")
                    .with_detail(warning),
            ),
        );
    }
    if model.active_layer != ActiveLayer::Initialization {
        if let Some(notification) = project_index_runtime_bootstrap_notification_with(|| {
            gwt_core::runtime::ensure_project_index_runtime().map(|_| ())
        }) {
            app::update(&mut model, Message::Notify(notification));
        }
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

    // Phase 8: bootstrap the index worker (reconcile + Issue refresh + watchers).
    if model.active_layer != ActiveLayer::Initialization {
        let repo_root = model.repo_path().to_path_buf();
        let active_worktrees = model.active_worktree_paths();
        gwt_tui::index_worker::bootstrap(&repo_root, &active_worktrees);
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
            if drain_pty_output_into_model(&mut model) {
                break;
            }

            let Some(msg) = event::poll_event_slice(deadline, PTY_OUTPUT_POLL_SLICE) else {
                continue;
            };

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

fn reset_startup_runtime_state_with(sessions_dir: &Path) -> Option<String> {
    reset_runtime_state_dir(sessions_dir)
        .err()
        .map(|err| err.to_string())
}

#[cfg(test)]
fn reset_startup_runtime_state_for_pid_with(sessions_dir: &Path, pid: u32) -> Option<String> {
    reset_runtime_state_dir_for_pid(sessions_dir, pid)
        .err()
        .map(|err| err.to_string())
}

fn persist_session_state_for_shutdown_with(model: &Model, path: &Path) -> Result<(), String> {
    model.save_session_state(path)
}

fn sync_startup_terminal_size(model: &mut Model, width: u16, height: u16) {
    app::update(model, Message::Resize(width, height));
}

fn project_index_runtime_bootstrap_notification_with<F, E>(
    ensure_runtime: F,
) -> Option<Notification>
where
    F: FnOnce() -> std::result::Result<(), E>,
    E: ToString,
{
    ensure_runtime().err().map(|err| {
        let detail = err.to_string();
        Notification::new(Severity::Warn, "index", "Project index runtime unavailable").with_detail(
            gwt_core::runtime::project_index_runtime_error_detail(&detail),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_tui::app;
    use gwt_tui::model::{ManagementTab, SessionLayout};

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
    fn reset_startup_runtime_state_for_pid_with_clears_only_target_namespace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current_pid = 6060_u32;
        let other_pid = 7070_u32;
        let current_dir = dir.path().join("runtime").join(current_pid.to_string());
        let other_dir = dir.path().join("runtime").join(other_pid.to_string());
        std::fs::create_dir_all(&current_dir).expect("create current pid dir");
        std::fs::create_dir_all(&other_dir).expect("create other pid dir");
        std::fs::write(current_dir.join("session-a.json"), "{}").expect("write current pid file");
        std::fs::write(other_dir.join("session-b.json"), "{}").expect("write other pid file");

        let warning = reset_startup_runtime_state_for_pid_with(dir.path(), current_pid);

        assert!(warning.is_none());
        assert!(current_dir.is_dir());
        assert_eq!(
            std::fs::read_dir(&current_dir)
                .expect("read current pid dir")
                .count(),
            0
        );
        assert!(other_dir.join("session-b.json").exists());
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
    fn project_index_runtime_bootstrap_notification_with_returns_warning_on_failure() {
        let notification = project_index_runtime_bootstrap_notification_with(|| {
            Err::<(), _>("[gwt-project-index-runtime] pip install -r failed".to_string())
        })
        .expect("warning notification");

        assert_eq!(notification.severity, Severity::Warn);
        assert_eq!(notification.source, "index");
        assert_eq!(notification.message, "Project index runtime unavailable");
        assert_eq!(
            notification.detail.as_deref(),
            Some("pip install -r failed")
        );
    }

    #[test]
    fn project_index_runtime_bootstrap_notification_with_returns_none_on_success() {
        let notification =
            project_index_runtime_bootstrap_notification_with(|| Ok::<(), &'static str>(()));
        assert!(notification.is_none());
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
    fn terminal_leave_commands_disable_bracketed_paste() {
        let ansi = terminal_leave_commands_ansi();
        assert!(ansi.contains("\u{1b}[?2004l"));
    }
}
