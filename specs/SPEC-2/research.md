# Research: SPEC-2 — Workspace Shell

## Elm Architecture in gwt-tui

The TUI follows the Elm Architecture pattern:
- **Model** (`model.rs`): Central state, ~220 LOC
- **Message** (`message.rs`): All action types, ~30 variants
- **Update** (`app.rs::update()`): State transitions, ~200 LOC
- **View** (`app.rs::view()`): Rendering, ~300 LOC

## Ctrl+G Prefix System

Chosen over tmux-style (Ctrl+B) because:
- Ctrl+G (bell character 0x07) is rarely used in modern terminal workflows
- Established in gwt v6.30.3 TUI
- 2-second timeout prevents accidental prefix lock

Implementation: State machine in `keybind.rs` with `Idle → Active → Action/Cancel`.

## ratatui ListState for Scrolling

Previous implementation used `render_widget` without scroll state. Fixed to use `render_stateful_widget` + `ListState` which provides:
- Automatic scroll to keep selected item visible
- `.highlight_style()` for visual selection indicator
- No manual scroll offset calculation needed

## Branch Detail — Split Layout Decision

Evaluated three options:
1. Tab area replacement (Enter/Esc toggle) — rejected: loses context
2. Left/right split — rejected: too narrow for details
3. **Top/bottom split (50/50)** — chosen: branch list always visible, detail updates on cursor move

## Session Persistence

Uses TOML format at `~/.gwt/sessions/{base64_path}.toml`. Base64-encodes worktree path for filename safety. Best-effort restore: errors logged but don't prevent startup.

## SPECs Tab Removal

SPECs moved from independent management tab to Branch Detail view because:
- SPECs are branch-specific (different branches have different specs/)
- Reduces tab count (8→7)
- Natural integration with branch workflow
