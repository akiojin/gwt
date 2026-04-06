# Data Model: TUI Theme System (SPEC-11)

## `theme.rs` public API

No runtime entities or data structures. All definitions are compile-time constants.

### `theme::color` — Semantic color constants

```text
const ACTIVE: Color          // Yellow — active/selected items
const FOCUS: Color           // Cyan — focused elements, headings
const SUCCESS: Color         // Green — success states
const ERROR: Color           // Red — error states
const WARNING: Color         // Yellow — warning states
const MUTED: Color           // Gray — inactive, secondary elements
const SURFACE: Color         // DarkGray — background fills
const TEXT_PRIMARY: Color    // White — main content
const TEXT_SECONDARY: Color  // Gray — secondary content
const TEXT_DISABLED: Color   // DarkGray — disabled/placeholder
const BORDER_FOCUSED: Color  // Cyan — focused pane borders
const BORDER_UNFOCUSED: Color // Gray — unfocused pane borders
const BORDER_ERROR: Color    // Red — error overlay borders
const ACCENT: Color          // Magenta — metadata/alt highlights
const AGENT: Color           // Blue — agent-specific
```

### `theme::border` — Border configuration helpers

```text
fn default() -> BorderType     // Rounded
fn focused() -> BorderType     // Thick
fn modal() -> BorderType       // Double
```

### `theme::icon` — Unicode icon constants

```text
const WORKTREE_ACTIVE: &str    // "◆"
const WORKTREE_INACTIVE: &str  // "◇"
const HEAD_INDICATOR: &str     // "▸"
const SESSION_SHELL: &str      // "›"
const SESSION_AGENT: &str      // "◈"
const LAYOUT_TAB: &str         // "▣"
const LAYOUT_GRID: &str        // "▦"
const SUCCESS: &str            // "✓"
const FAILED: &str             // "✗"
const IN_PROGRESS: &str        // "◐"
const ARROW_RIGHT: &str        // "▶"
const CIRCLE_EMPTY: &str       // "○"
```

### `theme::style` — Pre-composed Style helpers

```text
fn active_item() -> Style      // fg=ACTIVE + BOLD
fn selected_item() -> Style    // fg=TEXT_PRIMARY + bg=SURFACE + BOLD
fn header() -> Style           // fg=FOCUS + BOLD
fn muted_text() -> Style       // fg=TEXT_DISABLED
fn error_text() -> Style       // fg=ERROR + BOLD
fn tab_active() -> Style       // fg=ACTIVE + BOLD
fn tab_inactive() -> Style     // fg=MUTED
fn tab_separator() -> Style    // fg=SURFACE (DarkGray)
fn notification_style(severity) -> Style  // severity-based coloring
```

## Lifecycle

All values are `const` or `const fn`. No initialization, no mutation, no cleanup.
