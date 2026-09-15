#!/usr/bin/env bash
#
# Bounded apt-get for CI and local Linux installs (Issue #3701, Issue #4191).
#
# GitHub-hosted Ubuntu images often run unattended-upgrades just after boot
# and hold /var/lib/dpkg/lock-frontend. A bare `apt-get` then either fails
# immediately (`Could not get lock`) or waits without a deadline. This
# wrapper waits for the lock with a greppable log line, then runs apt-get
# with DPkg::Lock::Timeout so a lock that reappears mid-transaction still
# cannot hang the job.
#
# Issue #4191 added the rest. `Install tray + GTK dependencies (Linux)` is the
# first step of `Test (Rust)`, the only job that runs the whole workspace
# suite, and it used to run under a 5-minute GitHub step cap. Measured across
# the last 60 green runs the step takes p50 29s, p90 48s, p95 60s, max 106s,
# so that cap was ~3x the worst healthy run: a step that hit it was a stalled
# mirror, not a slow one, and the job then reported FAILURE having executed
# zero tests -- indistinguishable from a code failure.
#
# Raising the cap alone would not fix that, so the deadline now lives here:
#
#   * GWT_APT_TOTAL_DEADLINE bounds the whole invocation,
#   * a failed attempt is retried inside that deadline,
#   * GWT_APT_CACHE_DIR keeps the downloaded .debs so a warm CI cache
#     installs without touching a mirror,
#   * and giving up is reported as a dependency-install failure, in the log
#     and in the GitHub job summary, so "no tests ran" is legible at a glance.
#
# The workflow `timeout-minutes` is only the outer net and must stay above the
# total deadline; if GitHub kills the step first, none of the reporting above
# ever runs. crates/gwt/tests/ci_linux_dependency_install_contract_test.rs
# pins that relationship.
#
# Usage:
#   ci-apt.sh wait
#   ci-apt.sh update
#   ci-apt.sh install -y libgtk-3-dev ...
#   ci-apt.sh gtk-deps [extra packages...]
#
# Environment:
#   GWT_APT_LOCK_TIMEOUT     seconds to wait for a held lock (default: 180)
#   GWT_APT_LOCK_POLL        seconds between probe polls (default: 5)
#   GWT_APT_ATTEMPTS         attempts before giving up (default: 3)
#   GWT_APT_ATTEMPT_TIMEOUT  seconds allowed per apt-get call (default: 240)
#   GWT_APT_RETRY_DELAY      seconds between attempts (default: 15)
#   GWT_APT_TOTAL_DEADLINE   seconds for the whole invocation (default: 900)
#   GWT_APT_CACHE_DIR        .deb archive dir reused across CI runs
#   GWT_APT_GET              apt-get binary (default: apt-get)
#   GWT_APT_LOCK_PROBE       test-only command; exit 0 means the lock is held

set -euo pipefail

LOCK_TIMEOUT="${GWT_APT_LOCK_TIMEOUT:-180}"
POLL_SECONDS="${GWT_APT_LOCK_POLL:-5}"
ATTEMPTS="${GWT_APT_ATTEMPTS:-3}"
ATTEMPT_TIMEOUT="${GWT_APT_ATTEMPT_TIMEOUT:-240}"
RETRY_DELAY="${GWT_APT_RETRY_DELAY:-15}"
TOTAL_DEADLINE="${GWT_APT_TOTAL_DEADLINE:-900}"
CACHE_DIR="${GWT_APT_CACHE_DIR:-}"
APT_GET="${GWT_APT_GET:-apt-get}"

# The tray + WebView build dependencies every Linux job installs. This is the
# one home for the set: the workflows key their actions/cache entry on a hash
# of this file, so changing the list here invalidates the cached archives
# instead of serving the previous ones.
GTK_PACKAGES=(
  libgtk-3-dev
  libwebkit2gtk-4.1-dev
  libayatana-appindicator3-dev
  libxdo-dev
)

# Sentinels above the range apt-get and GNU timeout use, so "we ran out of
# time" and "the lock never cleared" stay distinguishable from an apt failure
# in the reason line the job summary quotes.
DEADLINE_EXHAUSTED=199
LOCK_CONTENTION=198

LAST_REASON="unknown"
LAST_ATTEMPT_BUDGET="${ATTEMPT_TIMEOUT}"
ATTEMPTS_USED=0
held_lock_path=""

log() {
  printf '[ci-apt] %s\n' "$*"
}

# Seconds left before the whole invocation must stop. SECONDS counts from
# script start, so this shrinks across attempts and lock waits alike.
remaining_seconds() {
  local left=$((TOTAL_DEADLINE - SECONDS))
  if ((left < 0)); then
    left=0
  fi
  printf '%s' "${left}"
}

smaller() {
  if (($1 < $2)); then
    printf '%s' "$1"
  else
    printf '%s' "$2"
  fi
}

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
  local budget
  budget="$(smaller "${LOCK_TIMEOUT}" "$(remaining_seconds)")"
  local waited=0
  while lock_is_held; do
    if ((waited >= budget)); then
      log "reason=dpkg lock contention after ${waited}s holder=${held_lock_path}"
      return 1
    fi
    log "waiting for dpkg lock holder=${held_lock_path} waited=${waited}s timeout=${budget}s"
    sleep "${POLL_SECONDS}"
    waited=$((waited + POLL_SECONDS))
  done
  if ((waited > 0)); then
    log "dpkg lock released after ${waited}s"
  fi
  return 0
}

# Bounds one apt-get call. Every Linux host that has apt-get also has
# coreutils `timeout`; the fallback keeps the script usable on hosts without
# it rather than refusing to run.
run_with_timeout() {
  local seconds="$1"
  shift
  local status=0

  if ((seconds <= 0)); then
    return "${DEADLINE_EXHAUSTED}"
  fi
  if command -v timeout >/dev/null 2>&1; then
    timeout -k 15 "${seconds}" "$@" || status=$?
  else
    "$@" || status=$?
  fi
  return "${status}"
}

