#!/usr/bin/env bash
#
# Fail the step when the test suite left a process from the build tree alive
# (SPEC #4551 T-050 / AC-5, mechanism (C)).
#
# #3040, #3845, #3911, #3954 and #4182 are all the same shape: a test spawns a
# child, asserts, and returns without waiting for it. Nothing in the source
# says so, and the suite stays green -- until the orphan holds a port, a lock
# or a lease and the *next* run hangs. #3845 hung the full suite forever;
# #4182 held the host verification lease and starved the whole fleet. The
# failure therefore has to be observed at runtime, right after the suite, and
# reported with enough detail to fix rather than to rerun.
#
# The population checked is deliberately narrow: processes whose executable
# lives under the workspace's target/ directory. Those can only have been
# built and started by this checkout, so a hit is a leak by construction and
# never a runner-infrastructure process. Orphans are reparented to init the
# moment their parent exits, so parentage cannot be used to find them.
#
#   GWT_ORPHAN_TARGET_DIR   build directory to scan (default: <repo>/target)
#   GWT_ORPHAN_GRACE_SECS   how long to wait for stragglers to exit (default 15)
#
# The grace period is what keeps this from becoming a flake gate that is
# itself flaky: a child in the middle of exiting is not a leak, so the scan
# repeats until the set is empty or the grace runs out.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${GWT_ORPHAN_TARGET_DIR:-$repo_root/target}"
grace_secs="${GWT_ORPHAN_GRACE_SECS:-15}"

if [ ! -d "$target_dir" ]; then
  echo "No build directory at $target_dir; nothing could have been spawned from it."
  exit 0
fi

# Resolve symlinks so the prefix comparison below matches the path the kernel
# reports for a running executable (/tmp vs /private/tmp on macOS).
target_dir="$(cd "$target_dir" && pwd -P)"

# Prints "<pid>\t<command line>" for every live process whose executable is
# under $target_dir. Linux is authoritative via /proc/<pid>/exe; elsewhere the
# argv[0] reported by ps is the best available approximation.
survivors() {
  if [ -d /proc ]; then
    local pid exe
    for entry in /proc/[0-9]*; do
      pid="${entry#/proc/}"
      exe="$(readlink "/proc/$pid/exe" 2>/dev/null)" || continue
      case "$exe" in
        "$target_dir"/*)
          printf '%s\t%s\n' "$pid" "$(tr '\0' ' ' <"/proc/$pid/cmdline" 2>/dev/null)"
          ;;
      esac
    done
  else
    ps -Ao pid=,args= 2>/dev/null | while read -r pid args; do
      case "$args" in
        "$target_dir"/*) printf '%s\t%s\n' "$pid" "$args" ;;
      esac
    done
  fi
}

deadline=$((SECONDS + grace_secs))
remaining="$(survivors)"
while [ -n "$remaining" ] && [ "$SECONDS" -lt "$deadline" ]; do
  sleep 1
  remaining="$(survivors)"
done

if [ -z "$remaining" ]; then
  echo "No processes from $target_dir survived the suite."
  exit 0
fi

echo "::error::The test suite left processes from the build tree running (SPEC #4551 AC-5)."
echo "Surviving processes under $target_dir after ${grace_secs}s:"
printf '%s\n' "$remaining" | while IFS=$'\t' read -r pid args; do
  echo "  pid=$pid cmdline=$args"
done
echo
echo "Each line is a child a test spawned and never reaped. Wait for it before"
echo "the test returns, or kill it in a guard that runs on every exit path."
exit 1
