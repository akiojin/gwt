# Quickstart: SPEC-10 - Project Workspace

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and enter the project initialization flow.
2. Exercise clone, existing repository selection, and migration paths with representative repositories.
3. Verify repository-kind detection and branch-protection behavior in the resulting workspace.
4. Treat the final coverage and manual-review tasks as the remaining work before closure.

## Expected Result
- The reviewer sees the current implemented scope for project workspace.
- Any missing behavior is logged against the remaining `2` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
