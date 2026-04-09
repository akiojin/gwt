# SPEC-6: Implementation Plan

## Phase 1: Notification Bus and Status Bar

**Goal:** Establish the async notification bus and implement status bar notifications for Info and Warn levels.

### Approach

- Define `Notification` struct and `Severity` enum in gwt-core
- Implement async notification channel (tokio mpsc)
- Add status bar notification area to the TUI layout

### Components

1. **Severity enum** — `Debug`, `Info`, `Warn`, `Error` in gwt-core
2. **Notification struct** — `{timestamp, severity, source, message}`
3. **NotificationBus** — Async mpsc channel: producers send, UI consumes
4. **Status bar area** — Right-side region in existing status bar for notification display
5. **Auto-dismiss timer** — 5-second timer for Info notifications
6. **Warn persistence** — Warn notifications stay until user dismisses (keybinding)

### Key Decisions

- tokio mpsc is chosen over crossbeam because the project already uses tokio
- Status bar notification area shares the existing status bar widget (no new bar)

## Phase 2: Severity Routing

**Goal:** Implement the severity-to-surface routing logic and upgrade the existing error modal.

### Approach

- Central router receives all notifications and dispatches to the correct surface
- Upgrade existing error queue overlay to use the new Notification struct
- Add severity-based filtering

### Components

1. **Router** — Match severity to surface: Debug->log, Info->status bar, Warn->status bar, Error->modal
2. **Error modal upgrade** — Refactor existing error queue to accept `Notification` objects
3. **Dismiss handlers** — Enter/Esc for modal, keybinding for status bar warnings

## Phase 3: Structured Log Integration

**Goal:** Log all notifications with structured format and extend the Logs tab.

### Approach

- All notifications (including Debug) are written to an in-memory structured log
- Extend the existing Logs tab to display the structured log with severity filtering
- Optional file persistence for debug diagnostics

### Components

1. **Structured log store** — In-memory ring buffer with configurable capacity
2. **Log entry format** — `{timestamp, severity, source, message}`
3. **Logs tab extension** — Severity filter buttons, scrollable log list
4. **Debug filter** — Toggle to show/hide Debug entries in Logs tab

### Key Decisions

- Ring buffer prevents unbounded memory growth (default: 10,000 entries)
- File persistence is optional and disabled by default

## Dependencies

- tokio — async runtime (already in use)
- Existing error queue overlay — refactored in Phase 2
- Existing Logs tab — extended in Phase 3

---

## Phase 5: tracing-based Structured Logging (2026-04-08)

**Goal:** Replace the in-memory `Notification` pipeline with a `tracing`-based, file-persisted structured logging foundation and wire the Logs tab + UI surfaces to it. Everything in a single PR so there is no transient state where two logging systems coexist.

### Architectural Summary

```text
               ┌─────────────────────────────────────────────────────────┐
               │                 every crate / call site                │
               │  tracing::{info!,warn!,error!,debug!}, #[instrument]    │
               └──────────────────────────┬──────────────────────────────┘
                                          │
                                          ▼
┌──────────────────── tracing_subscriber::Registry ─────────────────────┐
│                                                                        │
│  1. EnvFilter (reload::Handle) ──► dynamic level control (Settings UI) │
│                                                                        │
│  2. JSONL fmt Layer             ──► tracing_appender::non_blocking     │
│                                     └► rolling::daily(local)          │
│                                         └► ~/.gwt/logs/gwt.log        │
│                                                                        │
│  3. UI forwarder Layer          ──► tokio::sync::mpsc::Unbounded      │
│                                     (Warn/Error/Info only)            │
│                                                                        │
└────────────────────────────────────────┬───────────────────────────────┘
                                          │
                                          ▼ (UI thread, in app.rs update loop)
                           ┌────────────────────────────┐
                           │   toast / error modal      │
                           │   (existing widgets, now   │
                           │    driven by UiLogEvent)   │
                           └────────────────────────────┘

Independent of the subscriber:

  notify crate ──► ~/.gwt/logs/gwt.log appends ──► JSONL parser ──► LogsState.entries
                                                                         ▲
                                                                         │
                                                          Logs tab render (unchanged filters)
```

Key invariant: **the file is the single source of truth**. The Logs tab does not receive events from the tracing pipeline; it re-reads the file that the writer layer produced. This keeps UI consistency across restarts and guarantees that what the developer sees in the Logs tab is exactly what will be in the bug report.

### Key Decisions (from 2026-04-08 interview)

