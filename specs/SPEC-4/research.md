# Research: SPEC-4 - GitHub Integration

## Context
- The TUI already has `git_view.rs`, `pr_dashboard.rs`, and `issues.rs` screens, but the data plumbing is still partial.
- GitHub-backed views depend on `gh` availability and stable parsing of PR and check status output.
- Git View and PR Dashboard should stay separate tabs because they answer different navigation questions.
- Branch linkage, issue detail launch flows, and merge-readiness reporting still need a single canonical data path.
