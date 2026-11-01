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

## Additional terminal identity read

The terminal observer first obtains fresh execution diagnosis, which already
loads the ECR and validates the generation ledger. It then performed another
`execution_state::load` for every linked window to populate
`session_matches_execution`. That load repeats pointer/ledger reads, JSON
parsing, ledger and generation hashes, snapshot validation and ECR parsing.
The close predicate consumes this identity field only for a `Completed`
diagnosis. Limit that additional read to `Completed`, preserving its fresh
integrity, Session, owner and status checks. Every original diagnosis still
runs; there is no new cache, TTL, timer or authority policy.

The focused identity-reader regression failed before the guard: an `Active`
diagnosis invoked the additional reader once when zero was expected. After the
guard, Active/Blocked/Missing/Corrupt invoke it zero times. Existing terminal
tests (13 including the new case) and startup/expiry tests (24), including
Completed predecessor rejection and actual Completed restore/Monitor settlement
and exact close, pass. A concurrent Active-to-Completed transition cannot
authorize close through the earlier Active diagnosis. Independent diff review
found no blocking issue.

`active-terminal-read.json` contains the complete read-only Rust command-path
probe, input record/ledger hashes, source Git blob identities and twenty
alternating paired rounds after warmup. Both variants use the same compiled
checkout debug library and unchanged valid Active authority with three
generations. Twelve identity checks require 12 additional ECR loads before and
zero after; their median elapsed time was 155.0712ms and 0.00025ms. The near-zero
branch time measures the omitted command path, not the whole observer. No
elapsed-time assertion is used. The production reader seam independently
checks the no-read behavior, while the existing Completed tests check safety.

A separate installed PID 33132 sample still observed 63.125 CPU seconds over
30.9839357 wall seconds (2.03735 logical cores), with working set 4,395,114,496
bytes. Its executable hash and sample time are retained separately in the
artifact. This is neither the checkout candidate nor a causal comparison.
The microprobe does not establish whole-GUI CPU, large-ledger scaling, real
eight-Agent sixty-minute budgets, four-hour memory or PM-prompt acceptance.
Parent AC-1/2/3 remain unchecked. Final fresh canonical and isolated checkout
headed dark/light results belong to the delivery evidence.

## Duplicate validation of a freshly loaded ledger

The terminal observer and Work projection call `diagnose_with_mode`. Its
`load_generation_ledger` call already validates the complete owner ledger,
generation hashes, snapshot JSON/integrity and history chains, then the
worktree pointer/projection. Immediately validating the returned immutable
ledger with `generation_ledger_integrity_ok` repeats that entire computation
without another read or freshness guarantee. Remove that guard and its
unreachable invalid-`Some` arm. Existing loader errors still produce a corrupt
diagnosis; every strict load, caller binding and close/restore predicate stays.

The existing actual-reader projection regression passed before changes. A
test-only thread-local full-validation counter added to its blocked case
then failed with 3 validations instead of the expected 2. The two strict
fresh loads are retained; only the third validation of the already loaded
ledger is removed. Existing Active/Blocked/Corrupt, no-Git, ledger hash and
history mutation, missing/malformed projection and terminal regressions are
included in final verification. The counter adds no production state and
does not depend on elapsed time. Independent diff review found no blocking
issue.

After the change the same actual-reader regression passed with exactly two
validations. The five existing hash/history and missing/malformed/stale-pointer
authority cases also passed before final canonical verification.

`duplicate-ledger-validation.json` retains the complete read-only Rust probe,
source/input hashes and twenty alternating paired rounds after warmup. Both
variants use the same debug library, unchanged valid four-generation authority
and twelve actual strict loads per sample. The old path additionally validates
each returned ledger; the new path consumes it directly. All initial/final
ledger fields and record/ledger hashes match. Additional validations fell
from 12 to 0, with median elapsed time 137.7083ms before and 95.7519ms after.
These shared-host command-path observations exclude the other diagnosis reads,
Session/Monitor work and GUI. They do not establish shipping CPU, large-ledger
scaling, eight real Agents for sixty minutes, PM-prompt failures or long-duration
memory acceptance. Parent AC-1/2/3 remain unchecked. Final fresh canonical and
isolated checkout debug headed dark/light results belong to the delivery PR.

