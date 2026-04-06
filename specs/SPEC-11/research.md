# Research: TUI Theme System (SPEC-11)

## ratatui BorderType availability

ratatui 0.29 provides these `BorderType` variants:

- `Plain` — single line (`┌─┐│└┘`)
- `Rounded` — rounded corners (`╭─╮│╰╯`)
- `Double` — double line (`╔═╗║╚╝`)
- `Thick` — thick line (`┏━┓┃┗┛`)
- `QuadrantInside` / `QuadrantOutside` — block quadrant borders

**Decision:** Use `Rounded` as default, `Thick` for focused panes, `Double` for modals.

## Color mapping: current to semantic

| Current usage | Semantic name | ANSI Color | Context |
|---|---|---|---|
| `Color::Yellow` + BOLD | `ACTIVE` | Yellow | Active tab, selected item, input text |
| `Color::Cyan` | `FOCUS` | Cyan | Focused border, headings, interactive elements |
| `Color::Green` | `SUCCESS` | Green | Success state, HEAD indicator, open issues |
| `Color::Red` | `ERROR` | Red | Error state, closed issues, danger actions |
| `Color::Yellow` (no BOLD) | `WARNING` | Yellow | Warning notifications, in-progress state |
| `Color::Gray` | `MUTED` | Gray | Inactive tabs, unfocused borders |
| `Color::DarkGray` | `SURFACE` | DarkGray | Background fills, subtle text, status bar bg |
| `Color::White` | `TEXT_PRIMARY` | White | Primary content text |
| `Color::DarkGray` (text) | `TEXT_DISABLED` | DarkGray | Disabled/placeholder text |
| `Color::Cyan` (border) | `BORDER_FOCUSED` | Cyan | Focused pane border |
| `Color::Gray` (border) | `BORDER_UNFOCUSED` | Gray | Unfocused pane border |
| `Color::Red` (border) | `BORDER_ERROR` | Red | Error overlay border |
| `Color::Magenta` | `ACCENT` | Magenta | Metadata, alternative highlights |
| `Color::Blue` | `AGENT` | Blue | Agent-specific coloring |

## Icon mapping: current to new

| Current | New | Constant name | Context |
|---|---|---|---|
| `●` (U+25CF) | `◆` (U+25C6) | `WORKTREE_ACTIVE` | Branch has worktree |
| `○` (U+25CB) | `◇` (U+25C7) | `WORKTREE_INACTIVE` | Branch without worktree |
| `*` | `▸` (U+25B8) | `HEAD_INDICATOR` | HEAD branch marker |
| `▶` (U+25B6) | `›` (U+203A) | `SESSION_SHELL` | Shell session icon |
| `⭐` (U+2B50) | `◈` (U+25C8) | `SESSION_AGENT` | Agent session icon |
| `▣` (U+25A3) | `▣` (U+25A3) | `LAYOUT_TAB` | Tab layout indicator (keep) |
| `▦` (U+25A6) | `▦` (U+25A6) | `LAYOUT_GRID` | Grid layout indicator (keep) |

## Test impact analysis

Reviewed all `#[cfg(test)]` blocks in affected files. Test assertions check for:

- Text content like `"Branches"`, `"Shell"`, `"Mgmt"`, `"Help"` — unaffected by color changes
- Notification severity strings like `"INFO"`, `"WARN"` — unaffected
- Presence of session names — unaffected

No test assertions check for specific ANSI color codes. Icon character changes
(e.g., `●` to `◆`) are not asserted in any test.

**Conclusion:** All existing tests should pass without modification after migration.
