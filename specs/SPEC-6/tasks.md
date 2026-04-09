# SPEC-6: Tasks

## Phase 1: Notification Bus and Status Bar

### 1.1 Core Types [P]

- [x] TEST: Unit test for `Severity` enum ordering (Debug < Info < Warn < Error)
- [x] TEST: Unit test for `Notification` struct construction with timestamp, severity, source, message
- [x] IMPL: Add `Severity` enum in gwt-notification
  - File: `crates/gwt-notification/src/severity.rs`
- [x] IMPL: Add `Notification` struct in gwt-notification
  - File: `crates/gwt-notification/src/notification.rs`

### 1.2 Notification Bus [P]

- [x] TEST: Unit test for `NotificationBus` send/receive (async)
- [x] TEST: Unit test for bus non-blocking behavior (sender does not block on slow consumer)
- [x] IMPL: Add `NotificationBus` with tokio mpsc channel
  - File: `crates/gwt-notification/src/bus.rs`
- [x] IMPL: Bus capacity configuration (default: 256 pending notifications)

### 1.3 Status Bar Notification Area

- [x] TEST: Widget render test for status bar with Info notification displayed
- [x] TEST: Widget render test for status bar with Warn notification (distinct color)
- [x] TEST: Unit test for auto-dismiss timer (Info dismissed after 5s)
- [x] IMPL: Add notification area to status bar widget (right side)
  - File: `crates/gwt-tui/src/widgets/status_bar.rs`
- [x] IMPL: Auto-dismiss timer for Info notifications
- [x] IMPL: Warn notification rendering with amber/yellow color
- [x] IMPL: Dismiss keybinding for Warn notifications

## Phase 2: Severity Routing

### 2.1 Router [P]

- [x] TEST: Unit test for routing: Debug -> log only
- [x] TEST: Unit test for routing: Info -> status bar
- [x] TEST: Unit test for routing: Warn -> status bar with color
- [x] TEST: Unit test for routing: Error -> modal dialog
- [x] IMPL: Add `NotificationRouter` that dispatches by severity
  - File: `crates/gwt-tui/src/notification_router.rs`

### 2.2 Error Modal Upgrade

- [x] TEST: Unit test for error modal accepting `Notification` objects
- [x] TEST: Unit test for error queue stacking (dismiss one, next appears)
- [x] IMPL: Refactor existing error queue overlay to use `Notification` struct
  - File: `crates/gwt-tui/src/widgets/error_modal.rs`
- [x] IMPL: Modal displays message, details, and dismiss instruction
- [x] IMPL: Queue management: dismiss with Enter/Esc, show next in queue

## Phase 3: Structured Log Integration

### 3.1 Structured Log Store [P]

- [x] TEST: Unit test for ring buffer: inserts, capacity limit, oldest evicted
- [x] TEST: Unit test for log entry format (timestamp, severity, source, message)
- [x] IMPL: Add `StructuredLog` ring buffer store
  - File: `crates/gwt-notification/src/log.rs`
- [x] IMPL: Configurable capacity (default: 10,000 entries)

### 3.2 Logs Tab Extension

- [x] TEST: Snapshot test for Logs tab with severity filter active
- [x] TEST: Unit test for severity filtering (show only Warn+Error, show all, etc.)
- [x] IMPL: Extend Logs tab to display structured log entries
  - File: `crates/gwt-tui/src/screens/logs.rs`
- [x] IMPL: Severity filter toggle (keybinding to cycle filters)
- [x] IMPL: Debug filter toggle (show/hide Debug entries)
- [x] IMPL: Scrollable log list with timestamp and severity columns

### 3.3 Bus-to-Log Integration

- [x] TEST: Integration test: notification sent via bus -> appears in structured log
- [x] TEST: Integration test: notification sent via bus -> routed to correct surface
- [x] IMPL: Connect NotificationBus consumer to StructuredLog store
- [x] IMPL: Connect NotificationBus consumer to NotificationRouter

## Phase 4: Integration Testing

- [x] TEST: End-to-end test: Info notification appears in status bar and auto-dismisses
- [x] TEST: End-to-end test: Error notification appears in modal, dismiss shows next
- [x] TEST: End-to-end test: all severity levels logged in structured log
- [x] TEST: Regression test: existing error queue behavior preserved
- [x] TEST: Performance test: 100 simultaneous errors do not freeze UI

## Phase 5: tracing-based Structured Logging Migration

> **TDD:** every IMPL task in this phase must be preceded by the matching RED test. All tests in this phase were RED at 2026-04-08. Do not mark them done until the test is committed in failing state, then passes after the IMPL step.

### 5.1 Foundation — `gwt_core::logging::init`

