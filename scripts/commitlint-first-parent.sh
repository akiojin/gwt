#!/usr/bin/env bash
set -euo pipefail

from_ref="${1:-HEAD~1}"
to_ref="${2:-HEAD}"

bunx commitlint \
  --from "${from_ref}" \
  --to "${to_ref}" \
  --git-log-args="--first-parent"
