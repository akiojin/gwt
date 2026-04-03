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
- [ ] TEST: Unit test for log entry format (timestamp, severity, source, message)
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
- [ ] TEST: Performance test: 100 simultaneous errors do not freeze UI
