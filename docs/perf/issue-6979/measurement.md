# Issue #6979 refresh measurements

PM delivery boundary (2026-11-12 02:47 UTC): ship the current improvements as
one independently releasable slice and stop further measurement loops so the
pending application update can drain. Owner AC-2/3 remain unchecked. The
historical before-hour is complete; the matched after-hour is not run. The
latest normal native measurement is runner `be21d5a2`: Issue 5,901ms and File
6,508ms, above the original 5,000ms gate. The final runner `7c687948` also
contains the separately verified GC read grouping; its final native latency
and one-hour performance are unmeasured. This report does not claim full Issue
acceptance or change its thresholds.

The owner acceptance requires identical one-hour change traces, at most one tenth
of the before index writer bytes, after refresh p95 at most five seconds, and
actual Monitor scans and GUI actions meeting their deadlines during refresh.
Mechanism tests and untimed preparation do not satisfy those criteria.

The shared workstation is Windows 11 Pro build 26300, Core i9-14900K (24 cores,
32 logical processors), with approximately 128GiB visible memory. No hardware
or power settings were changed. The managed runtime is Python 3.14.0,
Chroma 1.5.5, NumPy 2.5.4, Torch 2.14.1, sentence-transformers 5.6.0 and
transformers 5.20.2. Other workstation activity is not controlled; index writer
counters are limited to the owned private index root.

The before runner is commit `2ec26e27c01e8fe51c99afffb0ff99f69ea4705e`.
The fixture source is delivered commit
`1107952d44c19acec69135565a511fa5ef3ae474`. Both profiles use the same frozen
5,889 real Issue/SPEC records and the same archived project. The cache capture
is immutable per file, with its capture interval recorded; a globally atomic
snapshot of the actively changing source cache is not claimed. Real E5 seed
vectors and file CAS preparation are outside both timed intervals.

The selected standalone fixture excludes `.gwt/work/**` operational history,
which the file index path policy already denies. Its 16,928 archived files are
preserved separately. All 2,017 canonical File records, exact embedding inputs
and CAS keys are identical before and after this selection (input manifest
SHA256 `5bd871cb463a454d9967b1c95bbc7cbb195c51afd07230ca569698e5472c1e39`).
The 5,889 Issue/SPEC inputs and 81 queued Monitor candidates are unchanged.
This avoids building 2,051 unrelated historical Work documents during target
preparation. Both profiles use selected fixture commit
`c407259a9098ae81ad71f558a8805a0d55a637fa`; their file bases must be published
against that commit before timing. The original archive, fixture commit,
excluded Git blobs and failed untimed startup evidence are retained.

## Byte boundary

`scripts/diagnostics/issue-6979-refresh.py` installs only through an explicitly
selected isolated `sitecustomize` directory. It records two separate categories:

- `python_file_write`: encoded bytes successfully passed to Python file writes
  below the selected index root, including JSON publication metadata.
- `chroma_logical_payload`: normalized float32 vector storage bytes, UTF-8
  IDs/documents/URIs and canonical JSON metadata passed successfully through
  Chroma's native upsert boundary for that root.

The second category measures logical application payload, rather than physical
SQLite/WAL/HNSW writes or filesystem amplification. ETW and physical OS write
bytes remain unmeasured. These categories are reported individually as well as
their application-writer sum; the sum is not a physical I/O total.

The self-test writes a known 1,048,576-byte file inside the root and the same
marker outside it. Exactly the inside write is counted. A real Chroma upsert
with that document, a four-dimensional float32 vector, ID and metadata counts
1,048,615 logical bytes. It reads the stored document/vector back. The diagnostic
captures paths, sizes, process IDs and timestamps without recording payload or
credential contents. Missing timed Issue/file payload instrumentation is a
measurement failure, rather than a successful zero-byte result.
The control also exercises a direct native upsert with Python list vectors;
the logical float32 size does not assume NumPy objects at that boundary.

## Fixed trace and responsiveness

The owned standalone Git fixture lives under ignored diagnostic storage; it is
not a development worktree. Thirteen trace events occur at offsets 0, 270, 540,
and every 270 seconds through 3,240 seconds. Each changes the same ordinary
Issue title and prepends a numbered header to the same tracked Rust file, with
fixed commit timestamps. Both profiles start from the identical fixture commit
and frozen cache. Unexpected record changes invalidate the matched trace.

