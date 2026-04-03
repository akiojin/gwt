# Data Model: SPEC-5 - Local SPEC Management

## Primary Entities
### SpecIndexEntry
- Role: Summary row for one local SPEC directory.
- Invariant: Index data must stay synchronized with `metadata.json` and artifact presence.

### SpecDetailState
- Role: Expanded view of one SPEC and its editable fields.
- Invariant: Edits must persist back to disk before they are treated as accepted.

### SpecLaunchRequest
- Role: Context passed from a SPEC detail view into agent-launch flows.
- Invariant: Launch context must not invent missing SPEC metadata.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
