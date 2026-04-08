# Quickstart: SPEC-10 - Project Workspace

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and enter the project initialization flow.
2. Exercise clone, existing repository selection, and migration paths with representative repositories.
3. Verify repository-kind detection and branch-protection behavior in the resulting workspace.
4. Treat the final coverage and manual-review tasks as the remaining work before closure.

## Phase 8 Reviewer Flow (Index Lifecycle Redesign)

These steps assume the Phase 8 implementation is in place (`bugfix/not-work-index` branch).

### 1. Cold-start runner auto-build (TUI-less)

```bash
# From a fresh shell, with no ~/.gwt/index/<repo-hash>/ present.
# Use a cross-platform SHA256 helper (`shasum -a 256` on macOS, `sha256sum`
# elsewhere) and canonicalize the worktree path to match the Rust helper,
# which calls `dunce::canonicalize` and resolves symlinks.
sha256hex() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | cut -c1-16
  else
    shasum -a 256 | cut -c1-16
  fi
}
REPO_HASH=$(printf '%s' "github.com/akiojin/gwt" | sha256hex)
WT_CANONICAL=$(python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$(pwd)")
WT_HASH=$(printf '%s' "$WT_CANONICAL" | sha256hex)

~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-files \
  --repo-hash "$REPO_HASH" \
  --worktree-hash "$WT_HASH" \
  --project-root "$(pwd)" \
  --query "watcher debounce" \
  --n-results 5
```

Expected: stderr shows NDJSON `{"phase":"indexing",...}` lines, then a final stdout JSON with `ok: true` and a non-empty `results` array. After completion, `~/.gwt/index/<REPO_HASH>/worktrees/<WT_HASH>/files/chroma.sqlite3` exists.

### 2. Legacy `.gwt/index` auto-removal

```bash
mkdir -p .gwt/index/legacy-junk
cargo run -p gwt-tui &
GWT_PID=$!
sleep 5
ls .gwt/index 2>&1 || echo "removed"
kill $GWT_PID
```

Expected: `.gwt/index` no longer exists after the TUI startup reconcile.

### 3. Async Issue refresh non-blocking

```bash
time cargo run -p gwt-tui --release -- --headless-bench-startup
```

Expected: startup completes in well under 2 seconds even when `gh issue list` would take 5+ seconds. Background refresh continues after the TUI is interactive.

### 4. Live SPEC edit reflected in search

1. Launch gwt-tui in this repo.
2. Open an agent pane on the current worktree.
3. In a separate shell, edit `specs/SPEC-10/spec.md` and add a unique sentinel string `WATCHER_SENTINEL_<random>`.
4. Within the agent pane, run `gwt-spec-search "WATCHER_SENTINEL_<random>"`.

Expected: the search returns SPEC-10 within 3 seconds.

### 5. Worktree remove cleans index

1. In gwt-tui, create a temporary worktree.
2. Note the path → compute its `wt-hash` → confirm `~/.gwt/index/<repo>/worktrees/<wt>/` exists.
3. Remove the worktree via gwt-tui.
4. Confirm `~/.gwt/index/<repo>/worktrees/<wt>/` is gone immediately.

## Original Reviewer Flow (Phases 1-7)

1. Run `cargo run -p gwt-tui` and enter the project initialization flow.
2. Exercise clone, existing repository selection, and migration paths with representative repositories.
3. Verify repository-kind detection and branch-protection behavior in the resulting workspace.
4. Treat the final coverage and manual-review tasks as the remaining work before closure.

## Expected Result
- The reviewer sees the current implemented scope for project workspace.
- Any missing behavior is logged against the remaining `2` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
