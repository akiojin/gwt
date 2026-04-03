# Quickstart: SPEC-6 - Notification and Error Bus

## Reviewer Flow
1. Trigger representative debug, info, warning, and error events in the current branch.
2. Verify what appears in the status bar, what persists, and what enters the error queue.
3. Inspect the logs view to confirm structured records match the emitted notifications.
4. Record any missing severity routing as remaining SPEC-6 execution work.

## Repeatable Evidence
- `cargo test -p gwt-tui e2e_notifications_land_in_structured_log_for_all_severities -- --nocapture`
- `cargo test -p gwt-tui notification -- --nocapture`
- `cargo test -p gwt-tui error -- --nocapture`
- `cargo test -p gwt-tui logs -- --nocapture`
- `cargo test -p gwt-core -p gwt-tui`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Expected Result
- The reviewer sees the current implemented scope for notification and error bus.
- Warn notifications can be dismissed without stealing `Esc` from existing search/edit flows.
- The Logs tab can show an active filter state and react to `f`/`d` controls from the app layer.
- The Logs tab presents structured entries in stable columns that are easier to scan by eye.
- A focused E2E test now proves every notification severity lands in the structured log rendering path.
- Any missing behavior is logged against the remaining `16` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
