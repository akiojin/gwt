#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMPDIR"
}
trap cleanup EXIT

printf '{"private":true,"type":"module"}\n' > "$TMPDIR/package.json"
ln -s "$ROOT/crates" "$TMPDIR/crates"

cd "$TMPDIR"

node_check() {
  node --preserve-symlinks --preserve-symlinks-main --check "$1"
}

node_check crates/gwt/web/app.js
node_check crates/gwt/web/branch-cleanup-modal.js
node_check crates/gwt/web/branch-list-state.js
node_check crates/gwt/web/migration-modal.js
node_check crates/gwt/web/project-clone-modal.js
node_check crates/gwt/web/board-surface.js
node_check crates/gwt/web/workspace-kanban-surface.js
node_check crates/gwt/web/theme-manager.js
node_check crates/gwt/web/theme-toggle.js
node_check crates/gwt/web/hotkey.js
node_check crates/gwt/web/operator-shell.js
node_check crates/gwt/web/focus-trap.js
node_check crates/gwt/web/window-docking.js
node_check crates/gwt/web/update-cta.js
node_check crates/gwt/web/terminal-context-menu.js
node_check crates/gwt/web/terminal-wheel-scroll.js
node_check crates/gwt/web/terminal-output-buffer.js
node_check crates/gwt/web/canvas-wheel-gesture.js
node_check crates/gwt/web/window-geometry-sync.js
node_check crates/gwt/web/custom-agent-env-editor.js
node_check crates/gwt/web/socket-receive-dispatcher.js
node_check crates/gwt/web/interaction-guard.js
node_check crates/gwt/web/viewport-persist-throttle.js
node_check crates/gwt/web/viewport-sync.js
node_check crates/gwt/web/project-tabs-renderer.js
node_check crates/gwt/web/window-tabs-renderer.js
node_check crates/gwt/web/clone-modal-focus-guard.js
node_check crates/gwt/web/ui-trace-profiler.js
node_check crates/gwt/web/ui-trace-wiring.js
node_check crates/gwt/web/close-project-tab-confirm-modal.js
node_check crates/gwt/web/release-notes-window.js
node_check crates/gwt/web/console-window.js
node_check crates/gwt/web/provider-usage-surface.js
node_check crates/gwt/web/terminal-attachments.js
node_check crates/gwt/web/project-index-search-surface.js
