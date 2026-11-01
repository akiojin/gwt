#!/usr/bin/env bash
# The isolated fixture's normal managed launch provides the test source;
# the live owner's old runtime remains unknown and is never rewritten.
set -euo pipefail
REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"
for CHECK_BIN in target/debug/gwt target/debug/gwtd; do
  test -x "$CHECK_BIN" || { echo 'Build checkout gwt and gwtd first' >&2; exit 1; }
done
command -v jq >/dev/null || { echo 'jq is required for browser-check' >&2; exit 1; }
export GWT_PLAYWRIGHT_BROWSER_PREFLIGHT=1
bash scripts/run-visual-tests.sh crates/gwt/playwright/tests/browser-preflight-live.spec.ts "$@"
