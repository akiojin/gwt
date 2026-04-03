# Quickstart: SPEC-9 - Infrastructure

## Reviewer Flow
1. Run the current infrastructure flows that are reachable from the product shell or CLI.
2. Verify hooks merge behavior against backup and restore expectations first.
3. Inspect Docker-related UI screens and confirm the background lifecycle worker drives DockerProgress transitions.
4. Confirm builtin embedded skills are present at startup and can be toggled from Settings.
5. Run the focused ServiceSelect, PortSelect, DockerProgress, and container lifecycle checks before treating the Docker UI slice as advanced.
6. Track the remaining release packaging gaps as open execution work.

## Repeatable Evidence
- `cargo test -p gwt-skills -- --nocapture`
- `cargo test -p gwt-tui model_new_defaults -- --nocapture`
- `cargo test -p gwt-tui docker_progress -- --nocapture`
- `cargo test -p gwt-tui update_branches_docker_stop_executes_and_refreshes_detail -- --nocapture`
- `cargo test -p gwt-tui update_branches_docker_restart_failure_routes_error_notification -- --nocapture`
- `cargo test -p gwt-tui settings -- --nocapture`
- `cargo test -p gwt-tui service_select -- --nocapture`
- `cargo test -p gwt-tui port_select -- --nocapture`
- `cargo test -p gwt-git -- --nocapture`
- `cargo test -p gwt-skills set_enabled -- --nocapture`
- `cargo test -p gwt-docker --lib -- --nocapture`

## Expected Result
- The reviewer sees the current implemented scope for infrastructure.
- Any remaining gaps are reviewer-flow or end-to-end acceptance items, not unchecked implementation tasks.
- No step should be treated as complete unless the code path is actually reachable today.
