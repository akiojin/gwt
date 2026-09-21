#!/usr/bin/env bash
#
# Re-run the changed test targets until their outcome wobbles (SPEC #4551
# T-055 / AC-6).
#
# The static gates in crates/gwt-core/tests/test_hygiene_test.rs reject the
# two mechanisms that have a textual shape. This catches what is left: a test
# whose result is not a function of its source. One green run proves nothing
# about such a test, which is exactly why the 16 issues this SPEC bundles were
# all discovered by an unrelated PR turning red.
#
# The comparison is between runs, not against green. A suite that fails the
# same way every time is a real failure and `Test (Rust)` already owns it;
# reporting it here twice would train people to ignore this job. Only a
# *changing* outcome is a flake.
#
#   GWT_FLAKE_TARGETS      space-separated `<crate>|test|<name>` / `<crate>|lib|`
#                          tokens naming the cargo test targets to exercise
#   GWT_FLAKE_RUNS         how many consecutive runs to compare (default 20)
#   GWT_FLAKE_BUDGET_SECS  wall-clock budget for the whole job (default 1800)
#   GWT_FLAKE_MIN_FREE_MB  stop repeating below this much free disk (default 2048)
#   GWT_FLAKE_LOG_DIR      where per-run logs go (default: a temp dir)
#
# AC-6 leaves N, scope and frequency to be decided and documented. Both are
# decided by measurement, not preference:
#
#   * Scope is per *target*, not per crate. One `cargo test --workspace
#     --all-features` costs 11m23s on the CI runner class, so repeating a
#     changed `gwt` twenty times could not fit any step budget.
#   * N is a ceiling, not a promise. A changed integration test costs seconds,
#     so it gets all 20 runs -- the cost model the SPEC assumed ("#4550 は 54
#     テスト 0.53 秒で、20 回でも 11 秒"). A `--lib` target does not: measured
#     warm, `cargo test -p gwt --lib` takes 381s and `-p gwt-core --lib` 193s,
#     which at N=20 would be 127 and 64 minutes. Rather than drop those targets
#     or hang the job, this times the first run and compares as many further
#     runs as the budget allows, never fewer than two, and says so in the log.
#   * Time is not the only budget. On 2026-09-21 a crate-scoped run of PR #4575
#     caught two genuine flakes (runs 4 and 6) and then filled the runner's
#     disk; the step summary write failed with "No space left on device", so
#     the findings vanished and the job was indistinguishable from an unrelated
#     runner fault. Three things follow: findings are reported the moment they
#     are observed, logs that merely agree with the first run are deleted
#     immediately, and a repetition loop stops on low disk and says plainly
#     that the check was cut short rather than failing as if it found something.
#
# Default parallelism is deliberate: #4023 and #4182 only reproduce when the
# tests compete with each other for the host.

set -uo pipefail

targets="${GWT_FLAKE_TARGETS:-}"
max_runs="${GWT_FLAKE_RUNS:-20}"
budget_secs="${GWT_FLAKE_BUDGET_SECS:-1800}"
min_free_mb="${GWT_FLAKE_MIN_FREE_MB:-2048}"

if [ -z "${targets// /}" ]; then
  echo "No changed test targets to exercise; nothing to do."
  exit 0
fi

# Turns one token into its cargo selector, e.g. `gwt|test|cli_test` into
# `-p gwt --test cli_test`.
selector_for() {
  local entry="$1" crate kind name rest
  crate="${entry%%|*}"
  rest="${entry#*|}"
  kind="${rest%%|*}"
  name="${rest#*|}"
  case "$kind" in
    test) printf -- '-p %s --test %s' "$crate" "$name" ;;
    lib) printf -- '-p %s --lib' "$crate" ;;
    *)
      echo "::error::Unrecognized flake target token: $entry" >&2
      return 1
      ;;
  esac
}

# "<exit status> <failed test names, sorted>" for one execution. Two runs agree
# when their outcomes are identical.
outcome_of() {
  local selector="$1" log="$2" status failed
  # No pipe around cargo: a pipeline would report the status of the tail, so a
  # suite the kernel killed would read as green.
  # shellcheck disable=SC2086 # the selector is a deliberate argument list
  if cargo test $selector --all-features >"$log" 2>&1; then
    status=0
  else
    status=$?
  fi
  failed="$(grep -E '^test .+ \.\.\. FAILED$' "$log" | awk '{print $2}' | sort -u | tr '\n' ',')"
  printf '%s %s' "$status" "${failed%,}"
}

log_dir="${GWT_FLAKE_LOG_DIR:-$(mktemp -d)}"
mkdir -p "$log_dir"

# Free megabytes on the filesystem the repeated runs write to.
free_mb() {
  df -Pm "$log_dir" 2>/dev/null | awk 'NR==2 {print $4}'
}

