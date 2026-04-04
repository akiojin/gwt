# Quickstart: SPEC-4 - GitHub Integration

## Reviewer Flow
1. Open the GitHub-backed management screens from the current workspace shell.
2. Inspect issues, PR summaries, and Git view output against the active repository state.
3. Switch to the Git View tab and confirm the header shows ahead/behind
   divergence and current PR link when the checked-out branch has both.
4. Switch to the PR Dashboard tab and confirm the list populates on tab focus.
5. Open PR detail with `Enter` and confirm CI / merge / review detail is loaded
   for the selected PR.
6. While PR detail stays open, use `Up` / `Down` to move selection and confirm
   the detail pane reloads for the newly selected PR.
7. Switch away from PR Dashboard and back, then confirm an already-open detail
   view reloads from live repo state instead of falling back to summary-only
   text.
8. Press `r` in PR Dashboard and confirm the PR list and open detail refresh
   from live repo state.
9. Confirm that branch linkage shows the right GitHub context for the
   checked-out branch.
10. Log any still-partial PR dashboard presentation gaps as acceptance gaps
    rather than unchecked execution tasks.

## Repeatable Evidence
- `cargo test -p gwt-git -- --nocapture`
- `cargo test -p gwt-tui git_view -- --nocapture`
- `cargo test -p gwt-tui pr_dashboard -- --nocapture`
- `cargo test -p gwt-tui load_git_view_with_populates_divergence_and_pr_link_metadata -- --nocapture`
- `cargo test -p gwt-tui load_git_view_with_omits_divergence_without_upstream -- --nocapture`
- `cargo test -p gwt-tui switch_management_tab_pr_dashboard_loads_prs_on_focus -- --nocapture`
- `cargo test -p gwt-tui switch_management_tab_pr_dashboard_reloads_detail_when_open -- --nocapture`
- `cargo test -p gwt-tui refresh_pr_dashboard_with_reloads_prs -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_pr_dashboard_enter_loads_detail_report -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_pr_dashboard_move_in_detail_view_reloads_selected_pr_detail -- --nocapture`
- `cargo test -p gwt-tui refresh_pr_dashboard_with_in_detail_view_updates_detail_report -- --nocapture`

## Expected Result
- The reviewer sees the current implemented scope for github integration.
- Any missing behavior is logged against acceptance gaps, not against unchecked
  tasks.
- No step should be treated as complete unless the code path is actually reachable today.
