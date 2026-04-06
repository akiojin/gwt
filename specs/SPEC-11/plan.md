# Plan: TUI Theme System (SPEC-11)

## Summary

Introduce `crates/gwt-tui/src/theme.rs` as the single source of truth for all UI
styling: semantic colors, border configurations, icon constants, and pre-composed
Style helpers. Replace ~302 inline `Color::*` references, ~15 `Borders::ALL` sites,
and ~9 Unicode icon literals across 22 files with `theme::` references. Visual tone
shifts to Minimalist Modern (rounded borders, muted cyan accent, soft grays).

## Technical Context

### Affected files (22 total)

**New file:**

- `crates/gwt-tui/src/theme.rs` — theme module (FR-001 through FR-005)

**Modified files (production):**

- `crates/gwt-tui/src/lib.rs` — add `pub mod theme;`
- `crates/gwt-tui/src/app.rs` — `pane_block()`, management pane rendering (~30 Color refs)
- `crates/gwt-tui/src/model.rs` — `SessionTabType::icon()` method (2 icon literals)
- `crates/gwt-tui/src/screens/mod.rs` — `list_item_style()`, `build_tab_title()`, `render_empty_list()` (~6 Color refs)
- `crates/gwt-tui/src/screens/branches.rs` — list + detail rendering (~34 Color refs, 2 icon literals)
- `crates/gwt-tui/src/screens/wizard.rs` — agent launch wizard (~37 Color refs, 1 Borders)
- `crates/gwt-tui/src/screens/pr_dashboard.rs` — PR list (~24 Color refs)
- `crates/gwt-tui/src/screens/specs.rs` — SPEC list (~18 Color refs)
- `crates/gwt-tui/src/screens/initialization.rs` — clone wizard (~17 Color refs, 3 Borders)
- `crates/gwt-tui/src/screens/git_view.rs` — git status (~17 Color refs)
- `crates/gwt-tui/src/screens/settings.rs` — settings UI (~14 Color refs)
- `crates/gwt-tui/src/screens/logs.rs` — log viewer (~14 Color refs)
- `crates/gwt-tui/src/screens/profiles.rs` — profile list (~11 Color refs, 1 Borders)
- `crates/gwt-tui/src/screens/issues.rs` — issue list (~10 Color refs)
- `crates/gwt-tui/src/screens/docker_progress.rs` — docker progress (~9 Color refs, 1 Borders, 2 icons)
- `crates/gwt-tui/src/screens/confirm.rs` — confirm dialog (~8 Color refs, 1 Borders)
- `crates/gwt-tui/src/screens/port_select.rs` — port select (~7 Color refs, 1 Borders)
- `crates/gwt-tui/src/screens/versions.rs` — version list (~6 Color refs)
- `crates/gwt-tui/src/screens/service_select.rs` — service select (~5 Color refs, 1 Borders, 1 icon)
- `crates/gwt-tui/src/screens/error.rs` — error overlay (~5 Color refs, 1 Borders)
- `crates/gwt-tui/src/screens/help.rs` — help overlay (~5 Color refs, 1 Borders)
- `crates/gwt-tui/src/widgets/status_bar.rs` — footer bar (~17 Color refs, 2 icon literals)
- `crates/gwt-tui/src/widgets/tab_bar.rs` — session tabs (~3 Color refs, 1 Borders)
- `crates/gwt-tui/src/widgets/markdown.rs` — markdown render (~5 Color refs)

**Excluded files (no change):**

- `crates/gwt-tui/src/renderer.rs` — vt100 PTY color mapping, not UI chrome
- `crates/gwt-tui/src/event.rs` — no styling
- `crates/gwt-tui/src/main.rs` — terminal setup only
- `crates/gwt-core/` — no TUI styling
- Test code within `#[cfg(test)]` modules — Color refs in test assertions may remain

### Assumptions

- ratatui 0.29 supports `BorderType::Rounded` and `BorderType::Thick` (verified: yes)
- ANSI 16 colors are sufficient for the Minimalist Modern palette (no true color needed)
- Test assertions check text content (e.g., `contains("Branches")`), not specific color values

