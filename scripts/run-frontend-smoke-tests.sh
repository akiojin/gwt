#!/usr/bin/env bash
set -euo pipefail

bash scripts/run-node-tests-with-linkedom.sh \
  crates/gwt/web/__tests__/branch-cleanup.smoke.test.mjs \
  crates/gwt/web/__tests__/migration-modal.smoke.test.mjs \
  crates/gwt/web/__tests__/project-clone-modal.smoke.test.mjs \
  crates/gwt/web/__tests__/project-chrome-consolidation.smoke.test.mjs \
  crates/gwt/web/__tests__/close-project-tab.smoke.test.mjs
