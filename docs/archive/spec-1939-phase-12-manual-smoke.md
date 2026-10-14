# SPEC-1939 Phase 12 / 13 — manual smoke checklist

T-IDX-111 (macOS) and T-IDX-112 (Windows best-effort) require a human
reviewer to drive a real `gwt` GUI build because the fit between
xterm.js / wry / tao / OS window chrome is not exercisable from
Playwright's embedded-frontend fixture pattern.

The Playwright behaviour suite (`crates/gwt/playwright/tests/index-status.spec.ts`)
gates **frontend logic** end-to-end after Phase 13: per-tab dot aggregation,
Settings.Index table rendering, and per-cell / scope-row Rebuild dispatch
through the `settings:open` event. Manual smoke just confirms that the
same frontend renders correctly when wired to the real backend.

> **Phase 13 scope change.** The project-bar `Index: ready / repair / …`
> badge has been withdrawn (concept separation: `issues` / `specs` are
> repo-shared while `files` / `files-docs` are per-worktree, so a single
> aggregated badge mixed scopes and produced cross-branch contamination).
> All steps that previously inspected the badge or its progress toast are
> removed. Use the per-tab dot for Files/Files-docs health and the
> Settings → Index tab for full per-scope details.

Use this checklist when verifying a release candidate. Record the
outcome in the Board with JSON operation `board.post` so
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
   <wt-hash>/files/`. The simplest recipe:

   ```bash
   # 1. List the indexed worktrees for the active gwt project:
   ls -d ~/.gwt/index/*/worktrees/*/

   # 2. Pick one of those `<wt-hash>/` directories and remove its `files/`
   #    chroma store to force `manifest_missing`:
   rm -rf ~/.gwt/index/<repo-hash>/worktrees/<wt-hash>/files
   ```

   When you re-launch `gwt`, the per-tab dot for the affected worktree
   should turn red while the orchestrator rebuilds, transition through
   yellow (repairing), and end on green (ready) once the rebuild
   completes. The project-bar surface stays unchanged because the badge
   no longer exists.

## T-IDX-111 — macOS manual smoke

Open the project and confirm each step in order. Stop and capture a
screenshot if any step fails.

1. **Project tab dot aggregation.** Launch `gwt` in the multi-worktree
   project. While the orchestrator runs, the affected project tab's
   coloured dot must follow red → yellow → green. Other project tabs
   must stay green throughout. The project-bar surface (Workspace /
   Open Project / theme toggle) must NOT show any `Index:` badge.
2. **Settings → Index tab opens.** Open Settings (existing entry point
   such as the Settings menu / button or any keybinding wired to it).
   Switch to the `Index` tab.
3. **Health table renders.** The Index tab must list `(scope, worktree)`
   cells with `last_repair_at` / `document_count` / `reason`. The
   unhealthy worktree's `files` cell must read `manifest_missing` (or
   the reason you seeded). Repo-shared scopes (`issues`, `specs`) must
   appear without per-worktree columns.
4. **Per-cell Rebuild → green dot.** Click the per-cell `Rebuild`
   button on the unhealthy worktree's `files` row. The cell must flip
   to ready and the project tab dot must return to green.
5. **No flicker / focus loss.** Throughout the run, the host window
   must not flicker, and keyboard focus must not jump.

## T-IDX-112 — Windows best-effort smoke

Repeat the macOS checklist on Windows. Pay extra attention to:

- Settings window mount does **not** flash a white frame > 1 frame
  when opened from the Index entry path.
- xterm scrollback continues to scroll while the orchestrator runs —
  there is a known interaction with terminal viewport reflow
  (SPEC-2008 Phase 24 covers this; mention any regression in the smoke
  Board post).

If a Windows host is unavailable, mark T-IDX-112 as `best-effort
deferred — no Windows host` in the Board, and rely on the
`Test (Rust, Windows)` and `Visual Regression` CI checks as proxy.

## Recording the result

After running the checklist, post a status update:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"board.post","params":{"kind":"status","owners":["SPEC-1939"],"topics":["phase-12-smoke"],"mentions":["user:akiojin"],"body":"SPEC-1939 Phase 12/13 macOS smoke (T-IDX-111) 完了:\n- 1 (tab dot aggregation): pass\n- 2 (Settings.Index opens): pass\n- 3 (health table): pass\n- 4 (per-cell Rebuild → green dot): pass\n- 5 (no flicker / focus loss): pass\n\nWindows (T-IDX-112): <pass | best-effort deferred>"}}
JSON
```

Valid `board.post` JSON params include `kind`, `body`, `parent`, `topics`,
`owners`, `targets`, `mentions`, and `broadcast`. `targets` highlights a post
for a specific agent / branch / session; `mentions` records a typed audience
marker that ships with the entry payload.

When the smoke passes on macOS, comment on Issue #2584 with the Board
link to close the Phase 12 / 13 verification follow-up.
