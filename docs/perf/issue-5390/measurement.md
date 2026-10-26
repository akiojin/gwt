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

Headed verification also exposed repeated same-size native PTY resize calls.
Compare the requested full `PtySize` with the existing native master's
`get_size` result before resizing. Query errors retain the original resize
path; real changes and externally changed master sizes remain observable.
This avoids ConPTY screen repaint output on unchanged geometry without
adding a size cache or changing frontend snapshot recovery.

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

## Headed fixture prerequisite

The first canonical run, `vrr-f835dfd9025a478cb85387a9b713a573`, passed the
Rust, format, Clippy, naming, legacy and hygiene commands. Both headed themes
failed before their first Agent launch with `No idle Agent member is available`.
The historical twelve-Agent fixture registered its custom command provider
but did not allocate fixed-tool members required by the current normal Launch
contract. This is preparation failure, not a CPU or responsiveness result.
Console/page error arrays were empty; owned GUI PID 18704 and daemon PID 716
exited. The failed run and traces remain in the verification record and
`crates/gwt/playwright/test-results/issue-5658-after-6e989044bc31418f8afb2babc6e1515c`.

Before the first normal launch, use the existing frontend roster read/add
operations to allocate twelve enabled custom-tool members in the disposable
project. No Session, capability or roster identity is forged. Keep all existing
twelve-pane, authenticated IPC, quiet output, input responsiveness, CPU budget
and theme/error assertions. The naming change remains the single predicate.

The next run, `vrr-da5607ae39fc4f08b6bbd5e64b7cdeee`, exposed an omitted
`expected_revision` in that preparation helper. The backend correctly rejects
member mutations without the freshly read revision; the helper waited for a
success snapshot instead. Pass the current revision exactly as the application
frontend does. Owned GUI PID 18508 and daemon PID 24460 exited; both failed
theme traces remain under `issue-5658-after-994f14d8fc804142b936a6e50e7afd6e`.

## Same-size native resize

The third canonical run, `vrr-d49e495013f448d0883800186933da94`, successfully
launched twelve genuine custom-tool Agent Sessions. Input probes reached the
visible terminal in roughly 150ms, but quiet sampling failed: dark received
one approximately 1,910-byte viewport repaint per pane, and light received
two. These were ordinary `terminal_output` events, not `terminal_snapshot`
messages. Provider code writes no stdout during sampling. Both-theme failure
data and traces remain under `issue-5658-after-09679aa765534b9a96da93a1ebc4933a`;
owned GUI 49656 and daemon 32884 exited.

The dependency path is snapshot/viewport geometry reconciliation to
`update_terminal_grid`, `Pane::resize`, then an unconditional native
`MasterPty::resize` call. Reapplying the same grid makes ConPTY repaint its
visible screen. Keep snapshot recovery and all quiet assertions; skip only
a native resize whose existing master reports exactly the requested size.

The existing native `pty::tests::test_resize` now covers initial/same-size
requests, a real size change and an externally changed native master size.
Its existing native-resize completion traces were 4 before the guard (RED)
and 2 after (GREEN), with the final actual master size restored to 120×48.
All 7 existing resize/parser/manager tests passed. No mock or dependency was
added. The full canonical matrix now also includes these focused crate tests.

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

## Single-pass live Work dirty scan

The next bounded slice retains live settlement checks. The terminal observer
and Work projection reach `live_settlement_diagnosis`; a saved `PathDirty`
receipt invokes `work_event_dirty_paths`. That reader previously started
`git status` and a separate ignored `git ls-files` on every call. Returning
saved facts or adding a cache would change freshness and Issue #5072's
clean-tree recovery contract, so neither is part of this fix.

Use `status --ignored=traditional --untracked-files=all` to obtain both ordinary
and ignored entries. An actual Git probe confirmed that `traditional` lists
ignored files recursively with `--untracked-files=all`; `matching` collapses
the ignored parent and cannot preserve the existing shard checks. Classify
`!!` only as an ignored canonical immutable shard, retaining the existing PR
delivery certificate exemption. Ignored legacy logs, noncanonical names and
writer temporary files remain excluded. The tracked/untracked state parser,
Git error handling, caller cadence and terminal-close policy stay unchanged.

The new actual-reader regression uses Git's trace file, two ignored canonical
shards and excluded legacy/noncanonical/temporary files. Before the fix its
path/basis assertion passed, then its one-process assertion failed with the
expected two built-in commands (`status`, `ls-files`). It asserts one fresh
Git operation after the fix. Its final structured trace counter filters by
the fixture's canonical worktree, excluding unrelated parallel tests' Git
processes. Existing settlement and checkout/owner
diagnosis regressions cover delivery, corrupt-index rejection and stale-clean
recovery; the implementation also retains both certificate checks.

`single-pass-scan.json` preserves twenty alternating paired command-path
measurements after one warmup per variant, using the same disposable fixture:
256 ignored canonical shards, two excluded ignored files, and a tracked legacy
log with staged and unstaged changes. All 258 path/state/basis rows matched.
Git starts per scan fell from 2 to 1; elapsed median was 223.67245ms before and
56.68125ms after. Raw samples, exact command vectors, Git version,
base SHA and source hashes are retained. These are shared-host observations
under concurrent build load, without a scheduling-sensitive timing assertion.
The paired harness measures the command path; the actual-reader regression
independently checks the production function. No certificate is synthesized
by the timing fixture.

This slice does not establish the parent 60-minute fleet CPU/RSS/timeout or PM
prompt criteria. The earlier 56 observed Git lifetimes are a normal-load lower
bound, not an exact scan count, and halving calls within this reader does not
imply halving whole-GUI CPU. Final canonical regression and isolated checkout
headed dark/light evidence are recorded with the delivery PR.
