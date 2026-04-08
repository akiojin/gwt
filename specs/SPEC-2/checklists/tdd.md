# TDD Checklist: SPEC-2 - Workspace Shell

- [x] `tasks.md` remains the source of truth for execution order.
- [x] Broad regression evidence exists on this branch (`cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt -- --check`, `cargo build -p gwt-tui`).
- [x] Broad tests pass, and the remaining shell task was retired as obsolete after an explicit availability check.
- [x] Each remaining unchecked task has a focused failing test or repeatable manual check defined.
- [x] The latest implementation slice has spec-focused verification evidence attached to it (`cargo test -p gwt-tui should_render_after_tick -- --nocapture`, `cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt -- --check`, `cargo build -p gwt-tui`).
- [x] The reviewer flow in `quickstart.md` has been captured as repeatable completion evidence.
