# TDD Checklist: SPEC-5 - Local SPEC Management

- [x] `tasks.md` remains the source of truth for execution order.
- [x] Broad regression evidence exists on this branch (`cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`).
- [x] Broad repo checks pass, and focused verification now exists for management-tab reachability, startup metadata loading, detail navigation, wizard launch prefill, and the restored `analysis.md` detail tab.
- [x] The live-shell section-navigation gap (`Left` / `Right` in Specs detail) now has focused app-level coverage.
- [x] The live-shell metadata/content edit reachability now has focused coverage for `e` (phase), `s` (status), and `Ctrl+e` (raw file edit).
- [x] Each remaining execution gap has a focused failing test or a repeatable manual check defined in `quickstart.md`.
- [x] The latest implementation slice has spec-focused verification evidence attached to it.
- [x] The reviewer flow in `quickstart.md` has been captured as repeatable completion evidence, and helper-level persistence tests are no longer presented as live-shell evidence.
