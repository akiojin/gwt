# Workspace Shell -- Implementation Plan

## Summary

Complete the partially implemented workspace shell features: help overlay auto-collection from code, session persistence improvement, and Git View tab addition. The core session management, keybind state machine, and management panel are already functional.

## Technical Context

- **Architecture**: Elm Architecture -- Model (`model.rs`), Message (`message.rs`), Update (`app.rs`), View (`view.rs`)
- **Keybind state machine**: `crates/gwt-tui/src/keybind.rs` -- Ctrl+G prefix with 2-second timeout
- **Session management**: `crates/gwt-tui/src/model.rs` -- session list, active index, display mode
- **Management panel**: `crates/gwt-tui/src/screens/` -- individual tab implementations
- **Session persistence**: `~/.gwt/sessions/` -- TOML files for session metadata

## Constitution Check

- Spec before implementation: yes, this SPEC documents all workspace shell requirements.
- Test-first: help auto-collection and session persistence tests must be RED before implementation.
- No workaround-first: help overlay uses code introspection, not a manually maintained list.
- Minimal complexity: each phase is independent and can be implemented/verified separately.

## Complexity Tracking

- Added complexity: keybind auto-collection macro/attribute, Git View tab
- Mitigation: auto-collection uses existing keybind definitions, no new data flow

## Phased Implementation

### Phase 1: Help Overlay Auto-Collection

1. Define a keybinding registry structure that maps key sequences to description strings.
2. Populate the registry from the existing keybind match arms in `keybind.rs`.
3. Render the help overlay (Ctrl+G,?) using the registry data.
4. Add tests: verify all bound keys appear in help output, verify unbound keys are absent.

### Phase 2: Session Persistence Improvement

1. Audit current session save/restore logic for completeness.
2. Add missing fields: display mode (tab/split), management panel visibility, active management tab.
3. Add graceful fallback for corrupted or incompatible persistence files.
4. Add tests: save/restore round-trip, corrupted file handling, missing directory recovery.

### Phase 3: Git View Tab

1. Implement Git View management tab showing recent git log and diff summary.
2. Wire into the management panel tab navigation (tab index 5).
3. Add tests: Git View renders commit list, handles empty repo, handles detached HEAD.

## Dependencies

- `crates/gwt-core/src/git/` -- git operations for Git View tab
- `toml` crate -- session persistence serialization (already in use)
