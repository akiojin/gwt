#!/usr/bin/env bash
set -euo pipefail

cargo_cmd="cargo"
if ! command -v "$cargo_cmd" >/dev/null 2>&1; then
  if command -v cargo.exe >/dev/null 2>&1; then
    cargo_cmd="cargo.exe"
  else
    echo "[FAIL] cargo or cargo.exe is required on PATH" >&2
    exit 1
  fi
fi

"$cargo_cmd" test -p gwt-core -p gwt --all-features &
pid_rust=$!

bash scripts/check-release-flow.sh &
pid_release_flow=$!

fail=0
wait $pid_rust || fail=1
wait $pid_release_flow || fail=1
exit $fail
