#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GUI_DIR="$ROOT_DIR/gwt-gui"

# Keep commit-time E2E focused on the current shell/UX-critical flows.
SUITES=(
  "e2e/agent-canvas-browser.spec.ts"
  "e2e/agent-terminal.spec.ts"
  "e2e/branch-worktree.spec.ts"
  "e2e/dialogs-common.spec.ts"
  "e2e/project-management.spec.ts"
  "e2e/responsive-performance.spec.ts"
)

run_playwright() {
  local mode="$1"
  shift

  echo "Running commit-time E2E in ${mode} mode..."
  (
    cd "$GUI_DIR"
    if [ "$mode" = "headed" ]; then
      pnpm exec playwright test "${SUITES[@]}" --project=chromium --headed
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
  if run_playwright headed; then
    exit 0
  fi

  echo "Headed Playwright failed; retrying in headless mode..."
  run_playwright headless
  exit 0
fi

echo "Headed Playwright unavailable in this environment; using headless mode."
run_playwright headless
