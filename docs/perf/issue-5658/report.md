# Issue #5658: idle GUI CPU

## Scope

This is a gwt runtime performance change, applicable to every opened project.
At intake, the work branch and fetched `origin/develop` both contained
`43f30e55a12dea2fa71ce4f5008896934494c03e`; there was no undelivered source.
The corrected comparison baseline is pristine develop
`7a61d9b2ca10747fa5b02cae321edd4fa8cab3b3`. The fetched base updates did not
change the six runtime/test files in this fix. PR #5674 also delivered the
zip 8.6 to 9.0.4 update needed for the required Cargo Deny advisory check;
that dependency prerequisite is part of the baseline. The pristine shipping
release baseline and its matching candidate comparison have completed.
The earlier debug baseline is diagnostic only. Memory ownership
remains with Issue #5561.

T12 corrects the performance build profile. The reported defect concerns the
installed shipping executable; the earlier checkout GUI CPU measurements used
unoptimized debug binaries (`opt-level = 0`). Those debug values cannot
establish a shipping-profile budget pass or failure. The CPU gate now requires
matching release-profile baseline and candidate binaries. The workload,
provider, cadence and thresholds remain unchanged. The required new pristine
release baseline is recorded below. Independent canonical debug headed
verification also remains required.
Both CPU binaries use `cargo build --release -p gwt --bin gwt --bin gwtd`,
matching the shipping Windows command in `.github/workflows/release.yml`
apart from its explicit native target triple. No profile or feature setting
was changed for this comparison.

### Relaunch audit, 2026-10-18

The relaunched owner is autonomous. After fetch, HEAD remains `7a61d9b2c`,
with zero unique commits and 162 commits behind `origin/develop`
(`b271696ad8a527e56b908d803e1b48072c5885c9`). The nine modified Rust files
remain undelivered. Within those paths, develop differs from the comparison
baseline only by making `observe_session` public; it does not contain these
optimizations. No owner acceptance checkbox is credited to the local source.

Independent read-only reviews found no concrete correctness issue in either
the nine-file patch or the measurement harness. The reviews checked snapshot
invalidation, Windows reparse fallbacks, fresh process identity/liveness,
Windows CPU baselines, broker epochs, legitimate launch authority, whole-GUI
CPU accounting, fixture/profile identity and headed reporter forwarding.
These reviews are not substitutes for measured CPU or canonical test results.
The previous candidate run stopped before its end snapshot and is incomplete;
the relaunch uses a new isolated HOME and the same recorded release binary,
fixture, provider and harness. Production timers, cache lifetimes, permission
checks and GUI layout remain unchanged.

## Deployed-process profile

The deployed v9.126.1 GUI, PID 31116, was observed from
2026-10-16 18:01:56 UTC for 30.0991041 seconds. `GetThreadTimes` measured
kernel plus user CPU, matching TID and creation FILETIME across samples.
The retained threads consumed 30.203125 CPU seconds, or 1.003 average cores.
This is a lower bound: threads that exited during sampling are omitted.

| Thread | TID | CPU seconds | Average cores |
| --- | ---: | ---: | ---: |
| main | 36336 | 7.046875 | 0.234122 |
| tokio-rt-worker | 18408 | 6.078125 | 0.201937 |
| gwt-monitor-runtime | 56248 | 5.953125 | 0.197784 |
| gwt-index-refresh-drain | 30608 | 4.000000 | 0.132894 |
| gwt-agent-capacity | 44456 | 1.843750 | 0.061256 |

[Raw thread snapshots](deployed-thread-cpu.jsonl) and
[existing dispatch-profiler rows](deployed-dispatch.json) preserve attribution.
The dispatch window ending at 18:02:59 UTC recorded 66 `AgentFrontend`
handlers above 50 ms, totaling 11,982.7143 ms, with p95 227.8122 ms and maximum
245.5223 ms. These are handler elapsed times, not thread CPU seconds.

The `Ready` agent request synchronously obtains snapshots for all project
panes. Its existing `Pane::snapshot_bytes` regenerated the same unchanged
terminal history on every request. This identifies repeated serialization
inside the measured path; it does not attribute every deployed CPU sample to
that serializer. System-wide ETW stack sampling was unavailable: WPR returned
`0xc5585011` (profile-system-performance policy), without elevation or a
change to the running production process.

## Change and focused evidence

