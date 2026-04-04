# TDD Checklist: SPEC-5 - Local SPEC Management

- [x] `tasks.md` remains the source of truth for execution order.
- [x] Broad regression evidence exists on this branch (`cargo test -p gwt-core -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`).
- [x] Broad repo checks pass, and focused verification now exists for management-tab reachability, startup metadata loading, detail navigation, wizard launch prefill, and the restored `analysis.md` detail tab.
- [x] The live-shell section-navigation gap (`Left` / `Right` in Specs detail) now has focused app-level coverage.
- [x] The live-shell metadata/content edit reachability now has focused coverage for `e` (phase selection), `s` (status selection), `Ctrl+e` (selected section edit), and `E` (raw file edit).
- [x] The metadata selection-menu path now has focused coverage for `Up` / `Down` cycling, constrained rendering, and persisted save behavior.
- [x] The `spec.md` section-scoped edit path now has focused coverage for heading selection, duplicate-heading disambiguation, nested-heading-safe replacement, fenced-code filtering, disappeared-section save errors, and section-selection hints.
- [x] Each remaining execution gap has a focused failing test or a repeatable manual check defined in `quickstart.md`.
- [x] The latest implementation slice has spec-focused verification evidence attached to it.
- [x] The reviewer flow in `quickstart.md` has been captured as repeatable completion evidence, and helper-level persistence tests are no longer presented as live-shell evidence.
