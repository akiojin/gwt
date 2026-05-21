#!/usr/bin/env bash
set -euo pipefail

bunx --package @playwright/test@1.49.1 playwright test \
  --config crates/gwt/playwright/playwright.config.ts "$@"
