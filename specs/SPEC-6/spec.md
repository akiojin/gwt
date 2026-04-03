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