Cache one serialized snapshot per Pane and discard it on output or successful
resize. The owned `Vec<u8>` API, output sequence, unfinished escape tail,
history limit and reconnect protocol remain intact. The additional retained
buffer has the size of that pane's latest snapshot and is dropped on mutation
or pane destruction.

The existing thread-local test allocator exposed 617 allocations for a second
unchanged 600-line snapshot. The regression now requires at most two, and
checks new output, an unfinished escape sequence, and resize without subsequent
output. All 97 existing and new gwt-terminal library tests passed.

The [paired serialization probe](snapshot-probe.json) uses 12 Pane parsers,
1,200 styled lines each, 40 rows by 120 columns, one warm snapshot, and then
60 snapshots per pane. Both variants returned 89,909,280 bytes. Elapsed time
was 18.603050 seconds without reuse and 0.003436 seconds with reuse. This
shared-host microbenchmark measures the repeated serialization, not the
whole-GUI CPU budget.

The remaining measured paths remove duplicate work at existing boundaries:

- Session census keeps directory-entry metadata for a regular sidecar's cache
  lookup. Symlinks and Windows reparse points retain target-metadata lookup;
  every cold read still restamps the path. Process liveness, cache eviction,
  uncertain rows and the all-sidecars-before-process-snapshot order stay intact.
- One index poll reads one broker view and fingerprints identical repo-source
  sets once. Worktree file lists remain temporary to one worktree. The next
  poll reads everything fresh; the broker's locked epoch reducer, discovery
  interval, quiet window and claim cadence are unchanged.
- Runtime health projects the refreshed known PIDs and root on normal ticks.
  Full discovery still runs every 60 seconds and CPU/memory updates every
  five seconds, with the same ancestry, focus, severity and heartbeat rules.
- On Windows, capacity retains the fresh complete process identity/executable census but
  refreshes CPU counters only for GUI roots. It consumes no CPU value from
  unrelated processes. This avoids Windows `GetSystemTimes` once per unrelated
  process, while retaining the GUI incarnation warm-up and sampling interval.
  Other platforms keep their original single CPU refresh: sysinfo on Linux
  advances process-time baselines even during metadata refresh, so splitting
  that path would underreport GUI CPU.
- Windows index snapshots reuse ordinary sibling-entry metadata within one
  fingerprint operation. Singleton, missing, unreadable and reparse entries
  retain the original no-follow path lookup. Digest construction, ordering,
  recursive errors and the non-Windows path are unchanged.
- Both read-only Session-ledger sweeps reuse ordinary Windows entry metadata.
  Reparse points retain `symlink_metadata`, as do startup's fresh candidate
  reads and all non-Windows paths. Parsing, negative-cache behavior and
  immutable snapshot ownership are unchanged.
- The private runtime memo and pruning set use hashed Path keys. Per-directory
  entry sorting uses file names with their common parent, and runtime lookup
  borrows the caller's existing Path. Candidate order and all retained stamps,
  errors and process probes remain unchanged.

| Regression oracle | Observed before the fix | Required after the fix |
| --- | ---: | ---: |
| Warm regular sidecar: extra path metadata probes | 1 | 0 |
| One index poll: broker reads / identical Issues+Specs fingerprints | 9 / 2 | 1 / 1 |
| Known-PID tick with only the root requested: reported process count | 218 | 1 |
| Capacity sample: unrelated test-process CPU counter initialized (ms) | 156 | 0 |
| Index snapshot: ordinary sibling path metadata probes | 1 | 0 |
| Warm ledger sweep: private / shared path metadata probes | 2 / 1 | 0 / 0 |

Focused GREEN verification passed:

- `cargo test -p gwt-terminal --lib`: 97 tests.
- `cargo test -p gwt --lib session_inventory`: 25 tests on Windows; the added
  Unix symlink-target case is delegated to required Rust CI.
- `cargo test -p gwt --lib index_worker::tests::source_poll_detects_edits_in_each_registered_worktree -- --exact --nocapture`:
  one test, including the 1/1 read counts and fresh next poll.
- `cargo test -p gwt --lib index_worker::tests::reports_refresh_observes_both_durable_sources_and_coalesces_noop -- --exact --nocapture`:
  one existing Reports compatibility test.
- `cargo test -p gwt --bin gwt runtime_health_poller::tests -- --nocapture`:
  13 tests, including root-only projection, ancestry/focus, severity and budgets.