The after profile uses the checkout GUI's normal broker and coordinator.
Before uses the historical public indexing runner with GUI background indexing
disabled. Its process wall time excludes Rust admission/claim overhead and is
reported with that explicit boundary. All raw terminal refresh outcomes,
continuations and failures are retained. Source-change-to-final-completion
includes the broker's 30-second quiet period and is reported separately from
claim elapsed time.

Each profile uses a fresh private HOME and private runtime/coordinator/worker
state. Installed dependencies and the offline real model cache are reused.
Automatic agent launch and PM startup are disabled. ScanNow invokes the actual
owned Monitor pipeline and durable state commit. Monitor control-to-observed
commit has a 60-second outer deadline. Actual native scan stage intervals,
rather than queued control time alone, establish overlap with refresh work.

Real headed Chromium runs dark and light themes sequentially for half the
measurement each. Native Monitor replies and actual palette clicks have a
one-second completion budget. Console/page errors and dropped UI trace entries
are failures. Native dispatch timing retains the existing 30ms diagnostic and
100ms watchdog boundaries. Agent-produced visual checks are separate from
`User Verification Result: n/a (autonomous)`.

## Root-cause controls

Windows resource background mode adds a 32MiB hard working-set maximum. Native
controls showed that changing memory priority alone leaves that cap and the
resulting page thrashing. Preserving the caller's exact minimum/maximum/flags
across entering background mode retains CPU Below Normal, very-low I/O and low
memory priority. A caller-imposed hard limit is preserved. Real E5 cold loading
completed in 17.988s; warm long-input encodes took 1.019–2.368s in that control.

The reused background worker retains at most one model graph. Keeping default
and pinned instances together reached 3.34GB RSS, exceeding the 2.5GiB logical
runner limit. Bounded eviction held RSS below that limit but repeated model
construction added 3.33–3.44s per revision change.

The final cache reuse requires proving the constructed model's identity. The
default request uses sentence-transformers' existing public `modules.json`
resolution route. Only a standard lexical Hugging Face snapshot path whose
commit equals the existing File model pin permits construction at that explicit
commit and reuse by both request keys. A local model directory, unknown path,
different commit or resolution failure keeps the original default constructor.
Merely observing an attribute or equal cache paths does not establish identity.
An untimed real-model control constructed one graph, then reused the same object
and parameter storage for pinned/default requests. It peaked at 1,971,572,736
RSS bytes; this load-only control does not establish encode memory or refresh
acceptance.

File refresh reuses immutable record verification from a compatible published
View. It checks actual native documents and metadata against their digests,
retains current source, membership and publication barriers, and fully verifies
new or repaired records. Missing collections or unknown proof fall back to the
existing parser/materializer repair path. Unchanged inherited records avoid
reparsing and unused CAS/vector reads; this is not a new unconditional storage
corruption audit. The existing whole-artifact warm path remains IDs-only.

The resident transport also preserves complete terminal replies. A Windows
regression showed a successful final send followed by a client connection reset
while the server closed a socket still used by its disconnect monitor. The
worker now shuts down sending, waits for the existing client-close signal with
bounded monitor cleanup, and releases the socket after the monitor stops.
Processing-time disconnect cancellation and the no-replay contract remain in
force. This follows Microsoft's [socket shutdown sequence](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-shutdown)
and [concurrent-close restriction](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-closesocket).

The prototype's slow initial builds, UTF-8 harness failure and invalid first
headed pilot are retained as failed diagnostics. That pilot's two Monitor scans
committed within 55.240s and 52.265s; 114 headed UI samples had no errors and a
172.663ms maximum. Its global log contains 4,060 timed native dispatch rows,
with a 34ms maximum. It does not prove refresh acceptance: an unfinished startup
works build held the private heavy lease, the diagnostic rejected a legacy
list-vector write, and live SPEC updates changed 302 chunks outside the trace.
The initial collector also omitted the private global log; a separate corrected
analysis preserves the original output and records those facts.

