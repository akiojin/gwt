# Issue #5390: native PR stocktake, slice B

The saved equivalent input localizes the long native operation to the local
stocktake. The four baseline calls completed successfully in 127.57–163.99
seconds, including a warm full PR cache and `include: []`. Each started 256 Git
processes: remote resolution, base verification, ref enumeration, 85 history
walks, 85 merge projections, and 83 projected source comparisons.

This establishes the dominant stage on the isolated input. The original
installed 9.145.0 operation was censored at 180.113 seconds with no stdout or
stderr. Its internal trace is unavailable. Its cache update and observed
`git diff` child are consistent with the measured stage, but do not prove that
it had only this cause. The separate Monitor `gh api pulls` deadline remains
separate evidence. Neither historical failure was rerun against production.

## Frozen input and native boundary

[Diagnosis](pr-list-b-diagnosis.json) preserves the full input manifest, 754
remote ref/OID pairs, explicit default hint, shallow history boundary, source
HEAD, baseline executable hash, cache/history bytes and hashes, complete
envelopes, Git argv/start/exit durations, and the executable harness source.
The baseline is checkout source `ce6606457af3f5270a5044a5b31831a39fb0199f`, built
before production edits. Git is `2.51.2.windows.1`.

The fixture is a separately owned repository under `target/pr-list-b-native`.
Its refs are frozen and its immutable object reads use the original object DB
as an alternate. All merge objects, caches, history, budget files and settings
remain inside the fixture. The shallow boundary is copied with the refs.
HOME and USERPROFILE are private to each case; GWT and GitHub authentication
variables are removed from each child. A native fake `gh` accepts only the
empty light PR list and an unavailable free rate probe; unrecognized calls
fail rather than contacting GitHub. No credential or execution authority is
copied.

The actual `gwtd.exe` receives a finite JSON input file, with EOF available
immediately, and writes stdout/stderr directly to files. Thus there is no
unclosed input writer or undrained output pipe in the harness. External
monotonic timestamps cover spawn, first stdout byte (100ms polling upper
bound), and process exit. Full JSON is parsed after exit. Git Trace2 captures
both raw startup `git remote -v` and logged Git commands.

Cold/warm names describe only the application PR cache. All four cases use
the same empty PR payload, refs, base and restored history. OS page cache,
host scheduling and other active work are uncontrolled. Timings are individual
observations, not a latency distribution, product SLA, or PM deadline.

| Baseline native case | Seconds | Git calls | Fake-gh attempts | Result |
| --- | ---: | ---: | ---: | --- |
| Default, warm | 163.994 | 256 | 0 | cache, 5 source / 80 bookkeeping |
| Default, cold | 127.575 | 256 | 2 | github fixture, same 85 rows |
| Include empty, cold | 133.535 | 256 | 2 | github fixture, same 85 rows |
| Include empty, warm | 135.021 | 256 | 0 | cache, same 85 rows |

All four inventories agree exactly on branch rows, ahead counts and source
classification. Cold calls report one budget-spending GitHub call; the second
fake process is the existing free rate probe. These are local fixture calls,
not measurements of real GitHub network latency.

## Same-input GREEN

[GREEN](pr-list-b-green.json) preserves the four native responses, complete
argv/timings, final source/test/native hashes, actual tracked CLI observations,
and the review regression's original failure. The measured source-file hashes
identify the implementation at the common baseline HEAD; producing commit,
canonical record and required CI bind delivery separately.

| Case | Before seconds | After seconds | Git calls before/after |
| --- | ---: | ---: | ---: |
| Default, warm | 163.994 | 29.136 | 256 / 23 |
| Default, cold | 127.575 | 29.355 | 256 / 23 |
| Include empty, cold | 133.535 | 16.723 | 256 / 23 |
| Include empty, warm | 135.021 | 22.150 | 256 / 23 |

Every after call exits zero and exactly preserves all 85 branch rows, ahead
counts, dates, owner fields and source classifications. Each performs six
batches of history/merge/diff work plus the five startup/base/ref reads. The
91.02% process reduction is deterministic for this input; elapsed reductions
of 76.99–87.48% remain single observations under uncontrolled host load.
Child Git durations are reported separately from the gaps between commands
and native total; they are not added up and mislabeled as exclusive operation
time. Three after observations still exceed the PM's 20-second boundary.

The actual tracked CLI regression first returned all eight candidate rows but
failed because it launched 26 Git processes. Its single-candidate observation
used five. GREEN compares candidate growth against that single-candidate
baseline rather than hardcoding the original six-process ceiling: eight
candidates now use the same seven calls as one. This preserves the original
RED condition while allowing the fixed additional base-tree snapshot read.
Existing required enumeration/count errors, optional unknowns, recovery,
complete empty stocktake, stale nulls and cold/warm provenance remain covered.
A simulated Git2.38 `--stdin` refusal preserves source classification through
the existing singleton helper.