- `cargo test -p gwt --lib agent_capacity::tests -- --nocapture`: 27 tests,
  including the new Windows non-GUI CPU regression. The 25 census tests also
  passed again after this change (164 focused tests across the changed areas).
- `cargo test -p gwt-agent --lib session_ledger::tests -- --nocapture`:
  eight existing tests passed, after the two extended warm tests reproduced
  path-read counts 2/1 instead of zero. This includes changed/deleted/malformed
  files, prior immutable versions and startup's fresh success read.

The [index metadata TDD record](index-metadata-tdd.json) retains the subsequent
one-path-probe RED failure and both GREEN refresh tests. A later fetch of
`origin/develop` at `e1d040bebc3856a810a0470b7f9ac9798546c0e2` found no changes
to any of the seven affected Rust files relative to the comparison baseline.

The index oracle also requires fresh reads on the next poll. Existing broker
coverage already rejects older epochs without changing their snapshot or
quiet deadline; this change does not add a parallel concurrency protocol.

## Isolated GUI method

The owner-specific browser-check helper accepts `-BuildProfile debug|release`
(default `debug`) and uses the matching checkout binaries. Shipping CPU
acceptance requires `release` for both baseline and candidate; the TS check
validates the executable profile path, and the retained verifier requires
matching release records. The helper uses fresh HOME, a private copy of 14,841
Session files (36,009,778 bytes), and the normal background index path. PM
auto-start and Issue Monitor launch are disabled.
The copied fixture is never linked to the live Session store. Its manifest
hash, executable hashes, relevant source hashes and exact owned process IDs
are recorded in `launch.json`.

Headed Chromium launches 12 custom command Agents through the regular Launch
Wizard, with real Sessions and launch-issued capabilities. Their Node providers
seed 600 styled history lines and use their own checkout CLI to make authenticated
`pane.read` requests, phase-spread at 66 requests/minute. These exercise
`AgentFrontend::Ready` and its project-wide snapshot reply without a paid model
or copied caller credentials. The same 12 live Sessions continue across themes.
Whole-process `TotalProcessorTime` and a
monotonic clock determine average cores, including short-lived threads.
Per-thread snapshots supply attribution. No user input or terminal output is
allowed during the measured window; marker input before and after must reach
both raw output and xterm. Console/page errors must be zero in both themes.
The retained verifier compares visible input p95 with the corresponding
baseline, allowing 500 ms for shared-host scheduling variation; every probe
must also complete within 15 seconds.

The pristine shipping release baseline, run
`before-08365bda0f83409aa003b71ca2f2d68f`, completed both headed themes at
`7a61d9b2ca10747fa5b02cae321edd4fa8cab3b3`. Its `gwt.exe` SHA-256 is
`76129bb736c279a5b2ed265d6b800a52aea986084ad503573e27d7b8e1d84b13`.
All twelve committed intake groups were ready, the same twelve real Agent
Sessions continued across themes, each pane held 600 history lines, and the
fixed workload offered 66 authenticated reads/minute. Both themes had zero
idle terminal bytes, zero IPC failures, 24 successful input probes and empty
console/page error lists. Both actual theme screenshots were inspected. The
baseline did not assert the CPU budget:

| Release baseline | Elapsed seconds | CPU seconds | Average cores | Completed reads |
| --- | ---: | ---: | ---: | ---: |
| Dark | 1800.0988174 | 671.359375 | 0.372956956 | 1976 |
| Light | 30.0676047 | 12.109375 | 0.402738267 | 31 |

The release [launch manifest](before-launch.json),
[cleanup](before-cleanup.json), [dark measurement](before-dark.json),
[light measurement](before-light.json), [dark errors](before-dark-errors.json)
and [light errors](before-light-errors.json) retain the measured identities.
Owned GUI PID 46700 started at `2026-10-17T01:41:12.7608746Z`; it and owned
daemon PID 3076 were confirmed exited at `2026-10-17T02:29:30.0142491Z`.
These baseline results are compared with the completed candidate below.
Canonical verification and delivery are recorded separately on the owner Issue;
local measurements do not check off acceptance before develop contains the fix.

### Completed release candidate

Run `after-51b0f128dfff477f94ee4779854c97a6` completed both headed themes on
2026-10-18. GUI PID 35236 retained its creation identity throughout, used the
same release image `e0d3b1be1d725b5669e5b1b1e148fbbbf77dbec9de9a109fdc1af6ae5176be81`,
and sampled only after all twelve new worktrees had committed intake snapshots.
No debugger, dump capture or local build/test ran during the measured interval.