Frozen transport uses the checkout's existing debug-only loopback-v1 and native
`gh` shim test seams. It preserves captured authors, states and labels, supplies
nonempty queued candidates and runs the actual Monitor pipeline and durable
commit. Unknown PR inventories, timelines and label-event facts are unsupported,
rather than fabricated empty results. This is a replay of GitHub's boundary,
rather than a live GitHub scan. Premeasurement native roundtrips must preserve
the logical record fingerprints; raw cache bookkeeping changes are retained.
The frozen provider's two actual native roundtrips committed in 11.060s and
7.086s, without scan errors or changes to the original logical records.

Initial builds, startup settling, warmup and mechanism controls are outside the
timed interval and are not reported as one-hour acceptance results.

## Valid short pilot before the remaining latency corrections

The selected-domain pilot using runner SHA256
`47e9ae016d66b7ab494dff2ac0e8c6b706dcee2f18eb5bd731f84ef514312ef6`
completed both changed scopes without input drift or failed refresh claims.
It still failed the five-second gate:

| Scope | Native claim | Runner rebuild |
| --- | ---: | ---: |
| Issue | 11,304ms | 8,569ms |
| File view | 31,514ms | 28,920ms |

Source change to final observed completion was 104,912ms, including the
30-second quiet period and broker scheduling. Both writes came from the actual
private worker. The Issue upsert carried 4,487 logical bytes and the File upsert
5,386 bytes; each computed one vector. The File model reload added 3,165ms.
Its build-progress file was replaced 2,018 times for 612,362 bytes, including
unchanged CAS hits. These observations identify redundant work; they do not
establish the size of a speedup before remeasurement.

The same pilot committed six actual Monitor scans (maximum 7,126ms), recorded
63 scan-stage/refresh intersections and 12 simultaneous UI intersections.
Its 114 headed dark/light samples had zero errors, with a 258.536ms maximum
completion time. These short diagnostics remain separate from the required
one-hour comparison and final-head verification.

## Completed historical one-hour baseline

The historical runner completed all 13 events and 26 public indexing calls in
the measured window 2026-11-11 20:53:51.531–21:53:58.325 UTC (3,606.794s,
including the theme transition). Every indexing call succeeded. The final
fingerprint check found no unexpected Issue changes and only the specified
Rust file changed in the fixture.

| Application writer category | Issue bytes | File bytes |
| --- | ---: | ---: |
| Normalized Chroma payload | 406,716,973 | 137,521,137 |
| Python file writes | 10,521,691 | 25,247,074 |

The application-writer sum is **580,006,875 bytes**. The 26 historical process
wall times have nearest-rank p95 **143,895.0053ms**. They exclude Rust admission
and claim overhead. The original native GUI-watchdog failure remains recorded:
completed receive-to-completion measurements include SaveUiTrace at 101ms and
ListIssueMonitor at 113ms. Completed synchronous GUI callbacks are a different
boundary: maximum 491ms, with 656 completions at least 100ms (649 inbox and seven
prepared-list callbacks). There are also 500 unfinished-handler watchdog samples;
these may describe the same callbacks and are not added to the completion count.
The original `hour-analysis.json` field `native_dispatch_max_ms` used only
request-completion rows. [Corrected native timing](before-hour/corrected-native-timing.json)
keeps the boundaries explicit, without modifying the original summary, analysis
or logs. This baseline is not an overall responsiveness PASS.

The real Monitor committed 120 scans, with maximum observed control-to-commit
9,139ms. Headed dark and light each recorded 1,674 actions, zero console/page
errors, no one-second action overruns, and maxima 566.844/618.493ms. All 60 UI
trace chunks reported zero dropped entries. Native scan phases intersected
refresh work 1,642 times; 485 intersections also overlapped a sampled UI action.

The original coverage summary incorrectly omitted historical `issues.staging`
from its scope discriminator. Preserve that summary unchanged. A separate
read-only correction accounts for all 1,209 Issue and 26,221 File native upserts.
Each of the 26 action intervals contains exactly one writer PID and the exact
expected float32 vector count: 5,889 Issue vectors or 2,017 File vectors. All
timed native payload rows belong to those intervals. This is a classification
correction backed by existing raw records, rather than an inferred zero-byte
result or a rerun of the measurement.

