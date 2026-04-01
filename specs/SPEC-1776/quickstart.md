# Quickstart: SPEC-1776 — TUI Migration

## Minimum Validation Flow

### Phase 0 (Scaffold)

```bash
# Build the new crate
cargo build -p gwt-tui

# Run it — should show blank ratatui screen, quit with q
cargo run -p gwt-tui
```

### Phase 1 (Minimal TUI)

```bash
# Run gwt-tui — should open with a shell tab
cargo run -p gwt-tui

# Inside TUI:
# - Type shell commands, verify output renders with colors
# - Paste multi-line text, verify the full payload reaches the PTY
# - Ctrl+G, c → opens new shell tab
# - Ctrl+G, ] → switch to next tab
# - Ctrl+G, [ → switch to previous tab
# - In normal mode: wheel / trackpad / PgUp / PgDn / Home scroll transcript history
# - In normal mode: drag to copy selection, release to copy
# - End or scrolling back to bottom returns to live follow
# - Ctrl+G, x → close tab
# - Ctrl+C twice → quit
```

### Phase 2 (Agent + Management)

```bash
cargo run -p gwt-tui

# Inside TUI:
# - Select a branch in Branches and press Enter → agent launch dialog
# - Select Claude Code, choose options → agent starts in new tab
# - Ctrl+G → management panel visible
# - Arrow keys to navigate agents
# - k → kill agent
# - Enter → switch to agent tab
# - Ctrl+G → dismiss panel
```

### Phase 3 (Split Panes)

```bash
# Inside TUI with 2+ tabs:
# - Ctrl+G, v → vertical split (side by side)
# - Ctrl+G, h → horizontal split (top/bottom)
# - Both panes render independently
# - Resize terminal window → panes adjust
```

### Running Tests

```bash
# Unit tests
cargo test -p gwt-tui

# Ensure gwt-core tests still pass
cargo test -p gwt-core

# Lint
cargo clippy -p gwt-tui --all-targets --all-features -- -D warnings
```
