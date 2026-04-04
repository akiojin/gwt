# Quickstart: SPEC-4 - GitHub Integration

## Reviewer Flow
1. Open the GitHub-backed management screens from the current workspace shell.
2. Inspect issues, PR summaries, and Git view output against the active repository state.
3. Switch to the PR Dashboard tab and confirm the list populates on tab focus.
4. Press `r` in PR Dashboard and confirm the PR list refreshes from live repo state.
5. Confirm that branch linkage shows the right GitHub context for the checked-out branch.
6. Log any still-partial CI, review, divergence, or PR-link fields as
   acceptance gaps rather than unchecked execution tasks.

## Repeatable Evidence
- `cargo test -p gwt-git -- --nocapture`
- `cargo test -p gwt-tui git_view -- --nocapture`
- `cargo test -p gwt-tui pr_dashboard -- --nocapture`
- `cargo test -p gwt-tui switch_management_tab_pr_dashboard_loads_prs_on_focus -- --nocapture`
- `cargo test -p gwt-tui refresh_pr_dashboard_with_reloads_prs -- --nocapture`

## Expected Result
- The reviewer sees the current implemented scope for github integration.
- Any missing behavior is logged against acceptance gaps, not against unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