Independent source review found and closed a base commit/tree snapshot race,
incomplete merge framing, and unchecked raw diff metadata. The framing test
first failed on a missing record terminator, then passed with the strict
parser. Final focused results are Git138 and CLI8 passing, with zero remaining
blocking source findings. Formal pre-PR and CI evidence are recorded on the
owner Issue and delivery PR; these measurements alone do not claim delivery.

## Diagnosis and chosen repair

`collect_unlanded_work_branches` enumerated candidates once, then restarted a
history walk, merge and source diff for each candidate, even on a warm PR
snapshot. The repeated Git startup and repeated graph/tree work dominated the
native interval. History saving and rendering occur after this local work.

The repair is scoped to the CLI collector. Cache memoization would introduce
freshness policy, and concurrent calls would introduce scheduling behavior.
Batching the same existing reads avoids both. The public singleton
`branch_has_non_gwt_changes` and its settlement runtime caller stay unchanged.

An initial prototype used the `ahead-behind` format atom. Independent review
found no supported Git minimum establishing its availability, so that option
was rejected before production implementation. The selected method uses
`rev-list --parents --stdin` with a frozen excluded base. Each head's reachable
nodes inside that returned graph are counted once, matching the old ahead
count. Graph boundaries outside the returned set are not counted. Required
base/enumeration/graph failures and malformed output remain errors.

