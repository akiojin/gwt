1. Reopen #1354 only when changing GitHub Issue detail or legacy issue-body compatibility. Use `SPEC-1776` for local SPEC viewing.
2. Add RED tests for index-only body + `doc:*` artifact reconstruction.
3. Update `issue_spec.rs` and `commands/issue_spec.rs` to prefer `doc:*` comments.
4. Verify `IssueSpecPanel` and `IssueListPanel` against both artifact-first and legacy issues.
5. Update #1643 to reference #1354 for detail-view behavior.