apt_options() {
  APT_OPTIONS=(-o "DPkg::Lock::Timeout=${LOCK_TIMEOUT}")
  if [[ -n "${CACHE_DIR}" ]]; then
    APT_OPTIONS+=(-o "Dir::Cache::archives=${CACHE_DIR}")
  fi
}

# apt refuses an archive directory without its partial/ subdirectory, and the
# cache action runs as the unprivileged runner user while apt writes as root,
# so the tree is made world-readable before it is saved.
prepare_cache_dir() {
  [[ -n "${CACHE_DIR}" ]] || return 0
  mkdir -p "${CACHE_DIR}/partial"
  log "cache=${CACHE_DIR}"
}

publish_cache_dir() {
  [[ -n "${CACHE_DIR}" ]] || return 0
  chmod -R a+rX "${CACHE_DIR}" 2>/dev/null || true
}

run_apt_get() {
  wait_for_apt_lock || return "${LOCK_CONTENTION}"
  # The last attempt before the total deadline is truncated to whatever is
  # left, so the reason line has to quote the budget actually applied rather
  # than the configured one.
  LAST_ATTEMPT_BUDGET="$(smaller "${ATTEMPT_TIMEOUT}" "$(remaining_seconds)")"
  log "cmd=${APT_GET} ${APT_OPTIONS[*]} $* timeout=${LAST_ATTEMPT_BUDGET}s"
  run_with_timeout "${LAST_ATTEMPT_BUDGET}" "${APT_GET}" "${APT_OPTIONS[@]}" "$@"
}

# One attempt at whatever this invocation was asked to do. `gtk-deps` refreshes
# the index and installs in the same attempt on purpose: a retry that reused a
# half-fetched index would install from stale metadata.
run_once() {
  local status=0
  if [[ "${MODE}" == "gtk-deps" ]]; then
    run_apt_get update || return $?
    run_apt_get install -y "${PACKAGES[@]}" || return $?
    return 0
  fi
  run_apt_get "${ARGS[@]}" || status=$?
  return "${status}"
}

describe_failure() {
  local status="$1"
  if ((status == 124 || status == 137)); then
    printf 'timed out after %ss' "${LAST_ATTEMPT_BUDGET}"
  elif ((status == DEADLINE_EXHAUSTED)); then
    printf 'total deadline of %ss exhausted' "${TOTAL_DEADLINE}"
  elif ((status == LOCK_CONTENTION)); then
    printf 'dpkg lock contention'
  else
    printf 'apt-get exited %s' "${status}"
  fi
}

run_with_retries() {
  local attempt=1
  local status=0
  while ((attempt <= ATTEMPTS)); do
    log "phase=${MODE} attempt=${attempt}/${ATTEMPTS} elapsed=${SECONDS}s deadline=${TOTAL_DEADLINE}s"
    status=0
    run_once || status=$?
    ATTEMPTS_USED="${attempt}"
    if ((status == 0)); then
      log "phase=${MODE} attempt=${attempt}/${ATTEMPTS} status=ok"
      return 0
    fi

    LAST_REASON="$(describe_failure "${status}")"
    log "phase=${MODE} attempt=${attempt}/${ATTEMPTS} status=failed reason=${LAST_REASON}"

    if ((status == DEADLINE_EXHAUSTED)); then
      break
    fi
    attempt=$((attempt + 1))
    if ((attempt <= ATTEMPTS)); then
      if (($(remaining_seconds) <= RETRY_DELAY)); then
        LAST_REASON="total deadline of ${TOTAL_DEADLINE}s exhausted"
        break
      fi
      log "phase=${MODE} status=retrying in ${RETRY_DELAY}s"
      sleep "${RETRY_DELAY}"
    fi
  done
  return 1
}

# The whole point of owning the deadline: a dependency install that gives up
# says so, instead of leaving a job that ran no tests looking like a job whose
# tests failed.
report_failure() {
  log "phase=${MODE} status=failed attempts=${ATTEMPTS_USED}/${ATTEMPTS} reason=${LAST_REASON}"
  printf '::error title=Linux dependency install failed::scripts/ci-apt.sh %s gave up after %s attempt(s): %s. No test or build step in this job ran.\n' \
    "${MODE}" "${ATTEMPTS_USED}" "${LAST_REASON}"
  if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    {
      printf '### Linux dependency install failed - no tests were executed\n\n'
      printf '`scripts/ci-apt.sh %s` gave up after %s attempt(s): %s.\n\n' \
        "${MODE}" "${ATTEMPTS_USED}" "${LAST_REASON}"
      printf 'This is an infrastructure failure in the dependency install step, not a test or code failure. Every later step in this job was skipped, so this run says nothing about the change under review.\n'
    } >>"${GITHUB_STEP_SUMMARY}"
  fi
}

if [[ "$#" -eq 0 ]]; then
  echo "usage: $(basename "$0") wait | gtk-deps [packages...] | <apt-get arguments...>" >&2
  exit 2
fi

MODE="$1"

if [[ "${MODE}" == "wait" ]]; then
  wait_for_apt_lock
  exit $?
fi

PACKAGES=()
ARGS=()
if [[ "${MODE}" == "gtk-deps" ]]; then
  shift
  PACKAGES=("${GTK_PACKAGES[@]}")
  if (($# > 0)); then
    PACKAGES+=("$@")
  fi
else
  ARGS=("$@")
fi

apt_options
prepare_cache_dir

if run_with_retries; then
  publish_cache_dir
  exit 0
fi

report_failure
exit 1