- [ ] TEST: Unit test: `LoggingConfig::from_env_and_file` — returns default `info` when neither `RUST_LOG` nor `config.toml` set; `RUST_LOG` wins over `config.toml`; `config.toml` wins over default.
  - File: `crates/gwt-core/src/logging/config.rs` (`#[cfg(test)]`)
- [ ] TEST: Unit test: `housekeep` deletes `gwt.log.YYYY-MM-DD` older than 7 local days relative to a frozen `Local::now`, leaves younger files, tolerates missing directory, returns non-fatal `Err` only on unreadable dir.
  - File: `crates/gwt-core/src/logging/housekeep.rs`
- [ ] TEST: Integration test (tempdir): `init(LoggingConfig { log_dir: tmp.path().into(), default_level: info })` followed by `tracing::info!(target: "t", key = "v", "hello")` produces a JSONL line in `tmp/gwt.log` containing `"level":"INFO"`, `"target":"t"`, `"message":"hello"`, `"fields":{"key":"v"}`.
  - File: `crates/gwt-core/tests/logging_init.rs`
- [ ] TEST: Integration test (tempdir): `init` returns a `reload_handle` whose `reload(EnvFilter::new("debug"))` immediately allows a subsequent `tracing::debug!` to reach the file.
  - File: `crates/gwt-core/tests/logging_reload.rs`
- [ ] IMPL: Create `crates/gwt-core/src/logging/mod.rs` exposing `init`, `LoggingConfig`, `LoggingHandles`, `UiLogEvent`, `LogLevel`.
- [ ] IMPL: `crates/gwt-core/src/logging/writer.rs` — build `tracing_appender::rolling::Builder` with the crate's UTC-dated daily rotation, `gwt.log` basename, produce a `non_blocking` writer + `WorkerGuard`.
- [ ] IMPL: `crates/gwt-core/src/logging/fmt_layer.rs` — JSONL format layer (`tracing_subscriber::fmt::layer().json().with_writer(non_blocking)`) with RFC3339 local timestamps.
- [ ] IMPL: `crates/gwt-core/src/logging/reload.rs` — build `Registry::default().with(reloadable_env_filter).with(fmt_layer).with(ui_forwarder_layer)`; return `reload::Handle`.
- [ ] IMPL: `crates/gwt-core/src/logging/ui_forwarder.rs` — `Layer<S>` impl; `on_event` visits fields into a `serde_json::Map`, builds `UiLogEvent`, and `sender.send` on a `tokio::sync::mpsc::UnboundedSender`; silently drops on `SendError`.
- [ ] IMPL: `crates/gwt-core/src/logging/housekeep.rs` — glob `gwt.log.*`, parse suffix as `chrono::NaiveDate` (UTC file date), compare to `today - 7 days`, remove stale, collect errors into a summary warning returned to caller.
- [ ] IMPL: Add `tracing`, `tracing-subscriber = { features = ["env-filter", "json", "fmt", "registry", "std"] }`, `tracing-appender`, `time` (for rolling), `serde_json`, `chrono` dependencies to `gwt-core/Cargo.toml`; update workspace as needed.

### 5.2 gwt-notification Removal (compile-breaking step — do it first)

- [ ] TEST: Delete every test that asserts behaviours specific to the `Notification` struct, `NotificationBus`, `NotificationRouter`, `NotificationLog`. Do not rewrite them. Phase 5 tests replace them.
- [ ] IMPL: Delete `crates/gwt-notification/` directory entirely.
- [ ] IMPL: Remove `gwt-notification` from `[workspace] members` in root `Cargo.toml`.
- [ ] IMPL: Remove `gwt-notification` from every crate's `Cargo.toml` `[dependencies]` section.
- [ ] IMPL: `rg "gwt_notification" crates/` → empty. Fix every call site by substituting a `tracing::*!` macro or a `UiLogEvent` variant per `plan.md` §Phase 5 Components.
- [ ] IMPL: Delete `crates/gwt-tui/src/notification_router.rs` and its `pub mod` in `lib.rs`.
- [ ] IMPL: Remove `model.notification_log` field and `apply_notification` function from `crates/gwt-tui/src/app.rs`; replace every caller with `tracing::{info,warn,error}!(target: "...", ...)`.

### 5.3 main.rs Wiring + Panic Hook

