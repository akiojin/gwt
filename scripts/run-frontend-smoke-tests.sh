#!/usr/bin/env bash
set -euo pipefail

pnpm dlx --package linkedom@0.18.12 node --test \
  crates/gwt/web/__tests__/branch-cleanup.smoke.test.mjs \
  crates/gwt/web/__tests__/migration-modal.smoke.test.mjs \
  crates/gwt/web/__tests__/project-clone-modal.smoke.test.mjs \
  crates/gwt/web/__tests__/open-project-split-button.smoke.test.mjs \
  crates/gwt/web/__tests__/close-project-tab.smoke.test.mjs
