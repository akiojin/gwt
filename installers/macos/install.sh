#!/usr/bin/env bash
set -euo pipefail

REPO="akiojin/gwt"
INSTALL_DIR="${GWT_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="latest"

usage() {
  cat <<'USAGE'
Usage: install.sh [--version <version>] [--dir <install-dir>]

Installs both gwt and gwtd into the target directory.
Defaults:
  version: latest GitHub Release
  dir:     $GWT_INSTALL_DIR or $HOME/.local/bin
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      VERSION="${2:?missing value for --version}"
      shift 2
      ;;
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

case "$(uname -m)" in
  arm64|aarch64)
    ARCH="arm64"
    ;;
  x86_64|amd64)
    ARCH="x86_64"
    ;;
  *)
    echo "Unsupported macOS architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

ASSET="gwt-macos-${ARCH}.tar.gz"
if [ "$VERSION" = "latest" ]; then
  URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
else
  TAG="v${VERSION#v}"
  URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET}"
fi

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

mkdir -p "$INSTALL_DIR"
curl -fsSL "$URL" -o "$TMPDIR/$ASSET"
tar -xzf "$TMPDIR/$ASSET" -C "$TMPDIR"

for BIN in gwt gwtd; do
  SOURCE="$(find "$TMPDIR" -type f -name "$BIN" | head -n 1)"
  if [ -z "$SOURCE" ]; then
    echo "Downloaded archive does not contain $BIN" >&2
    exit 1
  fi
  cp "$SOURCE" "$INSTALL_DIR/$BIN"
  chmod +x "$INSTALL_DIR/$BIN"
done

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo "Warning: $INSTALL_DIR is not in PATH." >&2
    echo "Add it to your shell profile before running gwt or gwtd by name." >&2
    ;;
esac

echo "Installed gwt and gwtd into $INSTALL_DIR"
