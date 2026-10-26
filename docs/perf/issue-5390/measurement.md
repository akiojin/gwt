# Issue #5390: Windows recurrence, Agent naming census

This is a gwt runtime performance fix. It applies to every project opened with
gwt. The parent performance family remains open.

## Observed production context

The unchanged installed v9.145.0 process was PID 33132, started at
2026-10-26T01:37:09.7582637Z. No production process was stopped, restarted,
or attached to a debugger. The active implementation Session, other project
agents and background work were left running. This is observed normal load,
not controlled idle. Compiler children are excluded from process CPU.

The 2026-10-26T15:23:14–15:23:44Z sample consumed 63.1875 process CPU seconds
over 30.0643549 wall seconds: 2.101741421 average logical cores.
Surviving threads included Tokio TID 32284 at 16.515625 CPU seconds,
main TID 31096 at 3.8125, Monitor TID 34792 at 3.578125 and capacity TID 31724
at 1.578125. Endpoint-only thread accounting misses exited workers and is
not a complete attribution of process CPU.

`live-context.json` retains the exact binary hash, identity, uptime and memory
snapshot. WS/private are observations at that timestamp, not a monotonic
memory-leak proof. `live-thread-cpu.jsonl` retains the two raw samples.

In the saved GUI dispatch summaries, TerminalConvergenceTick was the largest
measured handler: 59,727.74ms across 11 windows, maximum 3,027.36ms.
These summaries record slow handlers only. Their total is elapsed handler
time, not CPU time or input-to-paint latency. The complete selected interval
is retained in `live-dispatch.jsonl`.

WPR CPU capture was unavailable: `wpr -status` reported no active recording;
`wpr -start CPU -filemode` refused with 0xc5585011, “Failed to enable the
policy to profile system performance.” No elevation or policy change was
attempted. The evidence here is thread deltas, dispatch timing and direct
source-path analysis, not a complete native stack profile.

## Root cause and bounded fix

The 15-second TerminalConvergenceTick invokes `assign_missing_agent_names`.
Even already named and member-bound Sessions reached `observe_session`,
which constructs a fresh full OS process snapshot, followed by roster
read/reconciliation in `adopt_project_member_session`. Both were unnecessary
for a Session that already has its member identity.

Add the existing missing-member predicate before that adoption path.
Keep Session loading and name repair: an existing regression requires a
named legacy window to backfill an unnamed Session. Name allocation,
cross-project initial reservations, PM exclusion and explicit restore/launch
membership validation remain in their original paths. No timer, cache,
scheduler, CPU cap, authority or performance budget changes.

## Regression and route timing

The focused test uses the same live Session/sidecar fixture, already bound
member identity, existing window name, and unreadable unrelated roster.

| Source | Result | Naming operation elapsed |
| --- | --- | --- |
| Before guard | RED: roster JSON parse error | 34.6622ms |
| After guard | GREEN: existing name returned, roster untouched | 0.4469ms |

This is one debug-build route observation per source, under the current host
load, not a statistical benchmark, shipping CPU result or deployed-fleet
acceptance. The temporary timing print was removed after measurement.
The fresh canonical run must test the final source.

The initial focused suite passed 7 existing tests and failed the new test for
the expected reason. After the one-predicate production change, all 8 passed:
self-only naming/reservations, registered PM exclusion, projection invalidation,
provider identity, bound Session independence, PM/missing rejection,
named-legacy Session backfill and custom provider classification.
The legacy unnamed-window regression is also included in final verification.

## Residual work

A separate 15-second/19-snapshot observation found 56 distinct GUI-child Git
process lifetimes: 24 status, 32 ignored ls-files, maximum 8 simultaneously.
These are lower bounds from 500ms polling with CIM overhead; PID/start time
distinguish reused PIDs. Raw evidence is in `live-git-children.json`.

Projection diagnosis, Monitor observations and background terminal convergence
recheck persisted PathDirty settlement through this Git path. It also feeds
terminal-close safety, so omitting the scan would require a separate freshness
and fail-closed design. This slice does not claim to resolve that repeated
work, the hot Tokio worker, full process CPU, real-fleet 60-minute acceptance,
or four-hour memory acceptance. Existing parent ACs remain unchecked.

User Verification Result: n/a (autonomous).
Agent Visual Check: recorded by the final canonical headed run; unit results
alone do not establish a visual pass.