## Constitution Check

| Rule | Status | Notes |
|------|--------|-------|
| 1. Spec Before Implementation | PASS | SPEC-11 spec.md complete |
| 2. Test-First Delivery | PASS | Existing tests serve as regression suite; theme module adds unit tests for const correctness |
| 3. No Workaround-First Changes | PASS | Root cause (inline hardcoding) is explicit; solution is direct centralization |
| 4. Minimal Complexity | PASS | Single flat module with const values — no traits, no generics, no runtime config |
| 5. Verifiable Completion | PASS | SC-001 grep check, SC-002 cargo test, SC-003 clippy |
| 6. SPEC Category | DESIGN | One SPEC, one category |

## Complexity Tracking

| Addition | Reason |
|----------|--------|
| `theme.rs` new module (~150-200 lines) | Eliminates ~302 inline Color refs across 22 files; net complexity reduction |
| No new abstractions | Flat const module — no Theme trait, no builder pattern, no runtime selection |

## Project Structure

```text
crates/gwt-tui/src/
├── theme.rs          ← NEW: semantic colors, borders, icons, styles
├── lib.rs            ← ADD: pub mod theme;
├── app.rs            ← MODIFY: use theme::* in pane_block, render functions
├── model.rs          ← MODIFY: icon() method uses theme::icon::*
├── screens/
│   ├── mod.rs        ← MODIFY: shared helpers use theme::*
│   ├── branches.rs   ← MODIFY: 34 Color refs + 2 icons
│   ├── wizard.rs     ← MODIFY: 37 Color refs
│   ├── pr_dashboard.rs ← MODIFY: 24 Color refs
│   ├── ... (15 more screen files)
│   └── help.rs       ← MODIFY: 5 Color refs
├── widgets/
│   ├── status_bar.rs ← MODIFY: 17 Color refs + 2 icons
│   ├── tab_bar.rs    ← MODIFY: 3 Color refs
│   └── markdown.rs   ← MODIFY: 5 Color refs
└── renderer.rs       ← EXCLUDED (PTY colors)
```

## Phased Implementation

### Phase 1: Theme module foundation (FR-001 through FR-005)

Create `theme.rs` with all semantic definitions. Register in `lib.rs`.

**Files:** `theme.rs` (new), `lib.rs` (1-line add)

**Verification:** `cargo build -p gwt-tui` compiles, `cargo test -p gwt-tui` passes.

### Phase 2: Core infrastructure migration (FR-006, FR-007)

Replace inline values in shared infrastructure: `screens/mod.rs`, `app.rs` (pane_block,
management rendering), `widgets/tab_bar.rs`, `widgets/status_bar.rs`.

These files define patterns that all screens consume, so migrating them first
establishes the convention.

**Files:** `screens/mod.rs`, `app.rs`, `widgets/tab_bar.rs`, `widgets/status_bar.rs`, `widgets/markdown.rs`

**Verification:** `cargo build`, visual smoke test, existing tests pass.

### Phase 3: Screen migration (FR-006, FR-007, FR-008)

Replace inline values in all screen files. This phase has the most files but each
change is mechanical (find-replace with semantic mapping).

**Sub-phases (parallelizable):**

- 3a: High-change screens — `wizard.rs`, `branches.rs`, `pr_dashboard.rs`, `specs.rs`
- 3b: Medium-change screens — `initialization.rs`, `git_view.rs`, `settings.rs`, `logs.rs`
- 3c: Low-change screens — `issues.rs`, `profiles.rs`, `docker_progress.rs`, `confirm.rs`,
  `port_select.rs`, `versions.rs`, `service_select.rs`, `error.rs`, `help.rs`

**Files:** All 17 screen files + `model.rs` (icon migration)

**Verification:** `cargo test -p gwt-tui`, `cargo clippy`, SC-001 grep check.

### Phase 4: Verification and cleanup

Run full verification suite. Fix any remaining inline references. Confirm all
success criteria pass.

**Verification:** SC-001 through SC-006.
