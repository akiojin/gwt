# Quickstart: SPEC-7 - Settings and Profiles

## Reviewer Flow
1. Open the settings surface from the management shell.
2. Navigate profiles, environment settings, and the voice category.
3. Compare the visible voice fields against the intended configuration schema.
4. Track missing fields or validation mismatches as remaining SPEC-7 execution work.

## Repeatable Evidence
- `cargo test -p gwt-tui settings -- --nocapture`
- `cargo test -p gwt-core -p gwt-tui`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Expected Result
- The reviewer sees the current implemented scope for settings and profiles.
- Any missing behavior is logged against the remaining `12` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
