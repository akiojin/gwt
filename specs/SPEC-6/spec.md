# SPEC-6: Notification and Error Bus — Status Bar, Modal, Error Queue, Structured Log

## Background

gwt-tui currently has a basic error queue overlay that displays errors to the user. This SPEC defines a full notification system with 4 severity levels routing to appropriate display surfaces. The design is modeled after the old TUI's toast-style error display but extended with structured logging and severity-based routing.

## User Stories

### US-1 (P0): See Errors in Modal Dialog — PARTIALLY IMPLEMENTED

As a developer, I want errors to appear in a modal dialog that I must manually dismiss so that I do not miss critical failures.

**Acceptance Scenarios:**

- AC-1.1: Error notifications display in a modal dialog with message and details
- AC-1.2: Modal must be manually dismissed (Enter or Esc)
- AC-1.3: Multiple errors stack in a queue; dismissing one shows the next
- AC-1.4: Error queue handles 100+ simultaneous errors without UI freeze

### US-2 (P1): See Warnings/Info in Status Bar — PARTIALLY IMPLEMENTED

As a developer, I want warnings and info messages to appear in the status bar so that I am informed without interrupting my workflow.

**Acceptance Scenarios:**

- AC-2.1: Info notifications display in the status bar notification area (right side)
- AC-2.2: Info notifications auto-dismiss after 5 seconds
- AC-2.3: Warning notifications display in the status bar with a distinct color (yellow/amber)
- AC-2.4: Warning notifications persist until manually dismissed
- AC-2.5: Only the most recent notification shows; older ones are in the log

### US-3 (P1): View Structured Log History — PARTIALLY IMPLEMENTED

As a developer, I want to view a structured log of all notifications so that I can review past events and diagnose issues.

**Acceptance Scenarios:**

- AC-3.1: All notifications are logged with timestamp, severity, source, and message
- AC-3.2: Logs tab displays the structured log with filtering by severity
- AC-3.3: Log entries are scrollable and searchable

### US-4 (P2): Debug Messages Logged but Not Displayed — PARTIALLY IMPLEMENTED

As a developer, I want debug-level messages to be logged without being displayed so that I can review them when troubleshooting.

**Acceptance Scenarios:**

- AC-4.1: Debug notifications are written to the structured log
- AC-4.2: Debug notifications are never shown in the UI (no modal, no status bar)
- AC-4.3: Debug entries are visible in the Logs tab when debug filter is enabled

## Severity Routing

| Severity | Display Surface | Behavior |
|----------|----------------|----------|
| Debug | Structured log only | Not displayed in UI |
| Info | Status bar (right side) | Auto-dismiss after 5 seconds |
| Warn | Status bar with color | Persist until manually dismissed |
| Error | Modal dialog | Must be manually dismissed |

## Functional Requirements

| ID | Requirement | Priority | Status |
|----|-------------|----------|--------|
| FR-001 | 4-level severity enum: Debug, Info, Warn, Error | P0 | Not Implemented |
| FR-002 | Severity-to-surface routing as defined in routing table | P0 | Not Implemented |
| FR-003 | Status bar notification area (right side) with auto-dismiss timer | P1 | Not Implemented |
| FR-004 | Modal dialog for errors with message, details, dismiss button | P0 | Partially Implemented |
| FR-005 | Error queue: multiple errors stack, dismiss one at a time | P0 | Partially Implemented |
| FR-006 | Structured log: all notifications logged with timestamp, severity, source, message | P1 | Not Implemented |
| FR-007 | Logs tab shows filtered structured log (existing implementation extended) | P1 | Partially Implemented |
| FR-008 | Notification bus is async, non-blocking to UI thread | P1 | Not Implemented |

## Non-Functional Requirements

| ID | Requirement |
|----|-------------|
| NFR-001 | Notification display under 50ms from event |
| NFR-002 | Error queue handles 100+ simultaneous errors without UI degradation |

## Design Notes

