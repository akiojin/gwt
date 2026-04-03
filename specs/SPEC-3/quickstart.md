# Quickstart: SPEC-3 - Agent Management

## Minimum Validation Flow
1. **Verify cache-backed startup** - `cargo test -p gwt-tui app::tests::prepare_wizard_startup_prefills_spec_context_and_versions -- --nocapture`
2. **Open the wizard locally** - `cargo run -p gwt-tui` and trigger `Ctrl+G,n` to inspect cached version labels.
3. **Exercise session conversion** - Open an agent session, trigger `Ctrl+G,a`, choose a target agent, and inspect the confirm flow.
4. **Reconcile with acceptance** - Before closure, confirm process-level PTY replacement matches the SPEC rather than only metadata updates.
