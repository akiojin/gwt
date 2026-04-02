1. Use #1579 to reason about workflow order, registration, storage/API, and completion gate.
2. Use `SPEC-1776` to reason about local SPEC viewing and `#1354` only for GitHub Issue detail / legacy issue-body compatibility.
3. Use #1643 to reason about search and discovery only.
4. Treat migration as part of the redesign, not a separate cleanup task.
5. For embedded GitHub skills, choose REST first for metadata/check/comment paths, and keep GraphQL only for unresolved review-thread operations that still lack practical REST coverage.
6. Update `issue_spec.rs` to prefer `doc:*` artifacts. Keep `SpecIssueDetail.sections` stable for the frontend.
7. Reconcile `doc:spec.md`, `doc:tasks.md`, `checklist:tdd.md`, `checklist:acceptance.md`, and progress comments before declaring a SPEC done.
8. Apply the completion gate to #1654 before restoring its completed state.
