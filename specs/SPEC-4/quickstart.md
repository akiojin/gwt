# Quickstart: SPEC-4 - GitHub Integration

## Reviewer Flow
1. Open the GitHub-backed management screens from the current workspace shell.
2. Inspect issues, PR summaries, and Git view output against the active repository state.
3. Confirm that branch linkage shows the right GitHub context for the checked-out branch.
4. Log any still-partial CI, review, divergence, or PR-link fields as
   acceptance gaps rather than unchecked execution tasks.

## Repeatable Evidence
- `cargo test -p gwt-git -- --nocapture`
- `cargo test -p gwt-tui git_view -- --nocapture`
- `cargo test -p gwt-tui pr_dashboard -- --nocapture`

## Expected Result
- The reviewer sees the current implemented scope for github integration.
- Any missing behavior is logged against acceptance gaps, not against unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