- The notification bus is an async channel (`tokio::sync::mpsc` or similar) that decouples event producers from UI consumers
- Severity enum is defined in gwt-core so both core and TUI can use it
- Status bar notification area occupies the right portion of the existing status bar
- Structured log format: `{timestamp, severity, source, message}` stored in memory with optional file persistence

## Success Criteria

1. All severity levels route to the correct display surface as defined in the routing table
2. Status bar shows info/warn notifications with correct auto-dismiss and persistence behavior
3. Error modal queue handles concurrent errors gracefully
4. Structured log captures all notifications and is viewable in the Logs tab
5. Notification bus does not block the UI thread (async, under 50ms display latency)

---

## Phase 5: tracing-based Structured Logging Migration

### Background

Phase 1–4 landed an in-memory `Notification` / `notification_log` system that only works while the TUI is running. Investigation on 2026-04-08 confirmed the following defects that invalidate the original NFR for diagnostic value:

1. **`tracing_subscriber` is never initialised** (no matches in `crates/*/src`). Every existing `tracing::info!/warn!/error!/debug!` call across `gwt-tui`, `gwt-docker`, `gwt-config`, `gwt-agent`, `gwt-terminal`, `gwt-core` is dropped silently.
2. **Logs tab is purely in-memory.** `LogsState.entries` is populated only via `apply_notification` → `model.notification_log`. Restarting the TUI loses every event.
3. **`~/.gwt/logs/` holds only `agent_launch.log`.** No other file-based logging exists. The "optional file persistence" clause in Phase 3 was never implemented.
4. **`LogsMessage::Refresh` is a no-op.** `r` keybinding in `app.rs` is wired but handler has only a comment.

Phase 5 replaces the in-memory notification pipeline with a `tracing`-based file logging foundation. The `Notification` struct, `gwt-notification` crate, `NotificationBus`, `NotificationRouter`, `notification_log`, and `StructuredLog` ring buffer are removed in the same PR. UI surfaces (toast area on status bar, error modal, error queue) are retained but driven by `tracing::Event` → dedicated `tracing_subscriber::Layer` → UI channel instead of `Notification` objects.

### US-5 (P0): Diagnose Post-Mortem Crashes from Log Files

As a developer responding to a user bug report, I want all `tracing` events to be persisted on disk in a parseable format so that I can investigate crashes after the TUI has exited.

**Acceptance Scenarios:**

- AC-5.1: Starting `gwt-tui` creates `~/.gwt/logs/gwt.log` if it does not exist
- AC-5.2: `tracing::info!`, `warn!`, `error!`, `debug!` calls from any crate appear as JSONL lines in `gwt.log`
- AC-5.3: Each JSONL line contains at minimum: `timestamp` (RFC3339 local), `level`, `target`, `message`, `fields` (tracing kv), and span context when inside an `#[instrument]` span
- AC-5.4: The file is retained across restarts and rotations (no truncation on startup)
- AC-5.5: When TUI panics, the panic message and backtrace land in `gwt.log` before the process exits (panic hook forces flush of the non-blocking writer)

### US-6 (P0): Real-Time In-TUI Log Observation

As a developer using the TUI, I want the Logs tab to stream new events as they are written so that I can observe what the system is doing without leaving the TUI.

**Acceptance Scenarios:**

- AC-6.1: Opening the Logs tab shows every event from today's `gwt.log` file (today = current UTC day)
- AC-6.2: New `tracing` events emitted while the Logs tab is open appear within 500 ms of being written to the file (notify-crate driven; TUI tick fallback only if notify fails to start)
- AC-6.3: Existing filter UX (`FilterLevel::{All,ErrorOnly,WarnUp,InfoUp,DebugUp}` and Debug toggle) continues to work against the file-backed stream
- AC-6.4: Rotation at UTC midnight is observed: Logs tab keeps showing the new day's file from the moment it is created

### US-7 (P0): Warn/Error Events Still Surface in UI

As a developer, I want warnings and errors to continue appearing as toasts (status bar) and modal dialogs (error queue) so that I am not required to open the Logs tab to notice them.

**Acceptance Scenarios:**

