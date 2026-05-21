#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLAYWRIGHT_VERSION="${GWT_PLAYWRIGHT_VERSION:-1.49.1}"
PLAYWRIGHT_DEPS_DIR="${GWT_PLAYWRIGHT_DEPS_DIR:-${TMPDIR:-/tmp}/gwt-playwright-${PLAYWRIGHT_VERSION}}"
PLAYWRIGHT_NODE_MODULES="${PLAYWRIGHT_DEPS_DIR}/node_modules"
PLAYWRIGHT_BIN="${PLAYWRIGHT_NODE_MODULES}/.bin/playwright"
PLAYWRIGHT_RESOLVER="${PLAYWRIGHT_DEPS_DIR}/resolve-playwright.cjs"
RUN_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

mkdir -p "${PLAYWRIGHT_DEPS_DIR}"

if [[ ! -x "${PLAYWRIGHT_BIN}" ]]; then
  cat >"${PLAYWRIGHT_DEPS_DIR}/package.json" <<JSON
{"private":true,"dependencies":{"@playwright/test":"${PLAYWRIGHT_VERSION}"}}
JSON
  (
    cd "${PLAYWRIGHT_DEPS_DIR}"
    bun install --silent
  )
fi

cat >"${PLAYWRIGHT_RESOLVER}" <<'JS'
const Module = require("module");
const path = require("path");

const depsNodeModules = process.env.GWT_PLAYWRIGHT_NODE_MODULES;
const originalResolveFilename = Module._resolveFilename;

Module._resolveFilename = function resolveFromPinnedPlaywright(
  request,
  parent,
  isMain,
  options,
) {
  if (
    depsNodeModules &&
    (request === "@playwright/test" ||
      request.startsWith("@playwright/test/") ||
      request === "playwright/test" ||
      request.startsWith("playwright/"))
  ) {
    try {
      return originalResolveFilename.call(
        this,
        path.join(depsNodeModules, request),
        parent,
        isMain,
        options,
      );
    } catch {
      // Fall through to Node's normal resolver for non-package internals.
    }
  }
  return originalResolveFilename.call(this, request, parent, isMain, options);
};
JS

export GWT_PLAYWRIGHT_NODE_MODULES="${PLAYWRIGHT_NODE_MODULES}"
export NODE_PATH="${PLAYWRIGHT_NODE_MODULES}${NODE_PATH:+:${NODE_PATH}}"
export NODE_OPTIONS="--require ${PLAYWRIGHT_RESOLVER}${NODE_OPTIONS:+ ${NODE_OPTIONS}}"

mkdir -p "$RUN_DIR/crates/gwt"
cp -R "$ROOT/crates/gwt/playwright" "$RUN_DIR/crates/gwt/playwright"
ln -s "$ROOT/crates/gwt/web" "$RUN_DIR/crates/gwt/web"
rm -rf "$RUN_DIR/crates/gwt/playwright/snapshots"
rm -rf "$RUN_DIR/crates/gwt/playwright/test-results"
ln -s "$ROOT/crates/gwt/playwright/snapshots" "$RUN_DIR/crates/gwt/playwright/snapshots"
ln -s "$ROOT/crates/gwt/playwright/test-results" "$RUN_DIR/crates/gwt/playwright/test-results"

cd "$RUN_DIR"
"${PLAYWRIGHT_BIN}" test --config crates/gwt/playwright/playwright.config.ts "$@"
