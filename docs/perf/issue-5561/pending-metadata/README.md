# Issue #5561: pending Work metadata retention

This slice shares the group and shard portions of pending source keys and stores
freshly generated SHA-256 fingerprints as 32 bytes. Persisted keys and fingerprint
strings remain byte-for-byte compatible. Discovery, recovery checks, source
context, payload ownership, and the existing store format keep their semantics.

## Same-input allocation comparison

[measure.ps1](measure.ps1) extracts the actual `PendingWorkEventsSource` types
and private helpers from the delivered baseline and candidate source. The latest
repeat used `564effe5e`; its product types are identical to `53ce8f078`.
[probe.rs](probe.rs) links the existing core key parser, constructs the same
24 groups / 8,000 shards / 192,000 aliases, and checks every restored identity
through the same ordered SHA-256 digest. It uses a counting System allocator.

| Requested allocation | Before | After |
| --- | ---: | ---: |
| Pending state, including interner | 51,760,016 B | 20,782,392 B |
| Peak during construction | 51,760,320 B | 20,873,560 B |
| After pending state/interner drop | 128 B | 128 B |

The retained-state reduction is 30,977,624 B (59.85%). Both digests are
`3864751e2a177f0e6a04a760cc5d7c097d5c4d5d3c58ba15b8eaf5b494fc7f4a`.
The final 128 B are the two live digest strings. The interner's allocation and
the larger inline representation are included in the after result.

[comparison.json](comparison.json) records source/type/binary/dependency hashes
and compiler identity. Run from a built Windows checkout:

```powershell
pwsh -NoProfile -File docs/perf/issue-5561/pending-metadata/measure.ps1
```

This isolates metadata retention with shared empty content. It excludes source
discovery, actual payloads, projection folding, persistence, allocator headers,
and OS/native commitment. It proves neither a whole-route peak nor a GUI budget.
The local probe writes disposable generated sources and binaries under `target`.

## Regression evidence

The inherited [RED](red.log) was a non-sharing `SourceKey::capture` scaffold that
failed the alias-sharing assertion. [Its provenance](red-provenance.json) describes
that scaffold; it is not a failing compile against the old type names.
[GREEN](green.log) reruns all 54 existing ingest tests, including recovery on
missing/changed sources, old fingerprint state, incremental/full equivalence,
and allocation ownership. The existing loader test verifies shared allocations
are transferred; the new representation test checks Windows/ref key restoration,
shared suffixes/groups, exact SHA restoration, and rejection of arbitrary/upper
case stored fingerprints.

The related intake tests also passed in nextest (57 tests, retries disabled) and
in the serial libtest harness (57 passed, two diagnostic tests ignored). An
earlier same-process parallel libtest run had one logging-capture failure
(`issue_5380_containerless_warning_is_once_per_event_across_sources_and_rebuilds`,
expected two warnings, captured zero); that case passed alone and in both runners
above. No core source changed. The hygiene gate passed 37 tests, with its
maintenance helper ignored. Final verification uses the repository's prescribed
nextest process isolation.

## Recovered GUI baseline

The earlier 30-minute interrupted investigation is distinct from the subsequent
[completed one-hour baseline](baseline-20261109/summary.json). The latter was
recovered from the fixture's durable LocalApplicationData Temp directory, which
differs from this session's process TEMP. It contains 63 allocator snapshots,
698 OS samples, four deterministic PTYs, and a successful owned-process cleanup.
The fixture reports checkout `53ce8f078` and 726 source worktrees. Its requested
budget was 2 GiB, not the original Issue's 1.5 GB budget.

| Baseline snapshot | 30 minutes | 60 minutes |
| --- | ---: | ---: |
| Exact requested Rust live bytes | 167,663,704 | 238,371,675 |
| Sampled ingest-owner live bytes | 41,119,488 | 87,438,336 |
| Sampled Work payload live bytes | 16,997,472 | 35,500,896 |
| Process Console git ring message capacity | 338,720 | 338,720 |

The original symbol map resolves the growing key/fingerprint clone stacks to
`work_events_ingest.rs:686` and `:670`. Sampled stack weights are estimates of the
largest 80 stacks, not an exact heap census. The metadata increase is a confirmed
owner within the observed ongoing scan; it is not attribution of all production
GUI growth to one owner or proof of a permanent leak.

