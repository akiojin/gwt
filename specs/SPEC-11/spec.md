# Feature Specification: TUI Theme System

## Background

gwt-tui currently hardcodes all visual styling inline across 15+ source files
(app.rs, screens/*.rs, widgets/*.rs). Colors like `Color::Yellow`, `Color::Cyan`,
and `Color::DarkGray` are repeated dozens of times with no central definition.
Border styles use only `Borders::ALL` with default single-line rendering.
Icons are scattered Unicode literals (`●`, `○`, `*`, `▶`, `⭐`).

This makes it impossible to:

- Change the visual tone without touching every file
- Maintain consistent styling as new screens are added
- Reason about the semantic meaning of a color choice

This SPEC introduces a centralized theme module (`theme.rs`) that defines semantic
colors, border style sets, and icon constants. All existing inline values will be
replaced with `theme::` references. The visual tone shifts to **Minimalist Modern**:
rounded borders, muted cyan accent, soft grays, warm white text.

## User Stories

### User Story 1 - Consistent visual identity (Priority: P1)

As a gwt-tui user, I want the application to have a cohesive, modern visual identity
so that the interface feels polished and professional.

### User Story 2 - Maintainable styling (Priority: P1)

As a gwt-tui developer, I want all visual styling defined in one module so that
I can change colors, borders, or icons without editing every screen file.

### User Story 3 - Semantic color clarity (Priority: P2)

As a gwt-tui developer, I want color choices to express semantic meaning
(e.g., `theme::color::ACTIVE` instead of `Color::Yellow`) so that the intent
of each style is self-documenting.

## Acceptance Scenarios

1. Given gwt-tui is built, when I search for `Color::Yellow` or `Color::Cyan` in
   `screens/` and `widgets/`, then zero direct usages remain (all replaced by `theme::` refs).

2. Given the Branches screen is rendered, when I look at the pane borders, then
   they use rounded corners (`╭╮╰╯`) instead of sharp single-line corners.

3. Given a branch with a worktree is displayed, when I look at the indicator icon,
   then it shows `◆` (not `●`).

4. Given a focused pane, when I compare it to an unfocused pane, then the focused
   pane uses a visually distinct border style (e.g., thick/double) while the
   unfocused pane uses the standard rounded style.

5. Given the session tab bar, when I look at agent session icons, then they show `◈`
   (not `⭐`), and shell sessions show `›` (not `▶`).

6. Given `theme.rs` exists, when I inspect its public API, then it exposes:
   - `color` module with semantic constants (ACTIVE, FOCUS, SUCCESS, ERROR, MUTED, etc.)
   - `border` module with border set constructors (default, focused, modal)
   - `icon` module with all UI icon constants
   - `style` module with pre-composed Style helpers (active item, selected item, header, etc.)

7. Given all tests pass (`cargo test -p gwt-tui`), when I run `cargo clippy`, then
   no new warnings are introduced.

## Edge Cases

- Terminals with limited color support (< 256 colors): theme must use ANSI 16 base colors
  as primary palette to ensure universal compatibility.
- Very narrow terminals (< 40 cols): rounded borders and icon characters must not
  break layout calculations.
- Existing vt100 renderer color mapping (renderer.rs): must NOT be touched by this SPEC
  because it maps PTY output colors, not UI chrome colors.

## Functional Requirements

- FR-001: Create `crates/gwt-tui/src/theme.rs` module with sub-modules for color,
  border, icon, and style definitions.
- FR-002: Define semantic color constants using `ratatui::style::Color` (ANSI 16 base).
  Minimum set: ACTIVE, FOCUS, SUCCESS, ERROR, WARNING, MUTED, SURFACE, TEXT_PRIMARY,
  TEXT_SECONDARY, TEXT_DISABLED, BORDER_FOCUSED, BORDER_UNFOCUSED.
- FR-003: Define border set helpers that return `ratatui::widgets::BorderType` or
  `block::BorderType` for: default (Rounded), focused (Thick or Double), modal (Double).
- FR-004: Define icon constants as `&str`: WORKTREE_ACTIVE (`◆`), WORKTREE_INACTIVE (`◇`),
  HEAD_INDICATOR (`▸`), SESSION_SHELL (`›`), SESSION_AGENT (`◈`), LAYOUT_TAB (`▣`),
  LAYOUT_GRID (`▦`), and status icons (SUCCESS `✓`, FAILED `✗`, IN_PROGRESS `◐`).
- FR-005: Define pre-composed `Style` helpers: `active_item()`, `selected_item()`,
  `header()`, `muted_text()`, `error_text()`, `tab_active()`, `tab_inactive()`,
  `tab_separator()`.
- FR-006: Replace all inline `Color::*`, `Modifier::*` combinations in screens/*.rs
  and widgets/*.rs with `theme::` references.
- FR-007: Replace all inline border configurations (`Borders::ALL` + default `BorderType`)
  with `theme::border::*` helpers in `pane_block()` and overlay renderers.
- FR-008: Replace all inline Unicode icon literals with `theme::icon::*` constants.
- FR-009: Existing tests must continue to pass without modification (visual output may
  change but assertions on text content must hold).

## Non-Functional Requirements

- NFR-001: `theme.rs` module must have zero external dependencies beyond `ratatui`.
- NFR-002: All theme constants must be `const` or `const fn` — no runtime allocation.
- NFR-003: Total added lines in `theme.rs` should be under 200 lines.
- NFR-004: No file outside `crates/gwt-tui/src/` should be modified.

## Success Criteria

- SC-001: `grep -rn 'Color::Yellow\|Color::Cyan\|Color::Green\|Color::Red\|Color::Gray\|Color::DarkGray' crates/gwt-tui/src/screens/ crates/gwt-tui/src/widgets/` returns zero matches.
- SC-002: `cargo test -p gwt-tui` passes.
- SC-003: `cargo clippy --all-targets --all-features -- -D warnings` passes.
- SC-004: `theme.rs` exists and is `pub mod theme;` in `lib.rs`.
- SC-005: All pane borders render with `BorderType::Rounded`.
- SC-006: Focused panes use a visually distinct border (Thick or Double).