The [baseline artifact manifest](before-hour/artifacts.json) lists each raw and
stored SHA256, size and compression. It includes all writer rows, native events,
per-action results, UI samples, scan controls/commits and overlap intervals.
To reproduce the byte total, decompress `writes.jsonl.gz`, select rows whose
`time_ns / 1e6` lies within `write-instrumentation`'s recorded window (also in
`hour-analysis.json`), and sum `bytes` by `category` and path. The sum includes
successful native upserts and accepted Python writes, with no inferred physical
write amplification. The correction proves the historical staging path and
action/vector coverage separately from the original overall FAIL.

## Remaining native latency diagnostics

The following short runs use the real model, the same frozen logical corpus,
normal native refresh claims and isolated headed Chromium. They are failed
diagnostics, not substitutes for the matched after-hour:

| Runner SHA256 prefix | Issue claim | File claim | Native callback maximum | Application bytes |
| --- | ---: | ---: | ---: | ---: |
| `98c1bcbf` | 6,991ms | 7,250ms | 33ms | 2,640,444 |
| `b284eaac` | 6,058ms | 9,107ms | 30ms | 2,640,444 |
| `be21d5a2` | 5,901ms | 6,508ms | 32ms | 2,640,444 |

These runs retain every terminal refresh claim and have no failed claims,
completed native callback overruns, unfinished-handler watchdog warnings,
console/page errors, unexpected source changes or dropped UI trace entries.
The `b284eaac` run committed five native Monitor scans, with maximum observed
control-to-commit 8,109ms. Its dark/light themes each completed 56 actions;
maximum completion times were 234.688/129.716ms. The original five-second
refresh gate still fails.

Private inclusive phase timers in the `b284eaac` run put 2,399.492ms of File's
8,596.167ms handler in its two fresh canonical Git checks. Timings overlap and
must not be added. A separate unchanged-File worker control recorded handler
times 6,432/6,201/4,697ms, with 3,479/3,570/1,843ms in Git processes. A read-only
Win32 query of the actual pilot worker confirmed CPU priority `0x4000`, I/O
priority zero, memory priority one and working-set flags ten; the former hard
32MiB working-set cap was absent. These observations motivate reducing
redundant Git process startup while retaining both source barriers. They do
not justify changing priority, the deadline or the accepted corpus.

The `be21d5a2` run combines the normal Git boundary probes and defers HEAD
existence checks to failed peels, reducing a normal canonical capture from six
Git processes to three. It retains ambiguous-path/error fallback and both
fresh source captures. It commits six Monitor scans, with maximum observed
control-to-commit 9,117ms, while still failing refresh latency.

GC diagnostics distinguish an unchanged public rebuild from the all-retained
path. The former rotates generations and can expose an orphan in the first
16-ID block: a control of the actual pilot database spends 471.433ms in GC,
including 203.889ms count, 221.269ms delete and one 1.959ms ID read. This does
not explain the all-retained path's cost. A separate locked control executes
the original GC on 5,889 retained records: 369 native reads, 1,925.755ms and no
deletions. A diagnostic buffer reads 128 IDs at a time while preserving the
original 16-ID checks: 47 reads and 570.835ms. Both see the same complete ordered
ID hash `4c6e0ab73f76bccc9d81caeecb0b337a7513333e22bdfaca4b8fc68982417e83`.
No model is imported. This tests the read-grouping mechanism, rather than
establishing the five-second or one-hour acceptance criteria.

## Windows Monitor connection investigation (2026-11-13)

The later `after-s7g-120` diagnostic reported a 4,176ms List response. Its native
events separate three boundaries: `ListIssueMonitor` returned in 0ms at
11:02:17.281 JST, the background reader reported `status connect timeout after
200ms` at 11:02:17.567, and `IssueMonitorListPrepared` completed in 1ms. The
measurement helper ignored the error notification and accepted the next periodic
status at 11:02:21.132 as the reply. Preserve the original failed diagnostic;
4,176ms is not evidence of a four-second synchronous GUI callback. The corrected
private helper records error notifications as request failures immediately.

An independent Windows accept-cancellation defect was reproduced and proposed
for separate ownership. Normal Monitor scans do not change daemon residency,
so that defect was not established as the historical timeout's cause and no
runtime change for it is included here.

## Admitted diagnostic and metadata source reuse (2026-11-13)

The unchanged delivered runner `7c687948` was measured with the fixed corpus,
real E5, normal broker and actual headed dark/light Chromium in a fresh HOME.
Canonical `verify.run` record `vrr-e79b5aa3a68140ca964eb89bb12683be` acquired the
normal exclusive admission; other managed heavy jobs waited. Optional native
function timers were enabled. This remains a failed diagnostic, not a matched
one-hour result or a controlled before/after comparison.