| Release candidate | Elapsed seconds | CPU seconds | Average cores | Completed reads |
| --- | ---: | ---: | ---: | ---: |
| Dark | 1800.0558258 | 284.578125 | 0.158094055 | 1973 |
| Light | 30.0455285 | 4.515625 | 0.150292747 | 28 |

The thirty-minute result is 57.61% below the paired release baseline and meets
the unchanged 0.2-core budget. Both themes had twelve quiet Agent PTYs, zero
IPC failures, 24 successful input probes and zero console/page errors.
Visible input p95 improved from 1023.02 to 132.72 ms in dark and from 287.41 to
126.67 ms in light. The same twelve Session identities continued across themes.
Both candidate screenshots were inspected with no display regression observed.

The [launch manifest](after-launch.json), [cleanup](after-cleanup.json),
[dark measurement](after-dark.json), [light measurement](after-light.json),
[dark errors](after-dark-errors.json) and [light errors](after-light-errors.json)
retain the measured data. GUI PID 35236 and daemon PID 34188 were confirmed
exited at `2026-10-18T20:11:24.5785025Z`. `node docs/perf/issue-5658/verify.mjs`
passed both complete records, including the source, executable, profile,
fixture, workload, throughput, quiet-output and input-response comparisons.

In the candidate dark interval, surviving-thread deltas include Monitor
76.34375 CPU seconds, capacity 47.859375, index refresh 38.015625 and namespace
GC 24.265625. These are attribution diagnostics; the whole-process total above
includes exited threads and is the acceptance metric. The mandatory independent
debug headed run remains part of the fresh canonical pre-PR matrix.

The [release baseline thread summary](before-threads.json) attributes the
largest surviving-thread deltas during dark sampling:

| Thread | CPU seconds | Average cores |
| --- | ---: | ---: |
| gwt-monitor-runtime | 202.390625 | 0.112433 |
| gwt-agent-capacity | 134.843750 | 0.074909 |
| gwt-index-refresh-drain | 104.468750 | 0.058035 |
| main | 52.578125 | 0.029208 |
| runtime-namespace-gc | 28.109375 | 0.015615 |

All common threads total 627.093750 CPU seconds, below the whole-process
671.359375 seconds because exited or unmatched threads are omitted.
Whole-process CPU remains the acceptance metric. The retained verifier checks
both baseline theme records against their release launch identity and native
executable SHA before using their input p95. It permits source differences
only for LF/CRLF line endings; executable and provider hashes remain exact.

The pristine `7a61d9b2c` debug diagnostic completed both headed functional tests with
all twelve committed intake groups, the same twelve real Agent Sessions in
both themes, zero terminal bytes during sampling, zero IPC failures, 24 input
probes per theme, and empty console/page error lists. Its debug CPU readings
exceeded the numeric 0.2-core reference, which was not asserted; they do not
assess the shipping-profile CPU criterion:

| Theme | Elapsed seconds | CPU seconds | Average cores | Attempts / completed reads |
| --- | ---: | ---: | ---: | ---: |
| Dark | 30.0842647 | 22.890625 | 0.760883646 | 33 / 30 |
| Light | 30.0966718 | 19.812500 | 0.658295380 | 32 / 32 |

The preserved debug [launch record](diagnostic-debug-before-launch.json),
[cleanup](diagnostic-debug-before-cleanup.json),
[dark measurement](diagnostic-debug-before-dark.json),
[light measurement](diagnostic-debug-before-light.json),
[dark errors](diagnostic-debug-before-dark-errors.json) and
[light errors](diagnostic-debug-before-light-errors.json) are diagnostic only.
They retain executable, fixture and source identities with private paths
redacted. The `before-*.json` comparison records now contain the completed
shipping release baseline; these saved debug copies remain diagnostic.

