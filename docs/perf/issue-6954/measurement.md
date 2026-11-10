# Issue #6954: bounded open-PR readback

Owner: Issue #6954. Related specification: SPEC #4093 FR-003/FR-005/FR-008.
Classification: gwt runtime behavior. Spec Status: IMPLEMENTATION-GAP.

## Plan

Goal: acquire at most one small timeline page per launch-discovery scan,
retain complete observations and pending pagination, and distinguish an
optional budget deferral from a failed scan.

Non-goals / what stays untouched: deadline lengths, queue/launch/claim and
permission policy, UI, PR #6928's automatic-retry fix, and Issue #6932's
notification aggregation.

Acceptance criteria: owner AC-1 through AC-4. A unit fixture cannot substitute
for the required one-hour machine observation. Keep unproven criteria open.

Alternatives: extending the deadline retains the workload; excluding queued
Issues weakens PR discovery. Incremental pagination preserves the existing
complete-timeline and unknown-hold contracts while bounding each scan's work.

Implementation boundary:

- `crates/gwt/src/cli/issue.rs`: resumable bulk pager; retain full-read behavior.
- `crates/gwt/src/issue_monitor_worker.rs`: one small page per discovery scan,
  completed-result caching, optional-budget diagnostics and regressions.
- `crates/gwt/src/issue_cache.rs`: classify optional refresh budget deferrals.
- `crates/gwt/src/cli/daemon/server.rs`: merge-readback recovery bookkeeping
  and existing launch-continuation regression, if needed.

## Baseline

Fetched base and branch: `1cc2b8ed9`, zero source difference, no current PR.
Machine: Windows, running installed v9.156.0; source-only changes are not
deployed evidence.

Fixed observation window: 2026-11-10 08:52:00–09:52:00 JST, project `E:\gwt`.
Sources: machine-local `~/.gwt/logs/gwt.log.2026-11-{09,10}` and
`~/.gwt/logs/errors/errors.2026-11-*.jsonl`.

- AC-1: 12 open-pr-readback deadline failures; not met.
- AC-2: 12 scan failures + 5 cache refresh failures; not met.
- AC-3: 828 open-pr-readback stage samples, nearest-rank p95 1364.0912 ms,
  maximum 14569.2364 ms. These samples include lifecycle reads and cache hits,
  so this is not the bulk-only p95.
- GraphQL timeline calls: 120, p95 5004 ms, maximum 8870 ms.
- A successful four-call bulk read took 13992 ms. Fourteen failures through
  09:54 reported completed Issues 66 (10 times), 69 (twice), 71 and 76.
  The repeated 66-result prefix supports repeated full discovery rather
  than incremental continuation. Approximately 80 Issues require four
  25-alias calls, not 80 individual gh processes.

Related ownership: PR #6928 / Issue #6877 owns automatic retries retaining
the periodic cache policy. PR #6939 / Issue #6932 aggregates Notifications
and deliberately retains raw ledger records.

## Five-alias live query probe

On the same Windows terminal, the source's timeline selection was queried
read-only for Issues 3431, 4093, 6877, 6932 and 6954, one page per request:

| Request | Success | Elapsed ms | Response bytes | Complete timelines |
| --- | --- | ---: | ---: | ---: |
| 1 | yes | 2440.88 | 286551 | 4/5 |
| 2 | yes | 2159.87 | 286551 | 4/5 |
| 3 | yes | 1845.35 | 286551 | 4/5 |

All three calls were below 3 seconds; the incomplete timeline demonstrates
why cursors must survive a scan. This short probe does not establish a p95
or substitute for the one-hour acceptance window.

## Verification and delivery

Native regression run: 16/16 PASS (bulk pagination/partial results,
periodic caches, optional-budget ledger distinction, explicit-refresh errors,
once-per-streak warning/reset, invalid-cursor recovery, and existing PR-hold
and claim guards). Both ledger classification and invalid-cursor recovery
were proven RED before their fixes. Test hygiene: 37 PASS, one maintenance
helper ignored; no baseline additions.

The short isolated live-stage smoke completed in 44.6 seconds with seven
successful open-PR readback samples, p95/max 1474.9561 ms, and zero
scan/cache ledger failures. Empty-cache initialization deferred candidate
cache refresh and merge history within their existing budgets; those
degradations were INFO events. This is not one-hour evidence.

The opt-in test `issue_6954_live_readonly_stages` performs the same read-only
stages against real GitHub and repository preferences, every 30 seconds,
with an isolated HOME. It never executes claims, launches, settlements,
branch pruning or Monitor commits. Set absolute `GWT_LIVE_MONITOR_REPO`
and `GWT_LIVE_MONITOR_OUTPUT` plus `GWT_LIVE_MONITOR_SECONDS=3600`, then run
`cargo test -p gwt --lib -- --ignored --exact issue_monitor_worker::live_readonly_stage_observation::issue_6954_live_readonly_stages --nocapture --test-threads=1`.
Each invocation preserves a unique output subdirectory containing JSON
trace, ledger and observation metadata. The full-hour verdict requires
zero scan/cache ledger failures, zero failed open-PR readbacks, observed
samples and p95 below 3000 ms. The metadata records the built source SHA.

The named daemon launch-continuation tests are Unix-only; Windows cannot
claim their execution. Required Linux CI must provide their fresh result.

At this source commit, canonical one-hour verification and delivery remain
pending. Their current evidence is recorded on Issue #6954 and SPEC #4093.
Do not close Issue #6954 based on source-only tests or the old installed
version's log. The isolated read-only stage observation does not claim to
replace or validate the user's running installed GUI.

User Verification Result: n/a (autonomous).
Agent Visual Check: n/a (no UI surface).
