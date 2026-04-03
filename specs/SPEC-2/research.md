# Research: SPEC-2 - Workspace Shell

## Scope Snapshot
- Canonical scope: Tabs, split grid layout, management panel routing, and keyboard-driven workspace navigation.
- Current status: `in-progress` / `Implementation`.
- Task progress: `81/81` checked in `tasks.md`.
- Notes: Most shell behavior exists, but the supporting docs had drifted from the current management-shell shape and have been refreshed.

## Decisions
- Keep the workspace shell centered on tab management and split layout orchestration, not feature-specific screen ownership.
- Document the current management-shell inventory instead of preserving stale references to removed or orphaned screens.
- Use `gwt-git` read paths from `load_initial_data()` for Git View so the screen reuses the same best-effort repository hydration path as other tabs.
- Use the keybinding registry itself as the single source for help-overlay content so Ctrl+G shortcuts do not require separate manual sync.
- Persist the shell snapshot through a TOML artifact under `~/.gwt/sessions/` and restore it before initial data hydration.

## Open Questions
- `T077` was retired after confirming that the referenced `simplify` skill is not exposed in the current session or repository skill set.