[Provenance](baseline-20261109/provenance.json) records original file hashes and
the recovery limits. The old binary hash and complete temporary allocator source
are unavailable after target cleanup. The recovered fixture records the committed
checkout, but does not fingerprint its entire uncommitted source. Do not describe
this baseline and the new observation as a controlled matched comparison.

## New observation method

[observe.ps1](observe.ps1) reuses the owned checkout/fresh-HOME fixture with four
deterministic command PTYs, real headed Chromium and the checkout's Work history.
It records source and binary hashes. For the diagnostic build only, include
[live-allocator.rs](live-allocator.rs) from `main.rs` under Windows/non-test cfg
and call `issue_5561_allocator::start()` at main entry. Remove both lines before
ordinary product verification. `observe.ps1` supplies the required environment
variable and preserves runtime JSON, screenshots and cleanup on every exit.

The diagnostic counts successful requested Rust bytes and records sampled live
stacks every minute. It samples initial requests below 65,536 bytes at 1/64 and
all larger initial requests; realloc retains its original stack and sample
weight. Its 16,384-entry fixed stack table never grows. It reports overflow and
unresolved symbols. The 16-byte-or-larger headers, 3,670,016-byte static table,
native/JS heaps and allocator/OS overhead are outside requested Rust totals.
Reporter allocations are included; concurrent totals and stack tables are read
successively. Symbol resolution, headers and sampling affect timing/commitment.

The older sampler, base, repository history and shared-host load differ. Report
observed MB/hour for each run without claiming their difference is this patch's
causal GUI effect. The same-input product-type probe above supplies the controlled
allocation comparison. `analyze.py` uses ordinary least squares over the 5–60
minute observation window, excluding cold startup, and preserves separate Rust,
working-set and private-byte rates. MB means 1,000,000 bytes.

## Completed after observation

[The new capture](after-20261110/summary.json) contains 64 allocator snapshots
over 3,780.684 seconds and 697 dark-theme OS samples, all for the recorded GUI
PID. Source/binary hashes and the allocator archive hash are in
[metadata.json](after-20261110/metadata.json). The checkout was `564effe5e`, with
732 source worktrees. The private product representation is the same one tested
by the controlled probe. Both temporary product instrumentation lines were
removed after the owned GUI and producers stopped.

| GUI process-age snapshot | Before requested Rust live bytes | After requested Rust live bytes |
| --- | ---: | ---: |
| Startup sampler point | 770 | 7,104,321 |
| Approximately 30 minutes | 167,663,704 | 109,719,157 |
| Approximately 60 minutes | 238,371,675 | 143,999,376 |

Startup points are successive concurrent reads, not comparable clean-process
baselines: the new sampler resolves executable symbols in-process. All selected
snapshots reported zero attribution-table overflow. Symbol-resolution failures
remain visible in the after summary; they do not affect exact requested totals.

| Observed OLS rate, 5–60 minutes | Before MB/hour | After MB/hour |
| --- | ---: | ---: |
| Requested Rust live bytes | +163.125 | +80.013 |
| Working set | +206.585 | +96.032 |
| Private bytes | +220.456 | +114.020 |

These are observations of two ongoing cold ingests, not a matched causal GUI
comparison or steady-state rates. The new run still grows. Its dark maximum WS
is **591,417,344 B**, versus **408,805,376 B** in the recovered baseline; the
different diagnostic/native overhead prevents interpreting absolute WS as a
product regression or improvement. The same-input allocation probe, rather than
the difference between these rates, proves the metadata reduction.

The actual headed workload passed dark **3,600.006894 seconds** and light
**60.007610 seconds**, with four verified producers at both ends of each theme,
zero browser console/page errors, and all sampled WS below 1,500,000,000 B. Light
maximum WS was 583,884,800 B. The [cleanup receipt](after-20261110/shutdown.json)
confirms test exit 0, owned GUI stopped, and no cleanup errors. This is a
four-command-PTY observation, not the original real-model 10–12-window fleet.

The latest production PM series recorded roughly +200 MB/hour private growth
on v9.155.2 (Issue comment 6732148226). Its workload, binary and HOME differ;
neither the phase-local 30,977,624-byte reduction nor the observed +114.020 MB/hour
diagnostic series establishes a causal change to that production series.

The original Issue's real-model fleet, four-hour WS plateau, and same-condition
MB/hour criteria remain separate. No synthetic pane run checks those criteria.
Temporary allocator/main/hub diagnostics are removed from product code before
delivery; the ordinary product receives its own fresh canonical headed run.
