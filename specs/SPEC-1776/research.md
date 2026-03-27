# Research: SPEC-1776 — TUI Migration

## VT100 to ratatui Rendering

### Proven Pattern (v6.x)

The pre-v7.0.0 TUI used `vt100` crate + `ratatui` with a renderer that converts vt100::Screen cells to ratatui Buffer cells. Key mappings:

- `vt100::Color` → `ratatui::style::Color`
  - Named colors map 1:1 (Red→Red, etc.)
  - Indexed (0-255) → `Color::Indexed(n)`
  - RGB → `Color::Rgb(r,g,b)`
- Cell attributes: bold, italic, underline, inverse map directly to `ratatui::style::Modifier`
- Cursor position from vt100::Screen → set cursor in ratatui Frame

### Performance Consideration

- vt100 crate processes raw bytes efficiently
- Rendering should diff previous frame (ratatui handles this internally via double-buffering)
- Target: <16ms per frame at 120x40 terminal (trivially achievable)

## Ctrl+G Prefix Key Design

### State Machine

```
Idle → [Ctrl+G pressed] → PrefixActive
PrefixActive → [n/s/1-9/v/h/x/q/[/]/PgUp] → Execute action → Idle
PrefixActive → [Ctrl+G again] → Toggle management panel → Idle
PrefixActive → [Escape or timeout 2s] → Cancel → Idle
PrefixActive → [any other key] → Ignore → Idle
```

### Why Ctrl+G

- Not used by most terminal programs (Ctrl+A=tmux, Ctrl+B=tmux alt, Ctrl+G=bell in some)
- Bell character (0x07) is rare in modern terminal workflows
- Established in gwt v6.x TUI

## Split Layout Data Structure

Binary tree where leaves are pane IDs and internal nodes are split directions:

```rust
enum LayoutNode {
    Leaf(String),  // pane_id
    Split {
        direction: Direction,  // Horizontal | Vertical
        ratio: f64,            // 0.0..1.0, position of split
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}
```

## Business Logic to Extract from gwt-tauri

| Source (gwt-tauri) | Target (gwt-core) | Description |
|-|-|-|
| commands/terminal.rs launch_agent() | agent/launch.rs | Agent launch parameter construction |
| commands/terminal.rs session watcher | agent/session_watcher.rs | Session completion monitoring |
| state.rs PR cache polling | git/pr_status.rs | PR/CI status polling |
| state.rs AI summary trigger | ai/summary_trigger.rs | Periodic summary generation |
| commands/voice.rs | voice/runtime.rs | Voice input runtime management |
