# TDD Checklist: SPEC-3 - Agent Management

- [x] `tasks.md` remains the source of truth for execution order.
- [x] Broad regression evidence exists on this branch (`cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`).
- [x] Focused `gwt-tui` tests and clippy runs exist for recent SPEC-3 work,
  including session-conversion and version-selection coverage.
- [x] The latest wizard UX restoration slice was driven by failing
  branch-first and spec-prefill startup tests before implementation.
- [x] The reopened Quick Start restoration slice has RED tests in place before implementation changes land.
- [x] The reopened AgentSelect / popup-chrome slice has RED tests in place before implementation changes land.
- [x] The latest implementation slice has spec-focused verification evidence attached to it.
- [x] The Codex model snapshot sync slice was driven by failing wizard tests
  before the Launch Agent model list was updated.
- [x] The new-branch worktree materialization slice was driven by a failing
  pending-launch test before launch-time worktree creation was implemented.
- [x] The reviewer flow in `quickstart.md` has been captured as repeatable
  completion evidence, including launch-config and launch-materialization
  commands.