| Topic                  | Decision                                                             | Rationale                                                                      |
| ---------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| Primary goal           | Post-mortem debug **and** realtime observability (equal weight)      | User decided both matter; forces full file persistence + file watcher         |
| File layout            | Single `~/.gwt/logs/gwt.log` + `gwt.log.YYYY-MM-DD` (daily rotation) | Simplest grep, single tail target                                              |
| Logs tab source        | File watch via `notify` crate                                        | Survives restarts, no parallel ring buffer to keep in sync                     |
| Level control          | Live reload via `tracing_subscriber::reload::Handle` + Settings UI   | User scenarios (support flow) require runtime toggle                           |
| Format                 | JSONL only                                                           | Machine-parseable, structured fields preserved; pretty CLI is out of scope     |
| Retention              | 7 days                                                               | Balances disk vs diagnostic value                                              |
| Write strategy         | `tracing_appender::non_blocking` + `WorkerGuard` held in main        | Never block TUI render thread                                                  |
| Panic                  | `std::panic::set_hook` forces flush before re-panic                  | Crash reports must include the last events                                     |
| Notification model     | **Abolished**. `gwt-notification` crate deleted                      | The only remaining coordinates are tracing events + LogEntry (file-derived)   |
| Warn/Error UI          | dedicated `Layer` forwards to UI channel                             | Preserves existing UX without keeping `Notification` alive                     |
| File watching          | `notify` crate (event-driven)                                        | Latency < 500 ms; no polling cost                                              |
| Initial read           | Current UTC day's full file                                          | Matches appender rotation boundary; bounded read cost                          |
| Redaction              | **None**                                                             | User's directory, user's responsibility. Combined with restrictive 0700/0600 file perms (revised after reviewer comment B7) |
| tracing depth          | Subscriber init + `#[instrument]` on major user actions              | Span context is a big uplift for post-mortem debug                             |
| Multi-instance         | Trust POSIX `O_APPEND`, single file                                  | Realistically rare; line-buffered JSONL is robust to interleaving              |
| Test strategy          | Tempdir + real file E2E                                              | Tests the full pipeline, catches wiring bugs                                   |
| Logs tab UX            | Status quo (severity filter, debug toggle)                           | No scope creep                                                                 |
| `agent_launch.log`     | Collapsed into `gwt.log` via `target: "gwt_tui::agent::launch"`      | One file to search                                                             |
| Default level          | `info` for all crates                                                | Balanced default; `RUST_LOG` escape hatch                                      |
| File permissions       | Restrictive on Unix (dir `0700`, file `0600`); Windows ACL default | Reviewer comment B7 — combined with no-redaction, the previous `0644` default exposed tokens to other local users on shared hosts. Tightened in `crates/gwt-core/src/logging/writer.rs::tighten_log_dir_permissions`. |
| Rotation boundary      | UTC midnight                                                         | Matches `tracing_appender 0.2.4` file naming and watcher behavior              |
| UI channel             | `tokio::sync::mpsc::unbounded_channel`                               | tracing must never block background threads                                    |
| Removal plan           | Single PR, no staged deprecation                                     | Avoid a transient mixed-world                                                  |

### Phase 5 Components

1. **`crates/gwt-core/src/logging/mod.rs`** (new) — central module owning:
   - `init(config: LoggingConfig) -> Result<LoggingHandles>` that returns `{ _guard: WorkerGuard, reload_handle: reload::Handle, ui_rx: UnboundedReceiver<UiLogEvent> }`
   - `LoggingConfig { log_dir: PathBuf, default_level: LevelFilter }` (produced from `config.toml` + `RUST_LOG`)
   - `UiLogEvent { level, target, message, fields, timestamp }`
   - `housekeep(log_dir, retention_days=7)` — deletes `gwt.log.YYYY-MM-DD` older than cutoff
   - Internal `UiForwarderLayer` — a `tracing_subscriber::Layer<S>` impl that visits event fields, builds `UiLogEvent`, and `send()`s to the channel (drops silently if the receiver has been dropped during shutdown)
2. **`crates/gwt-tui/src/main.rs`** — call `gwt_core::logging::init` at the top of `main`, thread `LoggingHandles` into the app builder, set up `std::panic::set_hook` (capture backtrace, `tracing::error!(target: "gwt_tui::panic", ...)`, drop the guard, call the previous hook for ratatui terminal restoration).
3. **`crates/gwt-tui/src/app.rs`** — replace the `notification_log` field and every `apply_notification` call. Add an `async` background task that reads `ui_rx` and drives `model.current_notification` / `model.error_queue` (existing widgets unchanged). Remove every `use gwt_notification::*` import. Replace `append_agent_launch_log(_with)` with `tracing::info!(target: "gwt_tui::agent::launch", …)`.
4. **`crates/gwt-tui/src/screens/logs.rs`** —
   - Replace `pub use gwt_notification::Notification as LogEntry` with a local struct `LogEntry { timestamp: DateTime<Local>, level: tracing::Level, target: String, message: String, fields: serde_json::Map<String, Value> }`.
   - Replace `Severity` references with `tracing::Level` (map `Level::ERROR → FilterLevel::ErrorOnly`, etc.).
   - Keep `LogsState`, `FilterLevel`, render logic unchanged apart from the type swap.
   - Implement `LogsMessage::Refresh` to re-read today's file.
