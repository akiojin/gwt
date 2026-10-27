# Issue #5390: PR inventory read failures, slice A

This slice repairs the existing inventory truthfulness contract. A required
local enumeration or divergence read failure now returns the existing CLI error
instead of a successful empty stocktake. Optional source classification failure
still retains the branch with `has_non_gwt_changes: null`. Stale PR snapshots
still report an unobserved stocktake through the existing null contract.

The initial truthfulness comparison below is preserved from source commit
`a7c740931812a6daa30287b85118038e622e0ade`. The final compatibility repair and
its additional checked base read are recorded separately below; the original
RED/GREEN hashes and timings have not been reassigned to the later code.

## Reproduction and provenance

Run `cargo test -p gwt --lib pr_list_tracked_local_failures_are_errors_and_recover -- --nocapture`.
The existing fake-gh harness holds the environment lock and uses a fresh scoped
GWT home. The test initializes a real Git repository with the existing trusted
store fixture, pins both commit dates to `2026-10-27T00:00:00Z`, creates an empty
base commit, and commits `source.txt` containing `unlanded source` plus a newline.
It sets `origin/develop` to the base and `origin/work/issue-5390` to the source
commit. The initial PR cache is fresh, has default includes, and contains no PRs.

A native Git shim logs argv, fails only the selected command, and forwards all
other commands to the real Git executable. The actual JSON dispatcher calls
`DefaultCliEnv::list_open_prs`, `fetch_pr_inventory_tracked`, the local collector,
and the production renderer/error response. No production caller seam was added.
The cold PR read uses the existing fake-gh executable; it never contacts GitHub.

The immutable fixture manifest SHA-256 is
`bbb4adff7f2f880feee2c60ca4f21fe02166c8f05c5801b40c20690a1965a7d3`.
Both runs produced the same base commit
`5d00c01646d1f2f24cdd202ad3f56e7f4f411126`, source commit
`d195eb494538f935704e061a70ccc0e0086cc949`, and source tree
`b6982d6579593be9aeba760306725a6f43e84a51`.
The generated temporary directory and runtime-relative cache timestamps differ.

[RED](pr-list-a-red.json) and [GREEN](pr-list-a-green.json) preserve all seven
observations, complete envelopes, argv, counts, times, fixture manifest, tool
versions, and source/test/native-test-executable SHA-256 values. Their
`source_head` is the common baseline `92462a39a8d667cc0551eb9d4bf0f4dfe8095a13`;
the source hashes identify the uncommitted measured changes. Final producing
HEAD, canonical verification, CI coverage, and delivery are recorded separately
on the owner Issue and PR after commit.

RED failed at `assert_ne(code, 0)`: both required read failures had returned
`exit_code: 0`, `ok: true`, `source: cache`, and `unlanded_branches: []` / count 0.
GREEN returned `exit_code: 1`, `ok: false`, and the existing `error` field for
both failures, with no successful stocktake payload.

The first GREEN attempt already produced those correct responses but the new
test then looked for their text in `output`, causing an assertion-side panic.
Only that previously unreachable check was corrected to the existing `error`
field. The RED guard and fixture were unchanged; the final native test passed.
The intentionally different test hashes and the failed attempt's log hash are
retained in GREEN metadata.

## Same-condition observations

These are single functional observations, not a latency benchmark or SLA.
Git call counts include the native shim invocations observed during dispatch.

| Observation | Params / PR cache | Before result | After result | Git calls before/after | Total ms before/after |
| --- | --- | --- | --- | --- | --- |
| Enumeration failure | `{}` / warm | Success, empty/0 | Standard error | 1/1 | 111.02/134.70 |
| Divergence failure | `include=[]` / warm | Success, omitted branch/0 | Standard error | 2/2 | 144.85/163.79 |
| Normal recovery | `{}` / warm | Source branch/1 | Source branch/1 | 4/4 | 415.43/703.30 |
| Optional classification failure | `include=[]` / warm | Branch/1, unknown | Branch/1, unknown | 3/3 | 235.65/238.03 |
| Complete empty stocktake | `include=[]` / warm | Empty/0 | Empty/0 | 1/1 | 98.90/109.14 |
| Cold PR read | `include=[]` / absent | Source github, empty/0 | Source github, empty/0 | 1/1 | 279.48/306.66 |
| Warm PR read | `include=[]` / warm | Source cache, empty/0 | Source cache, empty/0 | 1/1 | 121.17/103.43 |

The cold PR observation makes two fake-gh invocations: the attempted free
rate-limit probe and the light PR list. The existing response reports one
budget-spending `github_calls`; the next warm read makes no additional gh call.
Successful PR provenance is unchanged, and local enumeration still runs for a
warm PR cache and for `include=[]`.

## Regression and measurement boundaries

The existing `gwt-git` `pr_status::tests` cover delivery refs, exact
`pm/resident`, unique commits, open-PR exclusion, bookkeeping-only branches,
owner inference, optional unknown classification, cache/REST/budget/backoff,
and stale-history protection. Existing CLI inventory tests cover successful
rendering and stale nulls. The new test supplies the missing actual tracked
error and recovery assertions, rather than testing only the renderer.

## Review compatibility repair

PR review exposed an existing hardcoded `origin/develop` assumption that became
a visible operation error after required-read failures stopped being hidden.
The added tracked-reader regression first failed with
`fatal: malformed object name origin/develop` in a main-only project.
[Base regression evidence](pr-list-a-base-review.json) preserves that RED and
GREEN with its source and native test executable hashes. Its GREEN precedes a
documentation-only update clarifying that the public constant is preferred.

The private resolver uses the existing checked `remote_branch_exists` reads.
It prefers `origin/develop`, then uses the full local `origin/HEAD` target,
and tries `origin/main` / `origin/master` only when the default hint is absent.
A dangling explicit default and the absence of every usable remote base remain
errors. It never uses local `HEAD` to manufacture an empty inventory. No fetch,
remote mutation, public API, settings, cache policy, or authority was added.

One fixture proves main-only reads, a custom remote default preceding main,
develop precedence, dangling-default errors, and no-base errors. The final
unchanged actual CLI fixture is preserved in
[compatibility observations](pr-list-a-green-compat.json), including the final
source/test/binary hashes, all seven complete responses, argv, counts and times.
On this develop fixture, each observation now includes one checked `show-ref`
before the stocktake. This additional read preserves integration-base validity;
it is not claimed as a latency reduction. The original comparison remains
historical evidence, while final canonical verification binds the final HEAD.

CI also identified the new CLI fixture's environment lock as invisible to the
existing same-function contract: the lock had been acquired inside its shared
helper in another file. The test now explicitly holds the existing reentrant
`env_test_lock` in its own function. The helper and guard remain unchanged.
The existing `ci_contracts` environment-lock test is included in acceptance
verification. The failed coverage shard could not aggregate coverage; it was
unmeasured, not a threshold failure.

`total_ms` covers dispatcher entry through captured JSON parsing, excluding
process startup and `DefaultCliEnv` construction. `logged_git_ms` is the
current-thread sum from the existing Git instrumentation. Per-command duration
and separate root/cache/history/reply durations are unmeasured. Non-Git time
cannot be assigned to one of those stages from this fixture.

The native test executable invokes production code in-process; it is not an
installed standalone gwtd process. This evidence does not diagnose the original
180-second no-response report, measure real GitHub latency, establish a new
deadline, or prove fleet CPU/RSS/timeout improvements. Those remain slice B
and the parent's existing acceptance criteria.

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface). The changed tracked reader has only
the CLI production caller; the GUI's separate cached reader is unchanged.