- AC-7.1: `tracing::warn!` from any crate produces a status-bar toast with the event message
- AC-7.2: `tracing::error!` from any crate enqueues an error modal with the event message and any `error.detail` field
- AC-7.3: The existing auto-dismiss (5 s Info), manual-dismiss (Warn), and modal-queue semantics are preserved
- AC-7.4: Direct `tracing::info!` calls also produce Info toasts (replacing former `Notification::info` calls)
- AC-7.5: If a background thread produces warn/error events while the UI is busy, no event is lost from the file (unbounded mpsc channel to UI; file write path is the single source of truth)

### US-8 (P1): Live Log Level Control From Settings

As a developer diagnosing a user report, I want to toggle the logging level live from inside the TUI without restarting so that the user can reproduce the issue with the right verbosity.

**Acceptance Scenarios:**

- AC-8.1: Settings screen (or Logs tab overlay) exposes a level selector: `ERROR / WARN / INFO / DEBUG / TRACE`
- AC-8.2: Selecting a level updates the `tracing_subscriber::EnvFilter` via `reload::Handle::reload` immediately
- AC-8.3: `RUST_LOG` environment variable at startup takes precedence over the saved level when set; otherwise default is `info` for all crates
- AC-8.4: Selected level is persisted to `~/.gwt/config.toml` (`[logging] level = "debug"`) so it survives restart
- AC-8.5: The level change itself is logged (`tracing::info!(target: "gwt_tui::logging", "level changed to {new}")`)

### US-9 (P1): Daily Rotation, Seven-Day Retention

As a developer, I want the log file to rotate automatically so that disk usage stays bounded without manual cleanup.

**Acceptance Scenarios:**

- AC-9.1: Rotation occurs at UTC midnight (00:00Z) via `tracing_appender::rolling::daily`
- AC-9.2: Rotated files are named `gwt.log.YYYY-MM-DD` using the UTC date
- AC-9.3: On TUI startup a housekeeping pass deletes any `gwt.log.*` entry older than seven days (keeps at most today + 7 historical files)
- AC-9.4: Housekeeping failures do not prevent TUI startup (logged as `warn` and continue)

### US-10 (P2): agent_launch.log Integration

As a developer, I want agent-launch events to land in the same log file as every other diagnostic event so that I only have one file to grep.

**Acceptance Scenarios:**

- AC-10.1: `append_agent_launch_log_with` is replaced by `tracing::info!(target: "gwt_tui::agent::launch", session_id, agent, workspace, env = ?filtered_env, …)`
- AC-10.2: `~/.gwt/logs/agent_launch.log` is no longer created
- AC-10.3: All existing redaction logic for sensitive env vars is **removed** (see FR-016: no sanitisation; the log file is a user-private artifact)
- AC-10.4: Existing `agent_launch.log` files on disk are left untouched (no migration, no deletion); the code path that created them is removed
- AC-10.5: Tests that previously verified redaction behaviour for `agent_launch.log` are deleted, not rewritten

### Updated Functional Requirements (Phase 5)

