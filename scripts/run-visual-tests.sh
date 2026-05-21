#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMPDIR"
}
trap cleanup EXIT

printf '{"private":true,"type":"module"}\n' > "$TMPDIR/package.json"
bun install --cwd "$TMPDIR" @playwright/test@1.49.1 >/dev/null

mkdir -p "$TMPDIR/crates/gwt"
cp -R "$ROOT/crates/gwt/playwright" "$TMPDIR/crates/gwt/playwright"
ln -s "$ROOT/crates/gwt/web" "$TMPDIR/crates/gwt/web"
rm -rf "$TMPDIR/crates/gwt/playwright/snapshots"
rm -rf "$TMPDIR/crates/gwt/playwright/test-results"
ln -s "$ROOT/crates/gwt/playwright/snapshots" "$TMPDIR/crates/gwt/playwright/snapshots"
ln -s "$ROOT/crates/gwt/playwright/test-results" "$TMPDIR/crates/gwt/playwright/test-results"

cd "$TMPDIR"
"$TMPDIR/node_modules/.bin/playwright" test \
  --config crates/gwt/playwright/playwright.config.ts "$@"
