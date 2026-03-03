#!/usr/bin/env bash
set -euo pipefail

run_as_root() {
  if [[ "$(id -u)" -eq 0 ]]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    echo "error: sudo is required when not running as root" >&2
    exit 1
  fi
}

install_args=(-y)
if [[ "${NO_INSTALL_RECOMMENDS:-0}" == "1" ]]; then
  install_args+=(--no-install-recommends)
fi

if [[ "${SKIP_APT_UPDATE:-0}" != "1" ]]; then
  run_as_root apt-get update
fi

appindicator_pkg=""
if apt-cache show libappindicator3-dev >/dev/null 2>&1; then
  appindicator_pkg="libappindicator3-dev"
elif apt-cache show libayatana-appindicator3-dev >/dev/null 2>&1; then
  appindicator_pkg="libayatana-appindicator3-dev"
else
  echo "error: neither libappindicator3-dev nor libayatana-appindicator3-dev is available" >&2
  exit 1
fi

packages=(
  libwebkit2gtk-4.1-dev
  "${appindicator_pkg}"
  librsvg2-dev
  patchelf
)

run_as_root apt-get install "${install_args[@]}" "${packages[@]}"

if [[ "${CLEAN_APT_CACHE:-0}" == "1" ]]; then
  run_as_root apt-get clean
  run_as_root rm -rf /var/lib/apt/lists/*
fi