## Reuse validated generation snapshots

Within `validate_generation_ledger`, the first generation loop decodes and
hydrates every immutable `execution_control_json`, validates its owner, status,
integrity, settlement timestamp and predecessor chain, then drops the parsed
record. Later the same function decodes and hydrates the same bytes again to
build `effective_projections`. Move the validated record into that existing
operation-local map after its original checks, and remove the second decode.
All fresh reads, hash/history validation, error priority and later lifecycle
and takeover projection updates remain in place. The map owns the records
earlier; no persistent cache or new authority API is introduced.

The existing actual-reader projection regression passed before changes. Its
test-only thread-local snapshot counter then failed with four decodes instead
of two. After the change it passes with two, while the separate full-validation
counter still proves two strict fresh validations. Existing hash/predecessor/
terminal mutation, projection-event truncate/reorder/mutation, contiguous
takeover writer/exact projection and missing/malformed/stale-pointer authority
regressions pass. Independent source review found no blocking issue.

`generation-snapshot-decode.json` retains the complete probe, source/input
hashes, both linked binary hashes and twenty alternating paired rounds. The
same probe source links against the actual before and after checkout debug
libraries. Both read an identical captured strict-valid five-generation ledger
and perform five warmup validations before measuring fifty real validator
calls. Process startup and input parsing are outside the sample. Every
validation succeeds and the complete ledger and fixture bytes stay unchanged.

Allocation/reallocation requests fell from 40,350 to 39,200 per fifty
validations (2.85%), and cumulative requested bytes from 10,221,000 to
10,180,200 (0.40%), identically in all twenty pairs. These are allocation
requests, not retained memory or RSS. Median elapsed time was 226.4727ms before
and 224.6686ms after; the overlapping samples do not establish a material wall
time improvement. No elapsed-time assertion is used.

A separate installed PID 33132 observation remains in the artifact with its
binary hash and sample times; it is an older executable, not this candidate or
a causal before/after comparison. The validator probe does not establish full
strict-load/diagnosis latency, whole-GUI or release CPU, real eight-Agent
sixty-minute budgets, PM-prompt failure rates or long-duration memory. Parent
AC-1/2/3 remain unchecked. Fresh canonical and isolated checkout debug headed
dark/light evidence belongs to the delivery record.

## Reuse complete linked-PR reads within one Monitor scan

The installed PID 33132 observation in `same-scan-linked-pr.json` used the
same older executable identified by its SHA256 and start time. Over
30.1424617 seconds it consumed 62.328125 CPU seconds, averaging 2.067785
logical cores. Several Tokio workers dominated the surviving-thread deltas.
This active shared-host interval is diagnostic, not controlled idle or a
causal measurement of this change. Exited workers are absent from the
endpoint thread accounting.

The bounded process/stage log shows recent `open-pr-readback` failures in
serial GraphQL timeline reads, after successful REST/cache inventory reads.
For example, the 01:19:35+09 scan spent 5,381 / 2,199 / 1,370 / 4,073ms on
successful timeline commands, then timed out its next command at 1,390ms.
The stage failed at 14,571.43ms under its existing budget. A later discovery
retained 30 completed Issues before failing; merge reconciliation then made
another bulk read and consumed 10.301 seconds. Those 30 Issue IDs and their
intersection with the later request are unobserved. The entire 10.301 seconds
is not claimed removable, and the Tokio CPU observation is not attributed
to this network fanout.

