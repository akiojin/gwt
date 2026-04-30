#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="${GWT_INSTALL_DIR:-$HOME/.local/bin}"

usage() {
  cat <<'USAGE'
Usage: uninstall.sh [--dir <install-dir>]

Removes both gwt and gwtd from the target directory.
Defaults:
  dir: $GWT_INSTALL_DIR or $HOME/.local/bin
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dir)
      INSTALL_DIR="${2:?missing value for --dir}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

for BIN in gwt gwtd; do
  rm -f "$INSTALL_DIR/$BIN"
done

echo "Removed gwt and gwtd from $INSTALL_DIR"
