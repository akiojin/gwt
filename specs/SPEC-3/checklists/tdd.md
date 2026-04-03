# TDD Checklist: SPEC-3 - Agent Management

- [x] `tasks.md` remains the source of truth for execution order.
- [x] Broad regression evidence exists on this branch (`cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`).
- [x] Focused `gwt-tui` tests and clippy runs exist for recent SPEC-3 work,
  including session-conversion and version-selection coverage.
- [x] The latest wizard UX restoration slice was driven by failing
  branch-first and spec-prefill startup tests before implementation.
- [x] Each remaining unchecked task has been closed through artifact reconciliation and recorded verification evidence.
- [x] The latest implementation slice has spec-focused verification evidence attached to it.
- [x] The reviewer flow in `quickstart.md` has been captured as repeatable
  completion evidence, including launch-config and launch-materialization
  commands.
