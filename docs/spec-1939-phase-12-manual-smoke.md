# SPEC-1939 Phase 12 — manual smoke checklist

T-IDX-111 (macOS) and T-IDX-112 (Windows best-effort) require a human
reviewer to drive a real `gwt` GUI build because the fit between
xterm.js / wry / tao / OS window chrome is not exercisable from
Playwright's embedded-frontend fixture pattern.

The Playwright behaviour suite (`crates/gwt/playwright/tests/index-status.
spec.ts`, 13 tests × 2 themes) gates **frontend logic** end-to-end:
state-machine transitions, click → `settings:open`, Settings.Index table,
per-cell / scope-row Rebuild dispatch, project tab dot aggregation, and
the progress toast. Manual smoke just confirms that the same frontend
renders correctly when wired to the real backend.

Use this checklist when verifying a release candidate. Record the
outcome in the Board (`gwtd board post --kind status --body ...`) so
SPEC-1939 Issue #2584 can close.

## Prerequisites

1. macOS (T-IDX-111 hard gate) or Windows (T-IDX-112 best-effort).
2. A local `gwt` build:
   ```bash
   cargo build -p gwt --release
   ```
3. A multi-worktree project with at least 2 active worktrees. The gwt
   repo itself works (this repository's `develop` worktree plus any
   feature worktree). Otherwise, materialise additional worktrees from
   the gwt GUI's *Start Work* flow against an existing project — agent
   CLIs do not create worktrees, so do not invoke `git worktree add`
   from automation.
4. Optional: pre-seed an unhealthy index scope so the bootstrap path
   enters `repair_required` and the auto-rebuild orchestrator runs.
   The chroma stores live under `~/.gwt/index/<repo-hash>/worktrees/
   <wt-hash>/files/`. The 16-char `repo-hash` is computed from the
   project's git remote URL (or local path when no remote is set), and
   each worktree gets its own `wt-hash` directory. The simplest recipe:

   ```bash
   # 1. List the indexed worktrees for the active gwt project (run from
   #    inside the project's repo root):
   ls -d ~/.gwt/index/*/worktrees/*/

   # 2. Pick one of those `<wt-hash>/` directories and remove its `files/`
   #    chroma store to force `manifest_missing`:
   rm -rf ~/.gwt/index/<repo-hash>/worktrees/<wt-hash>/files
   ```

   When you re-launch `gwt`, the bootstrap probe should detect the
   missing chroma store, surface the red `Index: repair` badge for one
   tick, then transition through `Index: repairing` (yellow + spinner)
   to `Index: ready` (green) once the orchestrator rebuilds the scope.

## T-IDX-111 — macOS manual smoke

Open the project and confirm each step in order. Stop and capture a
screenshot if any step fails.

1. **Bootstrap badge transitions.** Launch `gwt` in the multi-worktree
   project. The top-bar badge should briefly show `Index: checking`,
   transition to `Index: repair` (red) when the unhealthy scope is
   detected, then `Index: repairing` (yellow + spinner) once the
   orchestrator starts, and finally `Index: ready` (green).
2. **Project tab dot aggregation.** While the transition runs, the
   project tab's coloured dot must follow the same colour: red →
   yellow → green. Other project tabs must stay green throughout.
3. **Settings.Index tab opens via badge click.** Click the badge during
   any non-`ready` state. The Settings window must open with the
   `Index` tab pre-selected (`data-settings-tab=index`).
4. **Health table renders.** The Index tab must list `(scope, worktree)`
   cells with `last_repair_at` / `document_count` / `reason`. The
   unhealthy worktree's `files` cell must read `manifest_missing` (or
   the reason you seeded).
5. **Per-cell Rebuild → green dot.** Click the per-cell `Rebuild`
   button on the unhealthy worktree's `files` row. The cell must flip
   to ready, the project tab dot must return to green, and a fresh
   `Index: ready` (green) badge must remain stable.
6. **Progress toast on repairing click.** While the badge is
   `Index: repairing`, click it. A toast like `Rebuilding project
   index: X of Y scope(s) completed` must appear in the bottom-right
   for ~3.5 s.
7. **No flicker / focus loss.** Throughout the run, the host window
   must not flicker, and keyboard focus must not jump.

## T-IDX-112 — Windows best-effort smoke

Repeat the macOS checklist on Windows. Pay extra attention to:

- Badge button click does **not** open Settings as a flickering window
  (white frame ≤ 1 frame).
- xterm scrollback continues to scroll while the badge transitions —
  there is a known interaction with terminal viewport reflow
  (SPEC-2008 Phase 24 covers this; mention any regression in the smoke
  Board post).

If a Windows host is unavailable, mark T-IDX-112 as `best-effort
deferred — no Windows host` in the Board, and rely on the
`Test (Rust, Windows)` and `Visual Regression` CI checks as proxy.

## Recording the result

After running the checklist, post a status update:

```bash
gwtd board post --kind status --owner SPEC-1939 --topic phase-12-smoke \
  --mention user:akiojin --body $'\
SPEC-1939 Phase 12 macOS smoke (T-IDX-111) 完了:\n\
- 1: pass\n\
- 2: pass\n\
- 3: pass\n\
- 4: pass\n\
- 5: pass\n\
- 6: pass\n\
- 7: pass\n\
\n\
Windows (T-IDX-112): <pass | best-effort deferred>'
```

Valid `gwtd board post` flags (per the parser at `crates/gwt/src/cli/board.rs`):
`--kind`, `--body | -f`, `--title-summary`, `--parent`, `--topic`, `--owner`,
`--target <session-id|branch|agent-id>`, and `--mention <kind:id>` (e.g.
`user:akiojin`, `agent:codex`). `--target` highlights a post for a specific
agent / branch / session; `--mention` records a typed audience marker that
ships with the entry payload.

When the smoke passes on macOS, comment on Issue #2584 with the Board
link to close the Phase 12 verification follow-up.
