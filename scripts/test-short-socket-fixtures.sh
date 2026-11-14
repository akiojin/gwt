#!/usr/bin/env bash
# Issue #5272: reproduce a canonical verification TMPDIR below the host's
# native temporary root, including macOS /var/folders, without shortening it.
set -euo pipefail

socket_tmp=$(mktemp -d "${TMPDIR:-/tmp}/gwt-verify-5272.XXXXXX")
trap 'rm -rf "$socket_tmp"' EXIT
export TMPDIR="$socket_tmp/verification-socket-fixtures-$(printf 'nested-%.0s' {1..16})"
mkdir -p "$TMPDIR"

# The socket group was built before the execution budget. Keep its shared
# feature graph instead of asking Cargo to rebuild the narrower core graph.
node scripts/ci-windows-tests.mjs run-socket gwt-core lib gwt_core test_support::tests::socket_
node scripts/ci-windows-tests.mjs run-socket gwt-core test test_hygiene_test socket_
node scripts/ci-windows-tests.mjs run-socket gwt-core test daemon_socket_path_limit
node scripts/ci-windows-tests.mjs run-socket gwt lib gwt daemon -- --test-threads=1
node scripts/ci-windows-tests.mjs run-socket gwt test runtime_tests daemon_project_root_test -- --test-threads=1
