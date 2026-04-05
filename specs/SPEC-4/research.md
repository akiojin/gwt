# Research: SPEC-4 - GitHub Integration

## Scope Snapshot
- Canonical scope: Issues, pull requests, Git view state, and local branch linkage to GitHub records.
- Current status: `open` / `Ready for Dev`.
- Task progress: `0/36` checked in `tasks.md`.
- Notes: Only part of the supporting artifact set existed before this refresh, and the execution scope remains largely incomplete.

## Decisions
- Keep Issues, PRs, Git View, and branch linkage together because they share GitHub and local-git context.
- Treat existing screen scaffolding as partial delivery, not as evidence that the full GitHub workflow is complete.
- Preserve room for connector-driven details such as CI, reviews, and divergence state without inventing them in docs.

## Open Questions
- Confirm which GitHub data must be available offline versus fetched live at view time.
- Decide whether branch linkage should be driven from local git metadata, GitHub queries, or both.
