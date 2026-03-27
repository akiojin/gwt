#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GUI_DIR="$ROOT_DIR/gwt-gui"

# Keep commit-time E2E focused on the current shell/UX-critical flows.
SUITES=(
  "e2e/agent-canvas-browser.spec.ts"
  "e2e/agent-terminal.spec.ts"
  "e2e/branch-worktree.spec.ts"
  "e2e/cleanup-migration.spec.ts"
  "e2e/dialogs-common.spec.ts"
  "e2e/issue-cache-sync.spec.ts"
  "e2e/open-project-smoke.spec.ts"
  "e2e/pr-management.spec.ts"
  "e2e/project-management.spec.ts"
  "e2e/responsive-performance.spec.ts"
  "e2e/settings-config.spec.ts"
  "e2e/status-bar.spec.ts"
  "e2e/top-level-tools.spec.ts"
  "e2e/voice-input-settings.spec.ts"
  "e2e/web-preview-fallback.spec.ts"
  "e2e/windows-shell-selection.spec.ts"
)

run_playwright() {
  local mode="$1"
  shift

  echo "Running commit-time E2E in ${mode} mode..."
  (
    cd "$GUI_DIR"
    if [ "$mode" = "headed" ]; then
      pnpm exec playwright test "${SUITES[@]}" --project=chromium --headed --workers=1
    else
      pnpm exec playwright test "${SUITES[@]}" --project=chromium
    fi
  )
}

headed_possible=1

if [ -n "${CI:-}" ]; then
  headed_possible=0
fi

# Linux/X11/Wayland environments without a display cannot run headed.
if [ "$(uname -s)" = "Linux" ] && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
  headed_possible=0
fi

if [ "$headed_possible" -eq 1 ]; then
  run_playwright headed
  exit 0
fi

echo "Headed Playwright unavailable in this environment; using headless mode."
run_playwright headless
