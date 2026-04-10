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
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    Command,
};

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
    input_trace,
    message::Message,
    model::{ActiveLayer, Model},
};

const PTY_OUTPUT_POLL_SLICE: Duration = Duration::from_millis(10);
const PTY_REDRAW_FRAME_INTERVAL: Duration = Duration::from_millis(33);
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
    for (session_id, data) in coalesce_pty_output_chunks(model.drain_pty_output()) {
        app::update(model, Message::PtyOutput(session_id, data));
        drained = true;
    }
    drained
}

fn drain_pty_output_and_request_render(model: &mut Model, needs_render: &mut bool) -> bool {
    let drained = drain_pty_output_into_model(model);
    if drained {
        *needs_render = true;
    }
    drained
}

fn coalesce_pty_output_chunks(chunks: Vec<(String, Vec<u8>)>) -> Vec<(String, Vec<u8>)> {
    let mut merged: Vec<(String, Vec<u8>)> = Vec::new();
    for (session_id, data) in chunks {
        // Merge by session within the current drain pass so snapshot capture
        // follows the drawn frame boundary rather than PTY reader chunking.
        if let Some((_, existing)) = merged.iter_mut().find(|(id, _)| *id == session_id) {
            existing.extend_from_slice(&data);
        } else {
            merged.push((session_id, data));
        }
    }
    merged
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

fn dispatch_post_normalized_message(
    model: &mut Model,
    keybinds: &mut KeybindRegistry,
    msg: Message,
    needs_render: &mut bool,
) {
    let msg = match msg {
        Message::KeyInput(key) if model.active_layer != ActiveLayer::Initialization => {
            let terminal_focused = model.active_focus == gwt_tui::model::FocusPane::Terminal;
            let routed = keybinds.process_key_with_focus(key, terminal_focused);
            input_trace::trace_keybind_decision(key, terminal_focused, routed.as_ref());
            routed.unwrap_or(Message::KeyInput(key))
        }
        other => other,
    };

    let was_tick = matches!(msg, Message::Tick);
    app::update(model, msg);
    if was_tick {
        *needs_render |= should_render_after_tick(model);
    } else {
        *needs_render = true;
    }
}

fn handle_post_normalized_message<F>(
    model: &mut Model,
    keybinds: &mut KeybindRegistry,
    first: Message,
    needs_render: &mut bool,
    pending_messages: &mut VecDeque<Message>,
    next_message: F,
) where
    F: FnMut() -> Option<Message>,
{
    let burst = drain_mouse_scroll_burst(first, pending_messages, next_message);
    for msg in burst {
        dispatch_post_normalized_message(model, keybinds, msg, needs_render);
        if model.quit {
            break;
        }
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

fn next_message_for_loop_iteration<F>(
    pending_messages: &mut VecDeque<Message>,
    deadline: Instant,
    had_pty_output: bool,
    last_draw_at: Option<Instant>,
    mut poll_event: F,
) -> Option<Message>
where
    F: FnMut(Instant, Duration) -> Option<Message>,
{
    if let Some(msg) = pending_messages.pop_front() {
        return Some(msg);
    }

    let poll_slice = if had_pty_output {
        last_draw_at.map_or(Duration::ZERO, |last_draw_at| {
            pty_redraw_poll_slice(Instant::now(), last_draw_at)
        })
    } else {
        PTY_OUTPUT_POLL_SLICE
    };
    poll_event(deadline, poll_slice)
}

fn pty_redraw_poll_slice(now: Instant, last_draw_at: Instant) -> Duration {
    PTY_REDRAW_FRAME_INTERVAL.saturating_sub(now.saturating_duration_since(last_draw_at))
}

fn should_render_after_tick(model: &Model) -> bool {
    app::tick_redraw_required(model)
}

fn enter_terminal(writer: &mut impl io::Write) -> io::Result<()> {
    execute!(
        writer,
        EnterAlternateScreen,
        DisableAlternateScrollMode,
        EnableMouseCapture,
        EnableBracketedPaste,
    )?;
    enable_keyboard_enhancements(writer);
    Ok(())
}

fn leave_terminal(writer: &mut impl io::Write) -> io::Result<()> {
    disable_keyboard_enhancements(writer);
    execute!(
        writer,
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste,
    )
}

fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
}

fn enable_keyboard_enhancements(writer: &mut impl io::Write) {
    // Fail-open: keep startup working even when the host terminal ignores or rejects kitty flags.
    let _ = execute!(
        writer,
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
    );
}

fn disable_keyboard_enhancements(writer: &mut impl io::Write) {
    // Fail-open: shutdown should restore the terminal even if keyboard enhancement pop fails.
    let _ = execute!(writer, PopKeyboardEnhancementFlags);
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
    PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
        .write_ansi(&mut ansi)
        .expect("enable keyboard enhancement ansi");
    ansi
}

#[cfg(test)]
fn terminal_leave_commands_ansi() -> String {
    let mut ansi = String::new();
    PopKeyboardEnhancementFlags
        .write_ansi(&mut ansi)
        .expect("disable keyboard enhancement ansi");
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
    // SPEC-12 Phase 6 (CORE-CLI / #1942): argv-driven dispatch. When invoked
    // as `gwt issue ...` or `gwt hook ...`, hand off to the CLI entry point
    // without touching the terminal or initializing the TUI tracing
    // subscriber. Any other argv shape keeps the legacy TUI behaviour below.
    let argv: Vec<String> = std::env::args().collect();
    if gwt_tui::cli::should_dispatch_cli(&argv) {
        return run_cli(&argv);
    }

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

/// SPEC-12 Phase 6 (CORE-CLI / #1942): CLI entry point.
///
/// This is reached when argv[1] is a known CLI verb (`issue`, `pr`,
/// `actions`, or `hook`).
/// We resolve the repository coordinates from the current git remote,
/// build the production [`DefaultCliEnv`], and dispatch the subcommand
/// without initializing the TUI tracing subscriber (CLI invocations are
/// short-lived and must not interfere with concurrent TUI sessions writing
/// to the same log directory).
#[cfg(not(tarpaulin_include))]
fn run_cli(argv: &[String]) -> io::Result<()> {
    // For `gwt hook ...` we can run even outside a GitHub-linked repo,
    // because hooks don't need owner/repo for local atomic writes and
    // stdin judgement. For `gwt issue|pr|actions ...` we need the remote
    // coordinates and repo cwd.
    let needs_repo = matches!(
        argv.get(1).map(String::as_str),
        Some("issue" | "pr" | "actions")
    );

    if needs_repo {
        let repo_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (owner, repo) = match resolve_repo_coordinates() {
            Some(coords) => coords,
            None => {
                eprintln!(
                    "gwt {}: could not resolve GitHub owner/repo from the current git remote",
                    argv.get(1).map(String::as_str).unwrap_or("issue")
                );
                std::process::exit(2);
            }
        };
        let mut env = match gwt_tui::cli::DefaultCliEnv::new(&owner, &repo, repo_path) {
            Ok(env) => env,
            Err(e) => {
                eprintln!(
                    "gwt {}: {e}",
                    argv.get(1).map(String::as_str).unwrap_or("issue")
                );
                std::process::exit(1);
            }
        };
        let code = gwt_tui::cli::dispatch(&mut env, argv);
        std::process::exit(code);
    }

    // `gwt hook ...` path: hooks never touch GitHub, so we deliberately
    // skip the `gh auth token` resolution that `DefaultCliEnv::new`
    // performs. `new_for_hooks` constructs an env with an inert
    // HttpIssueClient (empty token / owner / repo) — attempting to call
    // the client would fail loudly, but the hook code paths route
    // through `run_hook` and never touch it.
    let mut env = match gwt_tui::cli::DefaultCliEnv::new_for_hooks() {
        Ok(env) => env,
        Err(e) => {
            eprintln!("gwt hook: {e}");
            std::process::exit(1);
        }
    };
    let code = gwt_tui::cli::dispatch(&mut env, argv);
    std::process::exit(code);
}

/// Parse the `origin` remote URL and return `(owner, repo)` when the remote
/// points at github.com. Supports both HTTPS and SSH URLs. Returns `None`
/// when the remote cannot be resolved or the host is not github.com.
fn resolve_repo_coordinates() -> Option<(String, String)> {
    use std::process::Command;
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_github_remote_url(&url)
}

fn parse_github_remote_url(url: &str) -> Option<(String, String)> {
    // SSH: git@github.com:owner/repo(.git)?
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let trimmed = rest.trim_end_matches(".git");
        let mut parts = trimmed.splitn(2, '/');
        let owner = parts.next()?.to_string();
        let repo = parts.next()?.to_string();
        return Some((owner, repo));
    }
    // HTTPS: https://github.com/owner/repo(.git)?
    for prefix in [
        "https://github.com/",
        "http://github.com/",
        "git://github.com/",
    ] {
        if let Some(rest) = url.strip_prefix(prefix) {
            let trimmed = rest.trim_end_matches(".git").trim_end_matches('/');
            let mut parts = trimmed.splitn(2, '/');
            let owner = parts.next()?.to_string();
            let repo = parts.next()?.to_string();
            return Some((owner, repo));
        }
    }
    None
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

    // SPEC-6 FR-015 + reviewer comment B5: bridge the tokio
    // UnboundedReceiver<LogEvent> from `logging::init` into a
    // std::sync::mpsc channel so the synchronous TUI loop can drain
    // UI log events without a tokio runtime.
    //
    // The bridge loop polls `try_recv` and watches an
    // `Arc<AtomicBool>` shutdown flag instead of using
    // `blocking_recv()`. The reason: the `UnboundedSender` produced
    // by `logging::init` is cloned into the global tracing
    // subscriber's `UiForwarderLayer` and there is no way to drop the
    // global subscriber, so `blocking_recv()` would never see all
    // senders dropped and the bridge thread would hang on shutdown.
    let logs_bridge_shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Some(mut ui_rx) = logging_ui_rx.take() {
        let (bridge_tx, bridge_rx) = std::sync::mpsc::channel::<Notification>();
        let shutdown = std::sync::Arc::clone(&logs_bridge_shutdown);
        std::thread::Builder::new()
            .name("gwt-ui-log-bridge".into())
            .spawn(move || {
                use std::sync::atomic::Ordering;
                while !shutdown.load(Ordering::Relaxed) {
                    match ui_rx.try_recv() {
                        Ok(event) => {
                            if bridge_tx.send(event).is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                            break;
                        }
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
        // Wire the notification bus first so log_event() entries flow into the
        // Logs tab as well as `~/.gwt/logs/index.log`.
        gwt_tui::index_worker::init_notification_bus(model.notification_bus_handle());
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
    let mut input_normalizer = event::InputNormalizer::default();
    let mut pending_messages = VecDeque::new();
    let mut needs_render = true;
    let mut last_draw_at = None;

    loop {
        drain_pty_output_and_request_render(&mut model, &mut needs_render);

        if needs_render {
            terminal.draw(|frame| {
                app::view(&model, frame);
            })?;
            needs_render = false;
            last_draw_at = Some(Instant::now());
        }

        // Check quit
        if model.quit {
            break;
        }

        // Event: poll
        let deadline = event::next_tick_deadline();
        loop {
            if let Some(msg) = input_normalizer.pop_pending(std::time::Instant::now()) {
                handle_post_normalized_message(
                    &mut model,
                    &mut keybinds,
                    msg,
                    &mut needs_render,
                    &mut pending_messages,
                    || input_normalizer.pop_pending(std::time::Instant::now()),
                );
                break;
            }

            let had_pty_output = drain_pty_output_and_request_render(&mut model, &mut needs_render);

            let Some(msg) = next_message_for_loop_iteration(
                &mut pending_messages,
                deadline,
                had_pty_output,
                last_draw_at,
                event::poll_event_slice,
            ) else {
                if had_pty_output {
                    break;
                }
                continue;
            };

            let terminal_focused = model.active_layer != ActiveLayer::Initialization
                && model.active_focus == gwt_tui::model::FocusPane::Terminal;
            let Some(msg) =
                input_normalizer.normalize(msg, std::time::Instant::now(), terminal_focused)
            else {
                continue;
            };

            handle_post_normalized_message(
                &mut model,
                &mut keybinds,
                msg,
                &mut needs_render,
                &mut pending_messages,
                || {
                    poll_immediate_message_for_scroll_burst(
                        deadline,
                        &mut input_normalizer,
                        terminal_focused,
                    )
                },
            );
            break;
        }
    }

    // Kill all live PTY processes on shutdown.
    model.kill_all_pty();

    // Signal the UI log bridge thread to exit so it does not outlive
    // `run_app` (reviewer comment B5).
    logs_bridge_shutdown.store(true, std::sync::atomic::Ordering::Relaxed);

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
    use crossterm::event::{KeyCode, KeyModifiers, MouseEvent, MouseEventKind};
    use gwt_tui::app;
    use gwt_tui::message::Message;
    use gwt_tui::model::{FocusPane, ManagementTab, SessionLayout};
    use gwt_tui::screens::docker_progress::{DockerProgressMessage, DockerStage};

    fn scroll_message(kind: MouseEventKind) -> Message {
        Message::MouseInput(MouseEvent {
            kind,
            column: 40,
            row: 12,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn coalesce_pty_output_chunks_merges_each_session_in_first_seen_order() {
        let merged = coalesce_pty_output_chunks(vec![
            ("shell-0".to_string(), b"frame".to_vec()),
            ("agent-1".to_string(), b"AA".to_vec()),
            ("shell-0".to_string(), b"-1".to_vec()),
            ("agent-1".to_string(), b"BB".to_vec()),
            ("shell-0".to_string(), b"\n".to_vec()),
        ]);

        assert_eq!(
            merged,
            vec![
                ("shell-0".to_string(), b"frame-1\n".to_vec()),
                ("agent-1".to_string(), b"AABB".to_vec()),
            ]
        );
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
    fn drain_pty_output_and_request_render_marks_dirty_after_output() {
        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        let mut needs_render = false;

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
            if drain_pty_output_and_request_render(&mut model, &mut needs_render) {
                drained = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(drained, "pty output should eventually be drained");
        assert!(
            needs_render,
            "draining PTY output must request a redraw so committed text appears immediately"
        );
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
    fn next_message_for_loop_iteration_prioritizes_pending_queue_during_pty_updates() {
        let mut pending_messages = VecDeque::from([Message::Tick]);
        let mut polled = false;

        let next = next_message_for_loop_iteration(
            &mut pending_messages,
            Instant::now(),
            true,
            Some(Instant::now()),
            |_deadline, _slice| {
                polled = true;
                None
            },
        );

        assert!(matches!(next, Some(Message::Tick)));
        assert!(
            !polled,
            "queued input should be consumed before polling when PTY output is active"
        );
    }

    #[test]
    fn next_message_for_loop_iteration_uses_frame_budget_after_pty_output() {
        let mut pending_messages = VecDeque::new();
        let mut observed_slice = None;

        let next = next_message_for_loop_iteration(
            &mut pending_messages,
            Instant::now(),
            true,
            Some(Instant::now()),
            |_deadline, slice| {
                observed_slice = Some(slice);
                Some(Message::Tick)
            },
        );

        assert!(matches!(next, Some(Message::Tick)));
        let observed_slice = observed_slice.expect("observed slice");
        assert!(observed_slice > Duration::from_millis(1));
        assert!(observed_slice <= PTY_REDRAW_FRAME_INTERVAL);
    }

    #[test]
    fn next_message_for_loop_iteration_uses_standard_slice_without_pty_output() {
        let mut pending_messages = VecDeque::new();
        let mut observed_slice = None;

        let next = next_message_for_loop_iteration(
            &mut pending_messages,
            Instant::now(),
            false,
            None,
            |_deadline, slice| {
                observed_slice = Some(slice);
                Some(Message::Tick)
            },
        );

        assert!(matches!(next, Some(Message::Tick)));
        assert_eq!(observed_slice, Some(PTY_OUTPUT_POLL_SLICE));
    }

    #[test]
    fn pty_redraw_poll_slice_waits_for_remaining_frame_budget() {
        let now = Instant::now();
        let last_draw_at = now - Duration::from_millis(5);

        let slice = pty_redraw_poll_slice(now, last_draw_at);

        assert_eq!(slice, Duration::from_millis(28));
    }

    #[test]
    fn pty_redraw_poll_slice_returns_zero_after_frame_budget_is_spent() {
        let now = Instant::now();
        let last_draw_at = now - PTY_REDRAW_FRAME_INTERVAL - Duration::from_millis(5);

        let slice = pty_redraw_poll_slice(now, last_draw_at);

        assert_eq!(slice, Duration::ZERO);
    }

    #[test]
    fn dispatch_post_normalized_message_routes_pending_keys_through_keybinds() {
        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Settings;
        model.active_focus = FocusPane::TabContent;
        let mut keybinds = KeybindRegistry::new();
        let mut needs_render = false;

        dispatch_post_normalized_message(
            &mut model,
            &mut keybinds,
            Message::KeyInput(crossterm::event::KeyEvent::new(
                KeyCode::Char('g'),
                KeyModifiers::CONTROL,
            )),
            &mut needs_render,
        );
        dispatch_post_normalized_message(
            &mut model,
            &mut keybinds,
            Message::KeyInput(crossterm::event::KeyEvent::new(
                KeyCode::Tab,
                KeyModifiers::NONE,
            )),
            &mut needs_render,
        );

        assert_eq!(
            model.active_focus,
            FocusPane::Terminal,
            "pending normalized keys should still use the keybind dispatch path"
        );
        assert!(needs_render, "key dispatch should request a redraw");
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

    #[test]
    fn terminal_enter_commands_enable_keyboard_enhancement_flags() {
        let ansi = terminal_enter_commands_ansi();
        let mut expected = String::new();
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
            .write_ansi(&mut expected)
            .expect("keyboard enhancement push ansi");
        assert!(ansi.contains(expected.as_str()));
    }

    #[test]
    fn terminal_leave_commands_pop_keyboard_enhancement_flags() {
        let ansi = terminal_leave_commands_ansi();
        let mut expected = String::new();
        PopKeyboardEnhancementFlags
            .write_ansi(&mut expected)
            .expect("keyboard enhancement pop ansi");
        assert!(ansi.contains(expected.as_str()));
    }

    #[test]
    fn should_render_after_tick_skips_idle_terminal_focus() {
        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        app::update(&mut model, Message::Tick);

        assert!(
            !should_render_after_tick(&model),
            "idle terminal ticks should not repaint the TUI"
        );
    }

    #[test]
    fn should_render_after_tick_keeps_non_terminal_focus_redrawing() {
        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;

        app::update(&mut model, Message::Tick);

        assert!(
            should_render_after_tick(&model),
            "management-focused ticks should keep rendering"
        );
    }

    #[test]
    fn should_render_after_tick_keeps_visible_docker_overlay_redrawing() {
        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        app::update(
            &mut model,
            Message::DockerProgress(DockerProgressMessage::SetStage {
                stage: DockerStage::BuildingImage,
                message: "Building image".to_string(),
            }),
        );
        app::update(&mut model, Message::Tick);

        assert!(
            should_render_after_tick(&model),
            "visible overlays that depend on tick-driven updates should still redraw"
        );
    }
}