Optional source projection uses the documented
[merge-tree batch format](https://git-scm.com/docs/git-merge-tree) and
[diff-tree batch format](https://git-scm.com/docs/git-diff-tree). Batches contain
at most 15 OID pairs, below 2KiB even with SHA256 OIDs, so the existing
write-before-read process helper can finish input before draining large
output. Source conflicts remain visible; conflicts confined to `.gwt` still
require a source comparison. Malformed or failed optional reads retain rows
with unknown source classification. Git 2.38 lacks `merge-tree --stdin`, so
only its explicit unsupported-option refusal uses the existing singleton
implementation for compatibility; no new Git minimum is introduced.

## Measurement limits and rejected harness attempts

No private spans separately time main/perf setup, PM runtime mapping,
DefaultCliEnv construction, cache load, history operations, or rendering.
Git command intervals and external native timings bound these stages but do
not assign their exclusive time. The fixture uses checkout cwd, not the
production PM runtime. The empty PR set does not exercise conflict hydration,
real network latency or API budget exhaustion; existing regressions retain
those contracts.

The first fixture attempt wrote CRLF into packed refs and alternates, causing
an unavailable-base error. The next attempt failed preflight before native
launch. A further attempt omitted the shallow boundary and was not equivalent
to the saved repository. These attempts are preserved and excluded from the
four valid baseline measurements. Ref and base/OID validation, LF-only Git
files and the copied shallow boundary precede the valid runs.

This slice provides no parent fleet/CPU/RSS, 60-minute or four-hour acceptance,
installed binary deployment, or production PM 20-second availability proof.
User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface; CLI collector only, singleton runtime
source classifier and frontend unchanged)

## Review and CI fixture repair

The original diagnosis, first GREEN and failed CI at d99585f1 remain above and
in their original JSON files. `pr-list-b-green-review.json` records the final
review-source hashes, a freshly built native executable and the same frozen
input across all four cases. Its common pre-commit HEAD is provenance only;
final producing-commit verification and CI are separate delivery evidence.

| Case | Baseline seconds | Review-source seconds | Git calls |
| --- | ---: | ---: | ---: |
| default-warm | 163.994 | 13.569 | 256 → 23 |
| default-cold | 127.575 | 11.892 | 256 → 23 |
| include-empty-cold | 133.535 | 11.879 | 256 → 23 |
| include-empty-warm | 135.021 | 10.359 | 256 → 23 |

Every one of the 85 branch rows, its classification and count matches the
baseline in every case. The stocktake spans six batches. These finite
measurements retain the previous limits and establish no PM20-second SLA.

The Git2.38 unsupported-option fallback now accepts translated diagnostic
wording while retaining exit129 and the literal `stdin` option. The existing
actual-entry fixture first failed for German wording and then passed. One
private batch-size constant and chunk zip remove repeated indexing. The
original eight-candidate fanout regression remains in place.

Coverage run40480351357 failed two existing fixtures before aggregation:
shard4 hit the 200ms synchronous operation deadline during cold warm-up,
and shard6 refolded a retained owned Start into a supposedly ownerless Work.
Coverage for that head was unmeasured, not below a measured threshold.
Only those test fixtures change: a scoped existing clock seam separates CPU
measurement from materialization/scheduler wall time while real transport
timeouts, CPU250ms and all assertions remain; ownerless state is seeded
consistently in aggregate/events and the first immutable root source.

Ordinary canonical intake immediately before the original full-history
comparison deterministically reproduces the old owner discrepancy and passes
after fixture repair. The first backup restoration preserved an older mtime,
so Cargo reused the RED executable; that failed attempt remains disclosed and
excluded from GREEN evidence. Updating mtime and actually rebuilding produced
the passing result. No production hook/Monitor or UI code changes.

Independent review of all four changed source files, including the explicit
intake synchronization, found zero remaining blocking findings. Required
same-HEAD verification, scoped CI coverage and merged/base inclusion remain
separate gates reported in the delivery PR and canonical owner sections.

## Early-exit stdin repair

The e4046d54a coverage run40490459273 passed both previous fixture failures,
but its actual CLI compatibility test failed at English `no-stdin`: Null
instead of true, with no singleton Git calls. The CI log does not record the
underlying IO error. Reading the shared stdin helper found that an early-exit
child could close stdin first, making `write_all` return BrokenPipe before
its exit129 and diagnostic were collected. Small batches prevent a full-pipe
deadlock but do not prevent this race.

A real-Git helper test with finite input larger than pipe capacity directly
reproduced the lost diagnostic, then passed after the narrow root repair.
Only BrokenPipe followed by a failed child returns the captured output;
a successful early exit still returns the write error. Other error paths and
all caller status checks remain. No capability probe, duplicate wrapper,
fixture stdin draining, new timeout or public API is added. Production blob
callers still reject failed status before parsing; Console record structure
is unchanged and failed-child stderr can be forwarded normally.

`pr-list-b-green-early-exit.json` keeps this RED/GREEN, the failed CI provenance,
five source hashes and a fresh native executable on the same frozen input.
Every branch field/count remains equal across all 85 candidates and four
conditions. The common pre-commit HEAD is provenance, not delivery evidence.

| Case | Baseline seconds | Early-exit-source seconds | Git calls |
| --- | ---: | ---: | ---: |
| default-warm | 163.994 | 11.843 | 256 → 23 |
| default-cold | 127.575 | 10.666 | 256 → 23 |
| include-empty-cold | 133.535 | 10.465 | 256 → 23 |
| include-empty-warm | 135.021 | 9.659 | 256 → 23 |

All prior limits remain: finite isolated measurements do not prove a PM20s
SLA, original installed180s root cause, API/merge-reconciliation remedy,
installed rollout or parent fleet/CPU/RSS acceptance. Final same-HEAD checks,
scoped CI coverage and merged/base inclusion are separate delivery evidence.

## Fatal merge isolation

Review PRRT_kwDOPLof2M6wpiMj exposed a second per-head compatibility gap.
Real Git 2.51.2 returned status 0 for a healthy bookkeeping head alone, but
fatal status 128 for either order of a mixed unrelated/healthy batch, with empty or partial
stdout. Treating that whole batch as unknown changed the healthy branch's
classification. The original singleton independently keeps the healthy head's
classification as false
and leaves the unrelated head unknown.

The existing bookkeeping test retains its original standalone assertions,
then adds just one unrelated root. That final test first fails with healthy
None versus Some(false), then passes with a narrow fatal 128 fallback to the
existing singleton. Ordinary exit 1, I/O errors and malformed output stay unknown;
explicit exit 129 with the stdin option retains compatibility with older Git.
This adds no timed retry,
scheduler, capability probe, new policy/API or runtime change. Partial fatal
stdout is not trusted. Independent source review found no blocking issue.

`pr-list-b-green-isolation.json` preserves this RED/GREEN/probe, five final
source hashes and a fresh comparison of four native runs on the same frozen
input. All
85 branch rows match every baseline field and count, and successful stocktake
still uses 23 Git calls.

| Case | Baseline seconds | Isolation-source seconds | Git calls |
| --- | ---: | ---: | ---: |
| default-warm | 163.994 | 15.480 | 256 → 23 |
| default-cold | 127.575 | 10.563 | 256 → 23 |
| include-empty-cold | 133.535 | 10.463 | 256 → 23 |
| include-empty-warm | 135.021 | 10.263 | 256 → 23 |

All previous evidence and diagnostic limits remain. Final producing-HEAD
verification, scoped coverage, review settlement and merged/base inclusion
are reported separately; previous source PASS is not reused for this repair.