The debug diagnostic's [completed-run timer audit](baseline-timer-drift.json) contains 560 scheduled
intervals after actual timer start, excluding bind warm-up. Mean spacing was
10,933.1375 ms against the nominal 10,909.090909 ms: 0.220427% slower, with
p50 10,919 ms and p95 10,973 ms. Quantization and host scheduling were not
separately attributed. At that mean, the fixed provider would offer about
4.35 fewer requests than the nominal 1,980 over 30 minutes. The test-only global
attempt-count assertion of nominal plus/minus two was therefore removed from
the TS test and retained verifier. The baseline had passed that stricter check.
For that oracle correction, provider code, period, phase, wrapper and CPU
sampler were unchanged. The
per-Agent success minimum `floor(elapsed / period) - 1`, aggregate success
minimum `floor(66 * elapsed / 60) - 12`, zero failures, input/output and CPU
requirements remain. The verifier requires matching provider, wrapper and
sampler hashes; the baseline's original TS hash remains recorded, while the
candidate's TS hash must match its current validation-only revision.

The earlier Shell-only unmodified checkout binary built at `43f30e55a`
measured 0.526285 cores
in dark and 0.564371 in light (30.1048593 and 30.5373401 seconds).
Both headed functional tests passed; these debug CPU values are diagnostic
only. The retained
[launch record](diagnostic-shell-before-launch.json), [cleanup](diagnostic-shell-before-cleanup.json),
[dark measurement](diagnostic-shell-before-dark.json), and [light measurement](diagnostic-shell-before-light.json)
fix the workload and executable identity. Both error lists are empty.
Consequently, the serialization microbenchmark alone is not acceptance
evidence for the GUI CPU budget.

The historical Shell dark diagnostic attributed 3.44 CPU seconds to Monitor runtime,
3.14 to capacity, and 2.42 to index refresh, alongside the GUI dispatch work.
Source inspection found repeated work in those paths: census enumeration
discarded directory metadata and then restatted every sidecar; an index poll
reloaded the broker for each scope and walked identical Issues/Specs sources;
known-PID health ticks copied and classified the retained full process table.
The follow-up changes reuse those observations within the existing operation,
without extending polling intervals, cache lifetimes or process authority.

This controlled workload is not an observation of twelve real AI providers
under the production PM/Monitor. It does not establish performance for agent
output, network work or the separate eight-agent/60-minute fleet criterion
owned by Issue #5385.

## Diagnostic exclusions and fixture correction

The Shell-only measurements above lack representative agent IPC. They also
start with a cold Work projection, so they do not establish steady idle
performance. They are retained as diagnostics, rather than substituted for
the twelve-Agent acceptance.

The first expanded fixture accidentally cloned all source branches:
620 remote refs, including 599 `work/*` refs. A 30-second diagnostic measured
0.908385 cores in dark and 0.959300 in light, but cold Work intake was still
unfinished after 258 seconds. Its unnamed worker consumed 18.765625 CPU seconds
in the dark interval. All functional assertions passed; neither interval is
idle-budget evidence.

Four normal/thread-info dumps from owned GUI PID 36172 were decoded offline.
The [sanitized diagnostic](native-setup-diagnostic.json) retains exact process
identity, same-process survival after each capture, thread CPU deltas and
function/source locations. All four unnamed-worker stacks were Work rebuild
(`content_fingerprint` and `load_pending_sources_for_rebuild`). Capacity and
runtime-health stacks reached `GetSystemTimes` through sysinfo CPU refresh.
These are setup observations, not a sampled-time attribution of idle CPU.
Work ingest/intake remains with Issue #5561.

An earlier live LLDB attempt crashed the debugger with `0xC0000005`, produced
zero complete samples, and could not prove detach. That diagnostic run is
excluded. The successful captures used `MiniDumpNormal | MiniDumpWithThreadInfo`
and offline decoding after a disposable sleep-process validation.

The corrected helper seeds a private bare origin inside the fresh HOME from
only the recorded source branch, preserving its HEAD and excluding foreign
work refs. The fixture project is a shared clone of that private origin, with
the normal wildcard fetch mapping. Normal gwt Launch creates the twelve
fixture branches/worktrees and pushes/fetches only against the private origin.
Before launch, the helper checks the matching HEADs, one seed branch, one
project origin tracking branch, and an effective push URL equal to the private
origin. The symbolic `origin/HEAD` is not counted as a branch. Existing fixture
branches from the earlier direct-origin diagnostics are left unchanged in the
source checkout. Both baseline and candidate must use this same isolated
workload, clone SHA, provider hash, wrapper and CPU sampler; the validation-only
TS oracle correction is described above.
Sampling also requires the authoritative intake index's current projection
version, source-list fingerprint and committed snapshots for all twelve new
worktrees, with a maximum 30-minute preparation wait. Full mode waits 31 seconds
after the eleventh Agent's seed/read readiness before the final ordinary Launch,
crossing the production 30-second ingest throttle so that launch queues all twelve.
The index is written
after successful intake. Sampled performance logs are diagnostic only: their
self-budget governor may thin completion rows. Ordinary incremental polls
remain included in the CPU observation.

