#!/usr/bin/env bash
# Issue #5272: reproduce a canonical verification TMPDIR below the host's
# native temporary root, including macOS /var/folders, without shortening it.
set -euo pipefail

socket_tmp=$(mktemp -d "${TMPDIR:-/tmp}/gwt-verify-5272.XXXXXX")
trap 'rm -rf "$socket_tmp"' EXIT
export TMPDIR="$socket_tmp/verification-socket-fixtures-$(printf 'nested-%.0s' {1..16})"
mkdir -p "$TMPDIR"

cargo test -p gwt-core --lib test_support::tests::socket_
cargo test -p gwt-core --test test_hygiene_test socket_
cargo test -p gwt-core --test daemon_socket_path_limit
cargo test -p gwt --lib daemon -- --test-threads=1
cargo test -p gwt --test runtime_tests daemon_project_root_test -- --test-threads=1