5. **`crates/gwt-tui/src/logs_watcher.rs`** (new) — background task:
   - On startup, parse today's `gwt.log` and send a `LogsMessage::SetEntries` to the app via an existing message channel.
   - Spin up a `notify::RecommendedWatcher` on `~/.gwt/logs/`; on `Modify` for `gwt.log`, resume reading from the last known offset, parse new JSONL lines, send a `LogsMessage::AppendEntries` message.
   - Handle rotation (file size shrinking / inode change) by reopening.
6. **`crates/gwt-tui/src/screens/settings.rs`** — add a `Logging` section with the level selector. Wire it to `LoggingHandles::reload_handle` via a message `LoggingMessage::SetLevel(LevelFilter)`.
7. **`crates/gwt-config/src/settings.rs`** — add `LoggingConfig { level: Option<String> }` to the on-disk schema. Round-trip tests.
8. **`crates/gwt-tui/src/message.rs`** — add `LogsMessage::AppendEntries(Vec<LogEntry>)`, `Message::Logging(LoggingMessage)`, `Message::UiLogEvent(UiLogEvent)`.
9. **Deletion**: `crates/gwt-notification/` directory, its entry in root `Cargo.toml` workspace members, every `use gwt_notification::*` reference, and every test that covered only the old model.

### Phase 5 Build Order

1. **Step 1 — foundation (RED → GREEN):**
   1. Delete `crates/gwt-notification` and every consumer (compile-broken intentionally).
   2. Add `gwt_core::logging::init` with JSONL writer, `WorkerGuard`, reload handle, UI forwarder layer.
   3. Re-wire `gwt-tui/src/main.rs` to call `init`; thread handles into `App`.
   4. Hook the panic hook.
   5. Repair every broken call site in `app.rs`, `screens/*`, `widgets/error_modal.rs`, `widgets/status_bar.rs`, `custom_agents.rs`, `index_worker.rs` using the new `UiLogEvent` path and direct `tracing::*!` macros.
   6. First green: `cargo build -p gwt-tui` succeeds.
2. **Step 2 — Logs tab file backing:**
   1. New `logs_watcher` task with a tempdir E2E test that emits a `tracing::info!` and asserts the `LogsState` observes the entry within 500 ms.
   2. Initial read of today's file for the current UTC day.
   3. Implement `r` refresh.
3. **Step 3 — UI forwarder + Settings level selector:**
   1. Tempdir E2E: emit `tracing::error!`, observe that the error modal queue receives the event.
   2. Settings screen level selector + `config.toml` round-trip + reload handle.
4. **Step 4 — Housekeeping + rotation + panic:**
   1. Test for 7-day housekeeping (tempdir, synthetic old files).
   2. Test for rotation observation (notify survives inode change).
   3. Test for panic hook flushing the final event.
5. **Step 5 — instrumentation sweep:**
   1. Add `#[instrument]` to the agreed functions (session switch, tab transition, agent launch flow, git ops entrypoints, docker ops entrypoints, index worker tick).
   2. No behaviour tests; rely on integration snapshot of the emitted JSONL for one representative flow (e.g. agent launch).

### Risks and Mitigations

| Risk                                                                                     | Mitigation                                                                                                                                           |
| ---------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `notify` crate unreliable on macOS FSEvents coalescing                                   | Use debouncer, and include a manual `r` refresh as the escape hatch (FR-022)                                                                         |
| Panic hook + ratatui terminal restore ordering                                           | Install tracing panic hook **after** capturing the previous hook, and call the previous hook last so the terminal is restored                       |
| WorkerGuard dropped before panic message is written                                      | Hold `_guard` in `main`'s scope as the very last local; the panic hook calls `std::mem::drop` on a clone-free guard before re-panicking             |
| Mass deletion of `gwt-notification` breaks unrelated tests                               | Do it in the first commit of the feature branch; compile errors are the checklist; no tests gated on the old types are rewritten — they are deleted |
| Logs tab reads entire day file on each refresh                                           | Track `last_offset`; refresh re-seeks from the start only when requested                                                                             |
| Multi-instance JSONL interleaving beyond `PIPE_BUF` (4 KiB)                              | Keep single-line JSONL < 4 KiB in the happy path; truncate extremely long fields (> 64 KiB) with `fields: {truncated: true}` in the fmt layer      |
| tracing events from `Drop` impls during shutdown fire after `WorkerGuard` drop           | UI forwarder silently ignores send errors; file writer is best-effort; this is acceptable for shutdown                                              |
| `config.toml` schema change breaks existing users                                        | Treat `[logging]` section as optional with a default; round-trip test for missing section                                                            |