Run the helper with `-BuildProfile release -SampleMilliseconds 1800000 -AssertCpuBudget`
for the shipping-profile 30-minute dark measurement; light
receives a separate 30-second check. A short release diagnostic uses
`-BuildProfile release -SampleMilliseconds 30000`. Both accept an explicit
private `-SessionsFixture` directory and forward Playwright arguments.

Canonical headed functional verification independently uses
`-BuildProfile debug -FunctionalOnly` and a short sample. `FunctionalOnly`
requires the debug profile; release binaries cannot replace this mandatory
headed gate. It retains the same twelve real Agent Sessions, 66 authenticated
requests/minute, before/after input probes, zero terminal output, both themes
and console/page error assertions. It observes the current intake state once
without waiting for cold Work ingestion, records readiness as not requested
and false, and labels its measurement `functional (includes startup)` with
`budget.asserted: false`. Combining this switch with `-AssertCpuBudget` is
rejected. This record establishes the functional checks only; it cannot
establish AC-2/AC-3 idle CPU acceptance. The separate steady-idle 30-minute
measurement and retained Node verifier remain required for that acceptance.
Each test allows the requested sample duration plus 45 minutes for setup.

User Verification Result: n/a (autonomous)

## Debug steady-state diagnostics

The six-source candidate's 30-second diagnostic measured 0.493851 cores in
dark and 0.369227 in light. Main-thread CPU fell to 1.328125/1.125 seconds,
but Monitor, index and capacity remained significant. These debug diagnostics
are not shipping-profile CPU acceptance.

The subsequent five-minute [dark diagnostic](diagnostic-steady-dark.json)
measured 111.218750 CPU seconds / 300.081 seconds = 0.370630 cores; its
[light diagnostic](diagnostic-steady-light.json) measured 0.400586 cores.
The [launch identity](diagnostic-steady-launch.json) and
[owned-process cleanup](diagnostic-steady-cleanup.json) are retained. All twelve
worktree groups were committed before sampling. Four safe minidumps were
taken during dark sampling, so the entire instrumented interval is excluded
from CPU acceptance. Both functional theme checks passed.

The [offline native summary](native-steady-diagnostic.json) identifies active
Monitor directory enumeration, Usage ledger `symlink_metadata`, and hook-audit
Git process creation. Capacity and index were sleeping in all four captures;
these samples cannot attribute their complete CPU time. Every successful
capture validated the exact owned image and creation token and confirmed the
same process survived. A late additional capture was skipped before opening
the process. No live debugger attachment was used.

For the five-minute interval, surviving-thread CPU includes Monitor 23.0625 s,
index 17.984375 s, capacity 16.015625 s, main 11.21875 s and namespace GC
7.125 s. These lower-bound thread deltas are diagnostic and include capture
effects; whole-process CPU remains the acceptance metric.

Standalone probes of the fixed historical fixture disprove PID-probe
memoization as a useful fix: 7,133 direct calls cost 6.51 CPU ms per sweep,
versus 7.81 ms with a per-sweep memo. Opening and fully enumerating 2,647
runtime directories costs 113.28 ms, excluding sorting and metadata. Loading
143 potentially live Session TOMLs costs 39.06 ms and their canonicalization
3.91 ms. None individually explains the whole residual census cost, and
early historical pruning would also affect recovery uncertainty ordering.

An exact-path container comparison over 5,698 fixture keys found locked tree
lookups at 54.688 ms versus hashed lookups at 11.719 ms; constructing the seen
set and pruning cost 42.969 versus 22.656 ms. All returned values and sums
matched. This motivates changing only the private runtime memo and prune set,
with external candidate ordering and all file stamps unchanged. The registered
index and ledger changes reuse Windows directory-enumeration metadata within
one operation; they introduce no cross-poll freshness extension. Their full
workload CPU effect remains unproven until measured on the final source.