| Boundary | Result |
| --- | ---: |
| Issue claim / handler | 6,670 / 6,031.896ms |
| File claim / handler | 8,110 / 7,549.017ms |
| Timed application writer bytes | 2,640,444 |
| Actual Monitor scans / maximum control-to-commit | 5 / 13,121ms |
| Headed dark / light actions | 58 / 58 |
| Maximum headed action / errors / one-second overruns | 196.428ms / 0 / 0 |
| Completed native callback maximum / watchdog overruns | 42ms / 0 |
| Sampled host CPU mean / maximum | 34.75% / 61% |

The source trace has no unexpected changes, writer instrumentation is proven,
and the owned GUI and frozen provider were stopped. Unowned desktop activity
remained outside admission's control. The earlier direct `after-codex1-120`
run also overlapped diagnostic fixture enumeration; preserve it as a
contaminated diagnostic, not performance-comparison evidence.

The Issue's four payload verifications are necessary: initial and final
5,889-record checks plus the new record before and after its upsert. Existing
verified records are already passed to the writer. They are not four full
scans and are retained. File source capture and publication checks also remain.

A separate, proved duplication occurs before generation work: the ordinary
Issue and SPEC readers enumerate and decode the same `meta.json` files. They
now share raw metadata through one kind of the existing bounded source-revision
cache. Legacy sources remain fresh, and a revision changing during a read is
not cached. Body reads, manifest stats, raw SPEC numbers, normalization,
initial/final health and the 16-entry cache bound are unchanged.

The existing canonical-source regression fails with two reads per metadata
file before the change and passes with one after it. Source-cache, UTF-8 and
revision/publication regressions pass. Comparing closed-fixture outputs before
and after gives identical 2,101 documents, 138 SPEC/manifest entries and 5,889
records; the canonical record SHA256 is
`19e61c5ccffb9c8998b0834baa0c6747acbdddffe23dc4371e395f562c2014ea`.
This proves input equivalence and the eliminated read, not a latency speedup.
Owner AC-2/3 still require the original successful pilot, matched after-hour,
raw recomputation and delivery proof.

## Local client lifetime and HEAD probe diagnostics (2026-11-14)

Two further admitted native diagnostics preserve the original corpus, real
model and deadlines. Neither passes the five-second refresh gate or replaces
the matched 13-event after-hour:

| Verification record | Broker p95 | All native rebuild p95 | Timed application bytes |
| --- | ---: | ---: | ---: |
| `vrr-586dab2167e1455ca68f8d427672de01` | 9,429ms | 9,377ms | 2,641,725 |
| `vrr-635dc40371a844458053fb129b3e3855` | 5,706ms | 5,674ms | 2,640,444 |

The second run records Issue/File rebuilds of 5,674/5,649ms. Its six actual
Monitor scans complete within 9,121ms; 58 dark and 56 light actions have zero
console/page errors and no one-second overruns. The maximum completed native
callback is 36ms, with no 100ms overruns or unfinished watchdog warnings. Both
owned GUIs are stopped. These short observations do not establish AC-2/3.
Raw artifacts remain under `.gwt/6979-soak/after-clients-20261114-120` and
`.gwt/6979-soak/after-head-boundary-20261114-120`. Application bytes combine
instrumented Python writes and normalized Chroma payload; physical OS writes
remain unmeasured.

The collector now audits every native rebuild in addition to terminal broker
claims, because Issue rebuilds can finish outside a terminal broker claim.
Both boundaries retain the unchanged 5,000ms threshold; no timing is removed.

Deterministic regressions prove three local reductions: empty or entirely
inherited staging no longer opens unused vector readers; Files/Docs GC readers
share one helper-local native lifetime; a HEAD-only canonical capture uses two
Git processes rather than three. Remote canonical selection still uses three
and takes precedence over HEAD. Both fresh source captures, metadata/vector
verification and publication barriers remain. The combined HEAD probe accepts
only strict output and retains the original fallback on ambiguity. No process
cache, model, storage format, ranking, scheduler or priority changes are made.
Host-variable pilot timings are not a controlled causal speedup comparison.
