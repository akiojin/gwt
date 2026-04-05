# Data Model: SPEC-6 - Notification and Error Bus

## Primary Entities
### NotificationEvent
- Role: Structured event emitted by producers with severity and message payload.
- Invariant: Severity must deterministically select the UI destination.

### ErrorQueue
- Role: Ordered queue of unresolved high-severity notifications.
- Invariant: Errors must not be dropped silently when the modal is closed.

### StructuredLogRecord
- Role: Persisted record for log review and diagnostics.
- Invariant: Log output must stay aligned with routed notification content.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
