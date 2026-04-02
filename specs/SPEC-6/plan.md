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
