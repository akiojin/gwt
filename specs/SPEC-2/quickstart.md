# Quickstart: SPEC-2 — Workspace Shell

## Minimum Validation Flow

### 1. Launch gwt-tui

```bash
cargo run -p gwt-tui
```

Verify: Management layer opens with Branches tab active, branch list in top half, detail in bottom half.

### 2. Navigate Branches

- Press `j` / `k` to move cursor — detail panel updates to show selected branch info
- Press `s` to cycle sort mode (Default → Name → Date)
- Press `v` to cycle view mode (All → Local → Remote)
- Press `/` to start search, type query, press `Esc` to clear

### 3. Branch Detail Sections

- Press `Tab` to cycle detail sections: Overview → SPECs → Git Status → Sessions → Actions
- In Actions section: press `Enter` on "Launch Agent" to open agent select, "Open Shell" to start shell

### 4. Session Management

- Press `Ctrl+G, c` to create a new shell session
- Press `Ctrl+G, g` to toggle Main layer (terminal sessions)
- Press `Ctrl+G, z` to toggle Tab/Grid layout
- Press `Ctrl+G, ]` / `[` to switch sessions
- Press `Ctrl+G, g` again to return to Management

### 5. Tab Navigation

- Press `Left` / `Right` arrow to switch management tabs
- Verify 7 tabs: Branches, Issues, Profiles, Git View, Versions, Settings, Logs

### 6. Quit

- Press `Ctrl+C` twice (within 500ms) to quit
- Or press `Ctrl+G, q`

### Running Tests

```bash
# Unit + E2E tests
cargo test -p gwt-tui

# E2E snapshot tests only
cargo test -p gwt-tui --test snapshot_e2e

# Update snapshots after UI changes
cargo insta test --accept -p gwt-tui --test snapshot_e2e
```
