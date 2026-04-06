# Tasks: TUI Theme System (SPEC-11)

## Phase 1: Setup

- [ ] T-001: Register theme module in `crates/gwt-tui/src/lib.rs` — add `pub mod theme;`

## Phase 2: Foundational — Theme Module (US1, US2, US3)

- [ ] T-002: Create `crates/gwt-tui/src/theme.rs` — define `color` constants (FR-002): ACTIVE, FOCUS, SUCCESS, ERROR, WARNING, MUTED, SURFACE, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_DISABLED, BORDER_FOCUSED, BORDER_UNFOCUSED, BORDER_ERROR, ACCENT, AGENT
- [ ] T-003: Add `border` helpers to `theme.rs` (FR-003): `default() -> Rounded`, `focused() -> Thick`, `modal() -> Double`
- [ ] T-004: Add `icon` constants to `theme.rs` (FR-004): WORKTREE_ACTIVE, WORKTREE_INACTIVE, HEAD_INDICATOR, SESSION_SHELL, SESSION_AGENT, LAYOUT_TAB, LAYOUT_GRID, SUCCESS, FAILED, IN_PROGRESS, ARROW_RIGHT, CIRCLE_EMPTY
- [ ] T-005: Add `style` helpers to `theme.rs` (FR-005): `active_item()`, `selected_item()`, `header()`, `muted_text()`, `error_text()`, `tab_active()`, `tab_inactive()`, `tab_separator()`, `notification_style()`
- [ ] T-006: Add unit tests for `theme.rs` — verify all const values are expected Color/BorderType, verify style helpers return correct fg/bg/modifier combinations
- [ ] T-007: Verify `cargo build -p gwt-tui` and `cargo test -p gwt-tui` pass with new module

## Phase 3: Core Infrastructure Migration (US2)

- [ ] T-008: Migrate `crates/gwt-tui/src/screens/mod.rs` (FR-006) — replace `Color::*` in `list_item_style()`, `build_tab_title()`, `render_empty_list()` with `theme::` refs
- [ ] T-009: Migrate `crates/gwt-tui/src/app.rs` (FR-006, FR-007) — replace `Color::*` in `pane_block()`, `management_tab_title()`, `render_keybind_hints()`, and management rendering; apply `theme::border::default()`/`focused()` to `pane_block()`
- [ ] T-010: Migrate `crates/gwt-tui/src/widgets/tab_bar.rs` (FR-006, FR-007) — replace Color refs and Borders with theme refs
- [ ] T-011: Migrate `crates/gwt-tui/src/widgets/status_bar.rs` (FR-006, FR-008) — replace Color refs and icon literals (`▣`, `▦`) with theme refs
- [ ] T-012: Migrate `crates/gwt-tui/src/widgets/markdown.rs` (FR-006) — replace heading color refs with theme refs
- [ ] T-013: Migrate `crates/gwt-tui/src/model.rs` (FR-008) — replace `SessionTabType::icon()` literals with `theme::icon::SESSION_SHELL` / `SESSION_AGENT`
- [ ] T-014: Verify `cargo test -p gwt-tui` passes after core migration

## Phase 4a: Screen Migration — High-Change (US1, US2) [P]

- [ ] T-015 [P]: Migrate `crates/gwt-tui/src/screens/wizard.rs` (FR-006, FR-007) — ~37 Color refs, 1 Borders site
- [ ] T-016 [P]: Migrate `crates/gwt-tui/src/screens/branches.rs` (FR-006, FR-008) — ~34 Color refs, 2 icon literals (`●`→`◆`, `○`→`◇`, `*`→`▸`)
- [ ] T-017 [P]: Migrate `crates/gwt-tui/src/screens/pr_dashboard.rs` (FR-006) — ~24 Color refs
- [ ] T-018 [P]: Migrate `crates/gwt-tui/src/screens/specs.rs` (FR-006) — ~18 Color refs

## Phase 4b: Screen Migration — Medium-Change (US1, US2) [P]

- [ ] T-019 [P]: Migrate `crates/gwt-tui/src/screens/initialization.rs` (FR-006, FR-007) — ~17 Color refs, 3 Borders sites
- [ ] T-020 [P]: Migrate `crates/gwt-tui/src/screens/git_view.rs` (FR-006) — ~17 Color refs
- [ ] T-021 [P]: Migrate `crates/gwt-tui/src/screens/settings.rs` (FR-006) — ~14 Color refs
- [ ] T-022 [P]: Migrate `crates/gwt-tui/src/screens/logs.rs` (FR-006) — ~14 Color refs

## Phase 4c: Screen Migration — Low-Change (US1, US2) [P]

- [ ] T-023 [P]: Migrate `crates/gwt-tui/src/screens/issues.rs` (FR-006) — ~10 Color refs
- [ ] T-024 [P]: Migrate `crates/gwt-tui/src/screens/profiles.rs` (FR-006, FR-007) — ~11 Color refs, 1 Borders
- [ ] T-025 [P]: Migrate `crates/gwt-tui/src/screens/docker_progress.rs` (FR-006, FR-007, FR-008) — ~9 Color refs, 1 Borders, 2 icons
- [ ] T-026 [P]: Migrate `crates/gwt-tui/src/screens/confirm.rs` (FR-006, FR-007) — ~8 Color refs, 1 Borders
- [ ] T-027 [P]: Migrate `crates/gwt-tui/src/screens/port_select.rs` (FR-006, FR-007) — ~7 Color refs, 1 Borders
- [ ] T-028 [P]: Migrate `crates/gwt-tui/src/screens/versions.rs` (FR-006) — ~6 Color refs
- [ ] T-029 [P]: Migrate `crates/gwt-tui/src/screens/service_select.rs` (FR-006, FR-007, FR-008) — ~5 Color refs, 1 Borders, 1 icon
- [ ] T-030 [P]: Migrate `crates/gwt-tui/src/screens/error.rs` (FR-006, FR-007) — ~5 Color refs, 1 Borders
- [ ] T-031 [P]: Migrate `crates/gwt-tui/src/screens/help.rs` (FR-006, FR-007) — ~5 Color refs, 1 Borders

## Phase 5: Verification and Polish

- [ ] T-032: Run SC-001 grep check — verify zero inline `Color::*` in `screens/` and `widgets/`
- [ ] T-033: Run `cargo test -p gwt-core -p gwt-tui` (SC-002, FR-009)
- [ ] T-034: Run `cargo clippy --all-targets --all-features -- -D warnings` (SC-003)
- [ ] T-035: Verify `pub mod theme;` in lib.rs (SC-004)
- [ ] T-036: Run `cargo fmt` to ensure formatting consistency

## Traceability Matrix

| User Story | Tasks |
|---|---|
| US1 — Consistent visual identity | T-002..T-005 (define theme), T-009 (borders), T-015..T-031 (screen migration) |
| US2 — Maintainable styling | T-001..T-007 (theme module), T-008..T-031 (eliminate inline refs) |
| US3 — Semantic color clarity | T-002 (semantic names), T-005 (style helpers), T-006 (tests) |

| FR | Tasks |
|---|---|
| FR-001 | T-001, T-002 |
| FR-002 | T-002 |
| FR-003 | T-003 |
| FR-004 | T-004 |
| FR-005 | T-005 |
| FR-006 | T-008..T-031 |
| FR-007 | T-009, T-010, T-019, T-024..T-031 |
| FR-008 | T-011, T-013, T-016, T-025, T-029 |
| FR-009 | T-014, T-033 |
