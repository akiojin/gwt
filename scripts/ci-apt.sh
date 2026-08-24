#!/usr/bin/env bash
#
# Bounded apt-get for CI and local Linux installs (Issue #3701).
#
# GitHub-hosted Ubuntu images often run unattended-upgrades just after boot
# and hold /var/lib/dpkg/lock-frontend. A bare `apt-get` then either fails
# immediately (`Could not get lock`) or waits without a deadline. This
# wrapper waits for the lock with a greppable log line, then runs apt-get
# with DPkg::Lock::Timeout so a lock that reappears mid-transaction still
# cannot hang the job.
#
# Usage:
#   ci-apt.sh wait
#   ci-apt.sh update
#   ci-apt.sh install -y libgtk-3-dev ...
#
# Environment:
#   GWT_APT_LOCK_TIMEOUT  seconds to wait for a held lock (default: 180)
#   GWT_APT_LOCK_POLL     seconds between probe polls (default: 5)
#   GWT_APT_GET           apt-get binary (default: apt-get)
#   GWT_APT_LOCK_PROBE    test-only command; exit 0 means the lock is held

set -euo pipefail

LOCK_TIMEOUT="${GWT_APT_LOCK_TIMEOUT:-180}"
POLL_SECONDS="${GWT_APT_LOCK_POLL:-5}"
APT_GET="${GWT_APT_GET:-apt-get}"

log() {
  printf '[ci-apt] %s\n' "$*"
}

held_lock_path=""

fuser_lock() {
  local lock="$1"
  if [[ "$(id -u)" -ne 0 ]] && command -v sudo >/dev/null 2>&1; then
    sudo fuser "${lock}" >/dev/null 2>&1
  else
    fuser "${lock}" >/dev/null 2>&1
  fi
}

lock_is_held() {
  if [[ -n "${GWT_APT_LOCK_PROBE:-}" ]]; then
    held_lock_path="probe"
    if "${GWT_APT_LOCK_PROBE}"; then
      return 0
    fi
    return 1
  fi

  if ! command -v fuser >/dev/null 2>&1; then
    return 1
  fi

  local lock
  local locks=(
    /var/lib/dpkg/lock-frontend
    /var/lib/dpkg/lock
    /var/lib/apt/lists/lock
    /var/cache/apt/archives/lock
  )
  for lock in "${locks[@]}"; do
    if [[ -e "${lock}" ]] && fuser_lock "${lock}"; then
      held_lock_path="${lock}"
      return 0
    fi
  done
  return 1
}

wait_for_apt_lock() {
  local waited=0
  while lock_is_held; do
    if (( waited >= LOCK_TIMEOUT )); then
      log "reason=dpkg lock contention after ${LOCK_TIMEOUT}s holder=${held_lock_path}"
      return 1
    fi
    log "waiting for dpkg lock holder=${held_lock_path} waited=${waited}s timeout=${LOCK_TIMEOUT}s"
    sleep "${POLL_SECONDS}"
    waited=$((waited + POLL_SECONDS))
  done
  if (( waited > 0 )); then
    log "dpkg lock released after ${waited}s"
  fi
  return 0
}

run_apt_get() {
  wait_for_apt_lock
  log "cmd=${APT_GET} -o DPkg::Lock::Timeout=${LOCK_TIMEOUT} $*"
  "${APT_GET}" -o "DPkg::Lock::Timeout=${LOCK_TIMEOUT}" "$@"
}

if [[ "${1:-}" == "wait" ]]; then
  wait_for_apt_lock
  exit $?
fi

if [[ "$#" -eq 0 ]]; then
  echo "usage: $(basename "$0") wait | <apt-get arguments...>" >&2
  exit 2
fi

run_apt_get "$@"