| ID     | Requirement                                                                                                                                                                               | Priority | Status          |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- | --------------- |
| FR-009 | Initialize `tracing_subscriber` in `gwt-tui` `main` with a non-blocking JSONL file writer rolling daily (UTC boundary) at `~/.gwt/logs/gwt.log`                                            | P0       | Not Implemented |
| FR-010 | Default `EnvFilter` is `info` for all crates; `RUST_LOG` overrides at startup                                                                                                             | P0       | Not Implemented |
| FR-011 | `reload::Handle` exposed to the app model so the Settings screen can change the level at runtime                                                                                          | P1       | Not Implemented |
| FR-012 | `std::panic::set_hook` forces a synchronous flush of the non-blocking writer before re-panicking; must compose with existing ratatui terminal restoration                                 | P0       | Not Implemented |
| FR-013 | Logs tab watches `~/.gwt/logs/gwt.log` via `notify` crate, parses each appended line as JSONL into `LogEntry`, and appends to `LogsState.entries` (existing filter logic stays untouched) | P0       | Not Implemented |
| FR-014 | Logs tab performs an initial read of today's file (current UTC day) on TUI startup, not when the tab is first opened                                                                      | P0       | Not Implemented |
| FR-015 | A dedicated `tracing_subscriber::Layer` forwards `Warn` and `Error` events over a `tokio::sync::mpsc::UnboundedSender<UiLogEvent>` to the main TUI loop, which drives toasts/error modal  | P0       | Not Implemented |
| FR-016 | No redaction layer: JSONL lines contain raw field values. `~/.gwt/logs/` is hardened on Unix to dir `0700` / files `0600` so only the owning user can read structured logs (revised after reviewer comment B7) | P1       | Implemented |
| FR-017 | Housekeeping: on startup delete `gwt.log.YYYY-MM-DD` files older than 7 days relative to today's UTC date                                                                                 | P1       | Not Implemented |
| FR-018 | `gwt-notification` crate is deleted. `Notification`, `NotificationBus`, `NotificationRouter`, `NotificationLog`, `notification_log` field on `Model`, and every call site are removed     | P0       | Not Implemented |
| FR-019 | `LogEntry` (formerly `pub use gwt_notification::Notification as LogEntry`) is redefined locally in `crates/gwt-tui/src/screens/logs.rs` as `{timestamp, level, target, message, fields}`  | P0       | Not Implemented |
| FR-020 | `append_agent_launch_log_with` is deleted; `agent_launch.log` file is no longer written                                                                                                   | P1       | Not Implemented |
| FR-021 | Major user actions (session switch, tab transition, agent launch flow, git ops, docker ops, index worker tick) are wrapped in `#[instrument]` spans so Logs tab shows span context        | P2       | Not Implemented |
| FR-022 | `LogsMessage::Refresh` (`r` key) performs a full reread of today's file — useful after notify events are missed                                                                           | P2       | Not Implemented |

### Phase 5 Non-Functional Requirements

| ID      | Requirement                                                                                                                                              |
| ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| NFR-003 | Writing a `tracing::info!` event must not block the TUI render thread for more than 1 ms in the happy path (non-blocking writer)                        |
| NFR-004 | Logs tab end-to-end latency (event emitted → visible in the tab) ≤ 500 ms on macOS / Linux when Logs tab is active                                       |
| NFR-005 | 10,000 rapid `tracing::info!` events in under 1 second must all be persisted to `gwt.log` and none lost (file is source of truth)                        |
| NFR-006 | Logs tab must not block when switching away: file watcher and parser run on a dedicated background task, not the render loop                              |
| NFR-007 | When `gwt.log` reaches 500 MB mid-day (pathological case), the TUI must still start within 2 s (initial read is bounded to "today" using rolling suffix) |

### Phase 5 Success Criteria

1. `cargo run -p gwt-tui` produces a non-empty `~/.gwt/logs/gwt.log` JSONL file within the first second of startup
2. `tail -f ~/.gwt/logs/gwt.log | jq -c '{ts, level, target, message}'` shows events in real time
3. Killing the TUI with `SIGKILL` does not corrupt the file; restarting appends to the same daily file
4. Causing a panic (deliberate `unreachable!`) leaves a final `panic` event with backtrace in `gwt.log`
5. `grep -r "use gwt_notification" crates/` returns zero results; `crates/gwt-notification` directory is deleted
6. The Logs tab, opened after 5 minutes of activity, shows every event from today — not just events since the tab was opened
7. Setting `RUST_LOG=gwt_tui=debug,gwt_docker=trace` shows those crates at the elevated level and only those
8. Changing the level via Settings UI updates the live filter without restart and persists to `config.toml`

### Out of Scope for Phase 5

- Remote log shipping (syslog, HTTP endpoint, OTLP exporter)
- Encryption / integrity (HMAC) of log files
- Log redaction / sanitisation of any kind
- Multi-instance coordination beyond POSIX `O_APPEND` semantics (advisory locks, per-pid files)
- CLI subcommand for tailing/pretty-printing the log file
- Logs tab search / source filter / JSON detail view
- Per-target EnvFilter editing UI beyond a single global level selector
- Migration / backfill of historical `agent_launch.log` content
