#!/usr/bin/env bash
#
# Re-run the changed crates' tests until their outcome wobbles (SPEC #4551
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
#   GWT_FLAKE_CRATES  space-separated crate names to exercise (required)
#   GWT_FLAKE_RUNS    how many consecutive runs to compare (default 20)
#   GWT_FLAKE_LOG_DIR where per-run logs are written (default: a temp dir)
#
# Default parallelism is deliberate: #4023 and #4182 only reproduce when the
# suite competes with itself for the host.

set -uo pipefail

crates="${GWT_FLAKE_CRATES:-}"
runs="${GWT_FLAKE_RUNS:-20}"

if [ -z "${crates// /}" ]; then
  echo "No changed crates to exercise; nothing to do."
  exit 0
fi

package_args=()
for crate in $crates; do
  package_args+=(-p "$crate")
done

log_dir="${GWT_FLAKE_LOG_DIR:-$(mktemp -d)}"
mkdir -p "$log_dir"

echo "Flake detection: $runs consecutive runs of ${package_args[*]}"
echo "Logs: $log_dir"

# Build once so a cold target/ does not count as the first run's latency and
# so a compile error fails fast instead of $runs times.
if ! cargo test "${package_args[@]}" --all-features --tests --no-run >"$log_dir/build.log" 2>&1; then
  echo "::error::Failed to build the test targets; see the log below."
  cat "$log_dir/build.log"
  exit 1
fi

# One line per run: "<exit status> <failed test names, sorted, space joined>".
# Two runs agree when their lines are identical.
outcomes=()
for run in $(seq 1 "$runs"); do
  log="$log_dir/run-$run.log"
  # No pipe: a pipeline would report the status of the tail, so a suite the
  # kernel killed would read as green.
  if cargo test "${package_args[@]}" --all-features --tests >"$log" 2>&1; then
    status=0
  else
    status=$?
  fi
  failed="$(grep -E '^test .+ \.\.\. FAILED$' "$log" | awk '{print $2}' | sort -u | tr '\n' ' ')"
  outcomes+=("$status ${failed% }")
  echo "run $run/$runs: status=$status failed=[${failed% }]"
done

stable=true
for outcome in "${outcomes[@]}"; do
  if [ "$outcome" != "${outcomes[0]}" ]; then
    stable=false
    break
  fi
done

if [ "$stable" = true ]; then
  if [ "${outcomes[0]}" = "0 " ] || [ "${outcomes[0]}" = "0" ]; then
    echo "Stable across $runs runs: every run passed."
  else
    echo "Stable across $runs runs: every run failed identically (${outcomes[0]})."
    echo "That is a deterministic failure, which \`Test (Rust)\` reports. Not a flake."
  fi
  exit 0
fi

echo "::error::Test outcome is not stable across $runs runs (SPEC #4551 AC-6)."
for run in $(seq 1 "$runs"); do
  echo "  run $run: ${outcomes[$((run - 1))]}"
done
echo
echo "A test whose result changes between identical runs depends on something"
echo "the source does not state -- wall-clock ordering, shared process state,"
echo "or a resource an earlier test never released. Per-run logs: $log_dir"
exit 1
