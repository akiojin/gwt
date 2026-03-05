#!/usr/bin/env bash
set -euo pipefail

cargo test -p gwt-core -p gwt-tauri --all-features &
pid_rust=$!

(cd gwt-gui && pnpm test) &
pid_vitest=$!

fail=0
wait $pid_rust || fail=1
wait $pid_vitest || fail=1
exit $fail
