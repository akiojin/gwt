# Data Model: SPEC-4 - GitHub Integration

## Primary Entities
### IssueSummary
- Role: Minimal issue record shown in the management surface.
- Invariant: Displayed issue state must stay consistent with fetched source data.

### PullRequestSummary
- Role: Aggregated PR status, review, and CI state for dashboard use.
- Invariant: Merge readiness must be derived from explicit source fields.

### BranchLink
- Role: Associates a local branch with GitHub issue or PR context.
- Invariant: Links must remain reversible and observable from Git View.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