The retained [directory-order comparison](directory-order-diagnostic.txt)
also checks all 9,327 ordered paths and their metadata. Using cached file names
for sorting entries with a common parent and reusing the caller's existing Path
reduced directory-only CPU from 182.031 to 135.938 ms per round (20 rounds).
This does not remove any directory enumeration. The comparison is a single
ordered microbenchmark on a shared host; its savings are not added to other
microbenchmarks to claim a whole-process result.

For these debug diagnostics, the wrapper and provider were byte-identical to
the debug baseline. The additional ledger source was bound to the measured
executable by a supplemental pre-launch SHA-256 manifest. T12 adds explicit
profile selection and validation to the wrapper, TS check and verifier, so the
new release baseline must use that same updated harness as its candidate.
The provider workload and CPU sampler are unchanged. The retained verifier
also checks the supplemental manifest's timestamp, executable hash and current
source hash.

The [candidate shared-ledger probe](shared-ledger-candidate-diagnostic.txt)
returned the same 2,877 sorted Session IDs in all thirteen loads. After one
cold load, twelve warm sweeps averaged 29.95 CPU ms and 38.05 wall ms. This
uses the freshly built normal library; an older-library comparison was not
claimed because its dependency features could not be proven identical.

The seven-file debug candidate's passive interim sampling measured 0.532952
cores over 106.717 s, followed by 0.443501 cores over 30.087 s. These remain
debug diagnostics rather than a shipping-profile budget determination. The
[passive thread records](diagnostic-seven-threads.jsonl)
retain the latter interval. Before taking any native dumps, the
[conversion record](diagnostic-seven-conversion.json) excluded the **entire
run** from acceptance. The [launch manifest](diagnostic-seven-launch.json)
and [owned-process cleanup](diagnostic-seven-cleanup.json) retain its source,
binary and process identities. The wrapper was intentionally aborted; this
run has no completed thirty-minute acceptance result.

The [sixteen-dump native summary](native-long-diagnostic.json) retains every
capture and decode outcome. Each capture checked owned GUI PID 13432, native
creation FILETIME `134366688416646545`, start
`2026-10-17T00:00:41.6646545Z` and the matching executable path, then verified
same-start survival. All sixteen captures were diagnostic normal minidumps
with ThreadInfo; decoding was exclusively offline. Fifteen PDB decodes
completed. Sample 2 crashed the offline decoder after 42 thread rows; its
partial result and successful index PC/module unwind plus symbolizer fallback
are explicitly retained. Raw dump memory, PDBs, arguments, environment and
private paths are not published. Observed stack counts are not CPU
percentages, and capture pauses exclude these measurements from acceptance.

Index samples 0 and 1 entered `dunce::canonicalize` through
`index::path_policy::relative_path` during built-in path filtering. This still
canonicalized both root and file for each input before the optimized metadata
snapshot. The earlier metadata-read counter did not cover these policy opens,
which explains the missed work. Sample 2's fallback also resolves path-component
comparison in snapshot sorting. Capacity samples resolve both sysinfo process
metadata refresh and `TargetWalk::advance`; Monitor samples resolve process
resource capture and runtime directory enumeration. These point samples do
not assign all remaining CPU to those functions.

The same owned capacity snapshot contained 22 live Session records: twelve
fixture agents and ten copied Session records whose other worktrees remained
live. `refresh_projects` includes every machine-inventory worktree root, so
its target walk was not limited to the twelve fixture agents. The captured
walk does not identify its visited root. The existing shared scan budget can
also leave later roots marked in progress without visiting them; that status
alone is not evidence of repeated scans of all fixture targets.

T11 now batches built-in path filtering: it canonicalizes the root once per
call, freshly canonicalizes each file, and uses one batch call from the index
poll. The [paired root-normalization probe](index-policy-root-diagnostic.txt)
preserved exact ordered results while reducing only this repeated root work;
it is diagnostic rather than whole-process evidence. The existing baseline
had six passing policy tests, the added regression failed on the missing batch
method, and all seven applicable Windows tests then passed. The Unix symlink
retarget test is outside that Windows run and remains for CI validation. The
179 unique focused passes at that stage were collected across implementation
steps. The subsequent completed release comparison above measures the final
T11 source. Fresh canonical verification and develop containment, rather than
these historical focused runs, govern delivery and owner acceptance settlement.

Diagnostic cost estimates for main-thread JSON serialization (about 0.031
cores), GC deduplication (0.011) and base64 encoding (0.003) do not
justify adding further code caches. These costs are not combined into a
whole-process savings claim and do not establish shipping-profile acceptance.
