#!/usr/bin/env bash
set -euo pipefail

cargo test -p gwt-core -p gwt --all-features &
pid_rust=$!

fail=0
wait $pid_rust || fail=1
exit $fail
