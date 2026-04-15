#!/usr/bin/env bash
set -euo pipefail

# Pre-commit: fast, non-IO-bound tests only.
# Coordination/board/persistence tests share state with running gwt and may
# fail when the app is active.  Full suite runs in pre-push / CI.
cargo test -p gwt-core -- \
  --skip coordination \
  --skip repo_hash::tests::detect_repo_hash
