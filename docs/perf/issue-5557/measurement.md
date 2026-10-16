# Issue #5557: scan starvation and queue admission

## Scope and source

This is a gwt runtime feature fix under SPEC #5390, not a repository-only
operating rule. The initial checkout contained no unique source commits after
fetching. Investigation used `origin/develop` at
`c409d53e092e7f23245acba7a335892485a02c88` and the scoped working changes.

The change preserves the existing claim planner, authority epochs, revision
fences, effect journal, and remote completion checks. No UI rendering, settings
schema, provider startup, or execution-generation recovery policy changes.

## Incident baseline

`baseline-errors.json` contains the original error-ledger records. At
2026-10-15T16:43:35Z the last scan was still 16:28:35Z; at 17:15:30Z it was
still 17:00:28Z. Both report a proposal-return deadline failure followed by
`launch_suppressed`. Issue #5557 reports eight active agents out of twelve
and queued work waiting for more than thirteen minutes during this incident.
These are production observations from the old runtime, not a paired benchmark
of provider startup.

The old default poll interval was 300 seconds. The shared scan deadline could
grow to 300 seconds with the number of in-flight candidates. Failed scans did
not schedule a short retry. `ScanNow` acknowledged the request but waited for
the existing full scan to finish before another scan could prepare admission.

## Cause and correction

The maintenance loop checked that more than ten seconds remained before
starting a candidate. A candidate could then run several fifteen-second
readbacks. Each readback inherited the whole remaining outer deadline, so it
could consume those ten seconds. Its failure became a degradation, but the
final proposal-return check discarded the whole scan, including launch work.

The deterministic operation-clock regression starts with eleven seconds left.
The old helper gives a readback eleven seconds; the fixed helper gives it one
second and retains ten for admission. A further maintenance read is refused
before its closure runs, and proposal-return remains valid. Branch pruning
uses the same bounded maintenance budget.

The normal poll interval is now at most thirty seconds, the shared scan
deadline is capped at sixty seconds, and a failed scan schedules a five-second
retry. Missed ticks are skipped. A started blocking scan remains single-flight
until it joins; the watchdog never creates overlapping full scans.

An explicit `ScanNow` also starts a bounded admission proposal independently
of maintenance. It rebases current preferences and refreshes the claim frontier.
Only a known, unchanged issue revision can reuse the full scan's eligibility
decision. Unknown or changed revisions defer to a full scan. Completion probes,
capacity/hold checks, durable commit fences, and the existing effect executor
remain authoritative. Successful commits advance the revision so concurrent
older proposals cannot replace them.

## Measurements and regression evidence

`controlled-timing.txt` records the native driver tests and measured durations.
The blocked-maintenance fixture starts with a free slot and a queued issue,
sends the real control frame through the hub, and measures launch-request
dispatch without releasing the maintenance scan. Remote observations and claim
success are deterministic fixtures; the effect journal, durable delivery, and
dispatch are real driver paths. No GitHub write or real provider starts.

| Metric | Before | After |
| --- | --- | --- |
| Queued head with available capacity | More than 13 minutes in the reported incident | 90 ms to launch-request dispatch while maintenance stays blocked |
| Failed-scan retry | Next 300-second poll in the old driver | 5,121 ms; two scans observed with the poll configured to 300 seconds |
| Maintenance budget with 11 seconds left | 11 seconds, including the launch reserve | 1 second; ten seconds retained for launch |
| Default poll / maximum scan budget | 300 / 300 seconds | 30 / 60 seconds |

The incident and controlled dispatch numbers come from different environments;
they demonstrate the removed dependency, not a provider-startup speedup ratio.

The retry fixture configures a 300-second poll, fails the first real driver
scan, and observes the next scan through the retry path. Its printed
`retry_ms` is measured wall time, not an assertion that a busy runner must
schedule work at an exact millisecond.

Stage logs use `gwt.issue_monitor.scan`. The full scan attributes prefs load,
capacity refresh, remote resolution, candidate load/reconciliation, merge and
window reconciliation, autonomous progress, pruning, issue settlement, claim
preparation, proposal-return, and total time. Individual remote stages record
duration and success, including failures. These distinguish local observation
cost from GitHub readbacks instead of blaming proposal-return alone.

The controlled results are not a live fleet service-level guarantee. Whole-fleet
eight-agent measurements remain owned by Issue #5385. Provider process startup
and human visual confirmation are outside these backend measurements.

## Verification contract

The acceptance matrix includes the Issue #5557 regressions, existing worker and
Monitor configuration tests, the independent daemon-supervision contract,
test hygiene, formatting, and workspace clippy.
Canonical pre-PR verification delegates the full platform and coverage matrix to
the repository's required CI checks; delivery requires those checks and a merge
into develop. Local workspace coverage is not measured.

Acceptance Surface: non-user-facing backend admission and scan timing; no
rendering, controls, protocol output, or browser interaction changes.

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface)

Rollback is a revert of this scoped change. The conservative revision rule can
defer a changed candidate until its next full scan; it does not authorize work
from stale issue content.
