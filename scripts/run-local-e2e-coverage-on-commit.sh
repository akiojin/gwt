#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GUI_DIR="$ROOT_DIR/gwt-gui"

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

run_playwright_coverage() {
  local mode="$1"

  echo "Running commit-time E2E coverage in ${mode} mode..."
  (
    cd "$GUI_DIR"
    rm -rf .nyc_output e2e/.nyc_output coverage-e2e
    mkdir -p .nyc_output
    if [ "$mode" = "headed" ]; then
      E2E_COVERAGE=1 pnpm exec playwright test "${SUITES[@]}" --project=chromium --headed --workers=1
    else
      E2E_COVERAGE=1 pnpm exec playwright test "${SUITES[@]}" --project=chromium
    fi
    if ! python3 - <<'EOF'
from pathlib import Path
raise SystemExit(0 if any(Path(".nyc_output").glob("*.json")) else 1)
EOF
    then
      echo "E2E coverage run did not produce any .nyc_output JSON files." >&2
      exit 1
    fi
    pnpm exec nyc report --nycrc-path .nycrc.e2e.json
    node ../scripts/check-e2e-coverage-threshold.mjs
  )
}

headed_possible=1

if [ -n "${CI:-}" ]; then
  headed_possible=0
fi

if [ "$(uname -s)" = "Linux" ] && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
  headed_possible=0
fi

if [ "$headed_possible" -eq 1 ]; then
  run_playwright_coverage headed
  exit 0
fi

echo "Headed Playwright coverage unavailable in this environment; using headless mode."
run_playwright_coverage headless
