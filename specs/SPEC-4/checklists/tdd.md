# TDD Checklist: SPEC-4 - GitHub Integration

- [x] `tasks.md` remains the source of truth for execution order.
- [x] Broad regression evidence exists on this branch (`cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`).
- [x] Focused SPEC-4 verification now exists for `gwt-git`, `git_view`, and
  `pr_dashboard`.
- [x] Each remaining unchecked task has a focused failing test or repeatable
  manual check defined.
- [x] The latest implementation slice has spec-focused verification evidence attached to it.
- [x] The reviewer flow in `quickstart.md` has been captured as repeatable completion evidence.