- [ ] TEST: Integration test: `crates/gwt-tui/tests/panic_hook_flushes_log.rs` — spawn a subprocess that calls `gwt_core::logging::init` in a tempdir, installs the panic hook, then `panic!("boom")`. Assert the tempdir `gwt.log` contains a final line with `"level":"ERROR"` and `"message"` containing `"boom"` and `"backtrace"` field.
- [ ] IMPL: `crates/gwt-tui/src/main.rs` — call `gwt_core::logging::init(...)` at the top of `main`, keep the returned `LoggingHandles` alive until the end of `main`.
- [ ] IMPL: `crates/gwt-tui/src/panic.rs` (new) — install a panic hook that: (1) captures `std::backtrace::Backtrace::capture`, (2) calls `tracing::error!(target: "gwt_tui::panic", backtrace = ?bt, message = %info, "panic")`, (3) calls `handles.flush_blocking()`, (4) invokes the previous hook (ratatui terminal restore).
- [ ] IMPL: `LoggingHandles::flush_blocking` — sync wrapper that drops-and-replaces the `WorkerGuard` to force a flush of the non-blocking writer (or uses `tracing_appender`'s explicit flush if available in the chosen version).

### 5.4 Logs Tab File Backing

- [ ] TEST: Unit test: JSONL parser handles well-formed lines, malformed lines (kept as `message` with `level=ERROR` + `parse_error` field), empty lines, partial trailing line (buffer until newline).
  - File: `crates/gwt-tui/src/logs_watcher/parser.rs`
- [ ] TEST: Integration test (tempdir): with `LoggingHandles` + `logs_watcher::spawn(ui_tx, log_path)`, write a `tracing::info!` and assert that within 500 ms the TUI app message channel receives `LogsMessage::AppendEntries` containing that entry.
  - File: `crates/gwt-tui/tests/logs_watcher_e2e.rs`
- [ ] TEST: Integration test (tempdir): when today's file already has 100 lines at startup, the initial read produces a single `LogsMessage::SetEntries(100 entries)` message before any `AppendEntries`.
- [ ] TEST: Integration test (tempdir): on file rotation (rename today → yesterday; create new today), the watcher continues to observe appends to the new today without manual refresh.
- [ ] TEST: Unit test: `LogsMessage::Refresh` re-reads today's file and emits `SetEntries`.
- [ ] IMPL: `crates/gwt-tui/src/logs_watcher/mod.rs` (new) — `spawn(ui_tx: UnboundedSender<Message>, log_dir: PathBuf)` returns a `JoinHandle`; internally maintains `last_offset: u64` and `current_inode: u64`.
- [ ] IMPL: `crates/gwt-tui/src/logs_watcher/parser.rs` — `parse_line(&str) -> LogEntry` (tolerates failures by producing an ERROR-level synthetic entry).
- [ ] IMPL: `crates/gwt-tui/src/logs_watcher/watch.rs` — `notify::RecommendedWatcher` on `log_dir`, debounced; on `Modify { kind: Data(Any) }` on today's file, seek to `last_offset`, read to end, parse, send `AppendEntries`. On `Create` of a new `gwt.log` (rotation), reset offset and inode.
- [ ] IMPL: Replace `pub use gwt_notification::Notification as LogEntry` in `crates/gwt-tui/src/screens/logs.rs` with a local struct `LogEntry { timestamp: DateTime<Local>, level: tracing::Level, target: String, message: String, fields: serde_json::Map<String, Value> }`. Preserve existing filter/render code with minimal type adjustments.
- [ ] IMPL: `LogsMessage::Refresh` handler in `crates/gwt-tui/src/app.rs` — send a refresh request to `logs_watcher`, which responds with `SetEntries`.
- [ ] IMPL: Map `tracing::Level → FilterLevel` correctly for `filtered_entries`.

### 5.5 UI Forwarder — Toast + Error Modal Driven by tracing

- [ ] TEST: Integration test (tempdir): `tracing::warn!(target: "t", "slow render")` results in `model.current_notification` being set to an Info/Warn toast with matching message within one tick.
  - File: `crates/gwt-tui/tests/ui_forwarder_warn_becomes_toast.rs`
- [ ] TEST: Integration test (tempdir): `tracing::error!(target: "t", detail = "timeout", "connect failed")` results in `model.error_queue` gaining an entry with message `"connect failed"` and detail `"timeout"`.
- [ ] TEST: Integration test: 500 back-to-back `tracing::error!` calls across two background threads — the `unbounded_channel` never blocks the emitting threads (measured latency < 1 ms p99 for the `info!` call) and all 500 entries appear in the file.
- [ ] IMPL: Add a background task in `crates/gwt-tui/src/app.rs` (or `crates/gwt-tui/src/ui_log_bridge.rs`) that reads `ui_rx: UnboundedReceiver<UiLogEvent>` and emits corresponding `Message::ShowNotification` / `Message::PushErrorNotification` messages (the old enum variants are kept, but now carry the new `UiLogEvent`-derived payload).
- [ ] IMPL: Update `crates/gwt-tui/src/widgets/status_bar.rs` and `crates/gwt-tui/src/widgets/error_modal.rs` to consume the new payload type (`UiLogEvent` or a simple `{level, message, detail}`). Preserve all existing render tests modulo the type swap.

### 5.6 Settings — Live Level Selector

- [ ] TEST: Unit test: `config.toml` schema round-trip for `[logging] level = "debug"`, missing section defaults to `info`, invalid value falls back to `info` with a `warn` event.
  - File: `crates/gwt-config/src/settings.rs`
- [ ] TEST: Widget render test: Settings screen shows a "Logging level" row with the current value and cycling it with `Space`/`Enter` emits `LoggingMessage::SetLevel`.
- [ ] TEST: Integration test (tempdir): invoking `SetLevel(Debug)` causes a subsequent `tracing::debug!` to appear in the file and in the Logs tab; reverting to `Info` stops it.
- [ ] IMPL: Add `LoggingConfig` to `crates/gwt-config/src/settings.rs`; update `load`/`save` and existing tests.
- [ ] IMPL: Add `LoggingMessage::SetLevel(LogLevel)` to `crates/gwt-tui/src/message.rs`.
- [ ] IMPL: Add a "Logging" section to `crates/gwt-tui/src/screens/settings.rs` with the level selector widget.
- [ ] IMPL: Wire the message handler in `app.rs` to call `handles.reload_handle.reload(EnvFilter::new(level.to_directive()))` and to persist the new value via `gwt_config::save`.
- [ ] IMPL: On level change, emit `tracing::info!(target: "gwt_tui::logging", from = %old, to = %new, "level changed")`.

### 5.7 agent_launch.log Integration

- [ ] TEST: Replace `append_agent_launch_log_with_writes_record_and_redacts_sensitive_env` and `append_agent_launch_log_with_appends_multiple_records` in `crates/gwt-tui/src/app.rs` with a new integration test `agent_launch_event_lands_in_gwt_log` that: (a) spawns `gwt_core::logging::init` in a tempdir, (b) calls the refactored agent launch flow, (c) asserts a JSONL line with `target == "gwt_tui::agent::launch"`, `fields.session_id`, `fields.agent`, `fields.workspace`, and a `fields.env` map that includes the original values (no redaction).
- [ ] TEST: Negative test: asserting that no file named `agent_launch.log` is created during an agent launch.
- [ ] IMPL: Delete `append_agent_launch_log`, `append_agent_launch_log_with`, and the redaction helper in `crates/gwt-tui/src/app.rs`.
- [ ] IMPL: Replace the call site with `tracing::info!(target: "gwt_tui::agent::launch", session_id = %sid, agent = %name, workspace = %path.display(), env = ?config.env, "agent launch")`.

### 5.8 Instrumentation Sweep

- [ ] TEST: Integration snapshot (tempdir): launching an agent produces at least one JSONL line whose `span` field contains `{"name":"agent_launch"}` and whose nested fields include the workspace.
  - File: `crates/gwt-tui/tests/instrument_agent_launch.rs`
- [ ] IMPL: Add `#[tracing::instrument(skip(self), fields(session_id = %self.id))]` to: `App::handle_session_switch`, `App::handle_tab_transition`, `agent::launch::launch_agent`, the top-level git op entrypoints in `gwt_core::git`, the docker ops entrypoints in `gwt_docker`, the `index_worker` tick loop.
- [ ] IMPL: Add a short section to `AGENTS.md` stating the convention: "New code in user-action paths should be annotated with `#[instrument]` whenever span context (session id, workspace, command) is not already implied by an outer span." (This is a doc change, not code.)

### 5.9 Cleanup and Verification Gate

- [ ] IMPL: Delete every `Notification`-era TODO comment and stale field. Run `rg "notification_log|NotificationBus|NotificationRouter|gwt_notification" crates/` — must be empty.
- [ ] IMPL: `cargo fmt --all`
- [ ] IMPL: `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] IMPL: `cargo test --workspace`
- [ ] IMPL: Manual smoke: `cargo run -p gwt-tui`, confirm `~/.gwt/logs/gwt.log` is populated, Logs tab streams events, Settings level toggle works, panic from a debug menu (or temporary `unreachable!`) leaves a final entry.
- [ ] IMPL: Update `README.md` / `README.ja.md` "Troubleshooting" section: "Logs are written to `~/.gwt/logs/gwt.log` as JSONL; tail with `tail -f ~/.gwt/logs/gwt.log | jq` or open the Logs tab inside the TUI."
- [ ] IMPL: Update `specs/SPEC-6/metadata.json` `status` to `done` and `phase` to `Phase 5: complete` on the final commit of the PR; add a `progress.md` entry summarizing the migration.