The source previously discarded complete timeline results after building
launch holds. Merge and claim completion then fetched their own timelines.
Carry only the existing observer's fully acquired results in the scan input,
and select missing Issues for subsequent bulk reads. Both daemon and GUI
local scan drivers pass that input. These read/proposal stages dispatch no
PR/Issue mutations; the existing effect executor applies those later.
Existing delivered-branch pruning between merge and claim remains separate
and unchanged. It does not change the already merged PR completion fact.
Each new candidate load starts a new map,
and cache fallback starts empty. This adds no persistent cache or freshness
policy.

The existing completion batch regression first failed because complete
Issues 1 and 2 were requested again: `[1, 2, 3]` instead of only missing
Issue `[3]`. After the repair it also verifies that a failed missing read
retains the complete merged and empty results, preserves the original failure
for the missing Issue without a single-Issue retry, excludes an unrequested
seed row, and performs zero fetches when every requested timeline is complete.
The existing readiness, merge intent, timestamp and deadline predicates are
used unchanged. These are same-input request-count results, not measured
network latency or shipping CPU improvements.

A separate in-process probe uses the same final debug library for both paths:
an empty-seed control reproduces the former data flow, and a complete-seed
variant reuses identical merged/empty results. After warmup, twelve alternating
pairs of 1,000 calls all produced the same two completion results. Fetch
callbacks fell from 1,000 to zero. Median elapsed times were 3.091600ms and
2.594200ms; overlapping samples do not establish a material latency improvement.
This is a data-flow control, not a pre-change-binary speed comparison, and
excludes startup, real network, GitHub parsing and the rest of the scan.
The complete probe source, executable/library hashes and raw pairs are in
the artifact. Merge-stage missing rows are not written into the initial map,
so those rows can still be read again during claim completion.

The artifact retains the RED source and executable hashes, raw failure and
GREEN evidence, the fixed input and final source hashes. Final canonical
verification, headed dark/light checks, required CI and base inclusion belong
to the delivery record. The initial discovery fanout timeout, installed
rollout, PM20s availability, parent CPU/RSS and real fleet 60-minute/four-hour
acceptance remain unproven.

## Empty execution observation (2026-11-01)

`read_execution_observations` used to collect the Session/OS inventory and
Git worktree list even when its input contained no Issues. Its eventual map
was always empty. Return that empty map before collecting those inputs; keep
the existing timing, nonempty diagnosis and caller's process-held-set update.
No deadline, cache, scheduler, schema or effect authority changes.

The focused regression failed with one Git spawn versus zero expected, then
passed after the three-line guard. Existing stopped-holder/reaper recovery
and blocked/dead-host release regressions also passed. The test directly
counts Git; source review establishes the census is skipped before the worker
is constructed.

The identical temporary native probe called the read-only function twelve
times against the real `E:/llmlb` and Session store. Before: 3463.9462ms total
wall time and 3.40625 probe CPU seconds. After: 0.0407ms and no observed CPU
ticks. The first before call was 938.4819ms; subsequent calls benefited from
warm process-local caches. These are probe-process measurements, excluding
child Git CPU, startup and the rest of the scan. Zero ticks means below the
observed resolution of `GetProcessTimes` for this short loop.

`empty-execution-observation.json` preserves the probe source, identical source
hash, executable hashes, per-call results, RED/GREEN output and limitations.
The baseline executable preceded the test-only addition; its recorded worktree
source hash includes that regression. The fixed library adds the guard.
The temporary example was removed after measurement.

Installed PID 38952 still consumed 51.265625 CPU seconds over 30.0667689 wall
seconds, about 1.705 cores. Sampled native stacks had no product symbols.
Recent execution-observation logs mix projects and daemon processes, omit
owner counts, and include nested stages. Neither those wall times nor this
empty-input probe proves a reduction in production GUI CPU or fleet budgets.

The managed-browser preflight returned `unknown` with `ledger_unavailable`
and `completion_missing` despite a current context and converged target.
Feedback Issue #6635 records the complete output. PM ruling `db103f30` directs
backend canonical verification first, followed by the original headed gate
after Issue #6622 is delivered. Parent AC-1/2/3 and overall delivery remain
unproven; this record does not replace headed, required CI or base inclusion.