# Findings go here the moment they are observed. On 2026-09-21 a run of the
# crate-scoped version caught two flakes and then filled the runner's disk; the
# step summary write failed, so the detection was invisible and the job looked
# exactly like an infrastructure failure. Reporting late is how a gate loses the
# result it just produced.
report_finding() {
  echo "$1"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    echo "$1" >>"$GITHUB_STEP_SUMMARY" 2>/dev/null || true
  fi
}

target_count=0
for _ in $targets; do target_count=$((target_count + 1)); done
budget_share=$((budget_secs / target_count))

echo "Flake detection: up to $max_runs runs per target, ${budget_secs}s total budget"
echo "Targets ($target_count): $targets"
echo "Logs: $log_dir  (free: $(free_mb)MB, floor: ${min_free_mb}MB)"

# Build once so a cold target/ does not count as the first run's latency and so
# a compile error fails fast instead of once per run.
for entry in $targets; do
  selector="$(selector_for "$entry")" || exit 1
  # shellcheck disable=SC2086 # the selector is a deliberate argument list
  if ! cargo test $selector --all-features --no-run >>"$log_dir/build.log" 2>&1; then
    echo "::error::Failed to build $entry; see the log below."
    cat "$log_dir/build.log"
    exit 1
  fi
done

wobbled=""
cut_short=""
for entry in $targets; do
  selector="$(selector_for "$entry")"
  safe="${entry//|/_}"

  started=$SECONDS
  first="$(outcome_of "$selector" "$log_dir/$safe-run-1.log")"
  elapsed=$((SECONDS - started))
  [ "$elapsed" -lt 1 ] && elapsed=1

  runs=$((budget_share / elapsed))
  [ "$runs" -gt "$max_runs" ] && runs=$max_runs
  [ "$runs" -lt 2 ] && runs=2

  echo "$entry: one run took ${elapsed}s; comparing $runs runs (budget share ${budget_share}s)"
  echo "  run 1/$runs: $first"

  unstable=false
  completed=1
  for run in $(seq 2 "$runs"); do
    # From the third run on. Two runs is the least that can detect anything and
    # is already the floor the time budget guarantees, so there is nothing to
    # protect below it -- stopping at one run would only produce a job that
    # costs a runner and reports nothing.
    available="$(free_mb)"
    if [ "$run" -gt 2 ] && [ -n "$available" ] && [ "$available" -lt "$min_free_mb" ]; then
      cut_short="${cut_short:+$cut_short }$entry(disk:${available}MB)"
      echo "  stopping after $completed run(s): only ${available}MB free, floor is ${min_free_mb}MB"
      break
    fi
    log="$log_dir/$safe-run-$run.log"
    outcome="$(outcome_of "$selector" "$log")"
    completed=$((completed + 1))
    echo "  run $run/$runs: $outcome"
    if [ "$outcome" = "$first" ]; then
      # A run that agrees with the first proves nothing further and its log is
      # the bulk of what this job writes. Twenty full-suite logs are what ran
      # the runner out of space on 2026-09-21.
      rm -f "$log"
    else
      unstable=true
      report_finding "FLAKE: $entry differs between runs -- run 1: [$first] vs run $run: [$outcome]"
    fi
  done

  if [ "$unstable" = true ]; then
    wobbled="${wobbled:+$wobbled }$entry"
    echo "  -> UNSTABLE after $completed run(s)"
  elif [ "$first" = "0 " ] || [ "$first" = "0" ]; then
    echo "  -> stable: $completed run(s), every one passed"
  else
    echo "  -> stable: $completed run(s), every one failed identically ($first)."
    echo "     That is a deterministic failure, which \`Test (Rust)\` reports. Not a flake."
  fi
done

if [ -z "$wobbled" ]; then
  if [ -n "$cut_short" ]; then
    # Not a gate failure. Saying so explicitly is the whole point: on
    # 2026-09-21 a disk-exhausted run was indistinguishable from a detection,
    # and from an unrelated runner fault.
    echo "::warning::Flake detection was cut short by the environment, not by a finding: $cut_short"
    echo "No target wobbled in the runs that did complete. This is an incomplete check, not a red gate."
    exit 0
  fi
  echo "Every changed target produced the same outcome on every run."
  exit 0
fi

echo "::error::Test outcome is not stable across runs (SPEC #4551 AC-6): $wobbled"
if [ -n "$cut_short" ]; then
  echo "(Some targets were also cut short by the environment: $cut_short --"
  echo " that is separate from the finding above, which stands on its own.)"
fi
echo
echo "A test whose result changes between identical runs depends on something"
echo "the source does not state -- wall-clock ordering, shared process state,"
echo "or a resource an earlier test never released. Per-run logs: $log_dir"
exit 1
