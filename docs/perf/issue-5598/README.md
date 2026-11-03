# Issue #5598 search measurements

The corrected native warm-worker workload **passes: 12/12 distinct searches, p95 1,993.92 ms**, against the fixed 1,622-Issue corpus and real model. The comparable baseline delivered 2/12 at p95 13,552.08 ms. Earlier corrected-source attempts missed the target and are retained, together with cold-worker failures. Fresh canonical verification and delivery status belong to the PR's Verification Run Record.

The CI follow-up relocates the provided-input regression into the existing JSON-envelope test module to keep `cli.rs` below its 1,000-line limit, and makes two test environment-lock acquisitions recover from poisoning. These edits affect only existing `cfg(test)` modules. The [source audit](ci-test-only-source-audit.json) records the reference commit, current file hashes, and identical normalized production prefixes for all three affected Rust files. Canonical verification checks that equivalence against Git and preserves the original measured full-source hashes; it does not relabel the existing latency measurement as a new run.

## Delivery review: retain only immutable Issue readers

Delivery review found that generation directory names alone do not prove immutability. The generic repo-scope incremental indexer updates its active generation in place. With Chroma 1.5.5, a retained reader observed the updated document text while its vector ranking remained stale; closing and reopening the client restored the correct ranking. The dedicated Issue indexer always stages and publishes a replacement generation, including incremental builds.

Reader retention is therefore limited to Issue collections. Memory, SPEC, Board, discussion, Work, report, legacy, and file-index readers retain request-scoped ownership. A focused regression first failed because the mutable-generation client was not closed, then passed with the narrower retention condition. Existing tests continue to cover Issue-reader reuse, replacement cleanup, and its finite limit. This preserves source freshness without adding a new invalidation protocol or changing publication.

The original 2,748.71 ms measurement below retains its executable and source provenance. It is not relabeled as a measurement of the later reader-scope correction.

The first corrected-source rerun is retained in [recovery-saturated/after-search-burst.json](recovery-saturated/after-search-burst.json): all 12 warm requests succeeded, but outer client p95 was **5,722.28 ms (fail)**. CPU was 100% before and after; free RAM fell from about 49.23 to 12.55 GiB. The native `gwtd:search` maximum was 3,214.82 ms and is also above the target. Its warm-stage maxima include admission 2,312.11 ms and runner transport 368.33 ms; the retained Issue-reader index-read mean was 3.90 ms. These nested durations are not additive. Saturation was observed, not established as the sole cause; the outer wall boundary and the 3,000 ms threshold remain unchanged.

## Corrected-source standard workload

The final unchanged-source standard rerun uses the original driver, all 12 distinct queries, the copied real source/index tuple, native Rust JSON clients, and inherited launch priority. Its frozen executable SHA-256 is `5ed8e1a1c145bc28810075004612ae41225663d14c614e9d3a0d4a6d618905a0`; [build provenance](recovery/build-provenance.json), [comparison](recovery/comparison.json), and [raw outcomes](recovery/after-search-burst.json) identify the measured source and boundary.

| Phase | Baseline successful / sent | Baseline p95 ms | Corrected successful / sent | Corrected p95 ms | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| Cold worker burst | 0 / 12 | 29,503.85 | 1 / 12 | 17,574.85 | Fail |
| Warm resident worker | 2 / 12 | 13,552.08 | 12 / 12 | 1,993.92 | Pass |
| Forced cold-worker recovery | 1 / 12 | 27,271.15 | 1 / 12 | 16,862.08 | Fail |

Explicit prewarm took 535.44 ms because the preceding cold burst had already loaded the model for its one successful request; the following Rust warmup took 267.60 ms. Cold failures were not folded into the warm result. CPU was observed at 51% before and 54% after, with about 13.28 / 19.03 GiB free RAM. This is an observed comparison, not a controlled-load causal speedup or a guarantee under arbitrary saturation. The two corrected-source failures remain visible.

The [post-measurement audit](recovery/fixture-after.json) confirms 1,622 documents, unchanged native TTL/metadata/manifest/pointer, unchanged corpus/vector tables, and no repair/failure guards. Administrative `acquire_write` rows may grow. The source tree remains the fixed 9,119 files with SHA-256 `05f16c59e2a15083e7278f843f1f6b1d827d1bc4f4e3d438e9e214041b7a2220`. Successful responses are still stale with refresh queued; no metadata was edited to report freshness. The [warm-stage summary](recovery/stage-summary.json) retains the actual 12 samples separately from startup.

## Workload and measurement boundary

The primary workload uses all **1,622 cached Issues**, their real text and labels, and a real semantic index. The fixed source also contains **125 SPEC owners / 2,882 SPEC records**; those are measured separately as a source-read component, not included in the primary Rust search scope. The unmodified source snapshot has 9,119 files and SHA-256 `05f16c59e2a15083e7278f843f1f6b1d827d1bc4f4e3d438e9e214041b7a2220`. [Fixture provenance](fixture.json) records the fixed memory source, original generation pointers, and native indexing output.

The usable Issue index was prepared by copying the existing real generation, then running the native incremental indexer. It reused 1,520 vectors and embedded 102 changed/new Issues, publishing all 1,622 documents. The original full-reembedding attempt was abandoned because a 16-document batch took approximately 47 seconds. A subsequent SPEC preparation failed after the shared dependency environment became unavailable; that failure is preserved in `fixture.json`. SPEC/memory preparation is not claimed as successful.

Each burst starts 12 fresh `gwtd` processes at a barrier and sends the canonical JSON `search` operation with `scopes:["issues"]`. This exercises the real Rust admission, search/repair, resident-worker transport, and real model. Wall time runs from client process creation through response and process exit. The reported p95 is the nearest-rank value over **all 12 requests**, including failures/timeouts; with 12 samples it is the maximum. A pass requires 12 successful responses and p95 at most 3,000 ms.

The model is `intfloat/multilingual-e5-base`, cached revision `d128750597153bb5987e10b1c3493a34e5a4502a`, with native 768-dimensional vectors. Dependencies are Chroma 1.5.5, sentence-transformers 5.6.0, portalocker 2.10.1, and Torch 2.13.0. A private launcher/venv imports installed dependencies through read-only `.pth` paths; runtime state, indices, coordinator, and worker endpoints stay inside the owned HOME. The runtime venv root is not a junction to production. [Dependency provenance](private-dependency-provenance.json) and [real model/Chroma probe](private-real-model-probe.json) retain the actual versions and module paths. Fake embeddings are disabled and model loading is offline.

These initial bursts use the same query, `search admission index sharing repair performance`. [queries.json](queries.json) provides the 12 distinct ordinary owner-search queries used by subsequent measurements; the harness accepts it through `--query-file`. No result or query-vector cache was added to make repeated identical queries pass.

## Initial before/after results

| Attempt | Executable SHA-256 | Runtime runner SHA-256 prefix |
| --- | --- | --- |
| Baseline, source `0b087f0b79dd5578dacadea51165caf467e43598` | `c42374da5699166bec520ecd94eea412176a1be9b802154eb987cf7585cfcafa` | `2d0b4f30825a10ed` |
| Cache-only attempt, before query QoS changes | `7f6bacbf4fb2a954ebd5df3dda6c5c72308089b96fad6d15712850e516ae0b94` | `04eaf92932fa8080` |

Each executable was copied to a stable checkout-owned path before measurement. The harness verifies that materialized runner bytes occur in the measured executable. Metadata includes executable/runner hashes and source digests; working-tree Rust digests alone do not prove compiler input correspondence.

| Phase | Baseline success / sent | Baseline p95 ms | Cache-only success / sent | Cache-only p95 ms |
| --- | ---: | ---: | ---: | ---: |
| Cold worker burst | 0 / 12 | 30,772.79 | 0 / 12 | 30,138.20 |
| Warm resident worker, 12 fresh clients | 3 / 12 | 15,939.00 | 9 / 12 | 11,422.65 |
| Forced cold-worker recovery stress | 0 / 12 | 29,619.44 | 0 / 12 | 29,759.61 |

The explicit real-model prewarm took 49,879.76 ms before and 40,576.40 ms after. The following single Rust warmup took 4,811.23 / 1,688.74 ms. Those preparation values are recorded separately; failed cold requests were not relabeled as warm successes. The cold phases exceed Rust's search deadline while loading imports/model state.

A normal GUI restart can leave the detached resident worker alive. The warm-worker phase models a restart launch wave under that condition. The forced termination phase is a stronger cold-worker recovery stress and is not asserted to happen on every GUI restart. The CLI benchmark does not measure browser rendering or GUI process startup.

The baseline warm responses were fresh. By the cache-only attempt, the index's native 15-minute TTL had elapsed: successful responses reported `stale_scopes:["issues"]` and `refresh_queued:true`. The source bytes were unchanged. This difference is retained explicitly, and neither attempt is a pass. Host load was not fixed: the after attempt observed 76% CPU before / 72% after on 32 logical CPUs, with approximately 17.6 / 14.6 GiB free RAM.

Full protocol outcomes, errors, exact durations, worker cleanup, and provenance are in [baseline-search-burst.json](baseline-search-burst.json) and [after-search-burst.json](after-search-burst.json). Successful response text remains in the owned local HOME; committed evidence keeps its hash, result count/scope, and health flags rather than publishing unrelated work-memory content.

## Comparable baseline with 12 distinct queries

A further [baseline measurement](comparison-baseline/baseline-search-burst.json) uses the same `queries.json` as the query QoS attempt and the next after measurement. Its fresh separate HOME copies the successful 1,622-Issue source/index tuple byte for byte from the final fixture. [Copy provenance](comparison-baseline/fixture.json) records all generation, pointer, metadata, manifest and source hashes. The previous final HOME was read only during preparation.

The [reuse audit](final-snapshot-reuse-audit.json) verifies the complete source hash, native TTL, document count and absence of repair/failure guards. Its SQLite physical hash differs from the original published file: the [read-only table comparison](final-snapshot-sqlite-audit.json) finds only administrative `acquire_write` rows changing from 20 to 24. All corpus, embedding, collection, FTS and migration tables match, as do all HNSW files. This physical difference is retained explicitly; no metadata was edited to restore freshness.

| Baseline phase, 12 distinct queries | Successful / sent | p95 ms | Result |
| --- | ---: | ---: | --- |
| Cold worker burst | 0 / 12 | 29,503.85 | Fail |
| Warm resident worker, fresh clients | 2 / 12 | 13,552.08 | Fail |
| Forced cold-worker recovery stress | 1 / 12 | 27,271.15 | Fail |

The static baseline executable remains `c42374da5699166bec520ecd94eea412176a1be9b802154eb987cf7585cfcafa`, source ref `0b087f0b79dd5578dacadea51165caf467e43598`, and its embedded runner remains `2d0b4f30825a10edd9b3b8aa8ec0c9f25bf8e39f5f7ee0954b72981f4be93581`. This driver's SHA-256 is `770f86af5b5e5335bd5cd107becd4fe05ac92e769ba9ded1cbb81b85f77c8bef`; source provenance now also captures the coordinator and CLI initialization files. Working-tree digests remain informational. Read-only CIM confirms the baseline worker and launcher inherited BelowNormal (`16384`).

Explicit real-model prewarm took 60,493.99 ms, followed by a 3,495.59 ms single Rust warmup. Warm successes were stale with refresh queued, using the unchanged native TTL. [The stage summary](comparison-baseline/stage-summary.json) selects the recorded warm UTC window: admission averages 9,072.70 ms over 12 samples; two successful runner samples average 2,264.52 ms for index reading and 3,122.41 ms for encoding. Failed-request samples are not filled. Host load was observed as-is: 91% CPU before and 57% after on 32 logical CPUs, with approximately 21.57 / 21.22 GiB free RAM. No controlled-load causal improvement ratio is claimed.

## Intermediate query QoS attempt: all responses delivered, latency unmet

The next attempt uses all 12 distinct queries in `queries.json` and the PM-approved successful snapshot in a separate HOME. Its static executable SHA-256 is `3a28d7b25cc5c2dc9bdc257bbc0845a957bbd8846546a94738aa9e94f56cb6e0`; its embedded runner SHA-256 is `efd8f94c0c9113ceb8d244a6c11586e643ece8d2a0cd1acfa33363aa359d4914`. Query QoS uses one CPU thread and Windows Normal priority. Read-only CIM snapshots confirm the actual worker and its Python launcher are both Normal (`32`) before and after the warm burst.

| Phase | Successful / sent | p95 ms | Result |
| --- | ---: | ---: | --- |
| Cold worker burst | 0 / 12 | 29,171.92 | Fail |
| Warm resident worker, 12 distinct fresh clients | 12 / 12 | 3,700.45 | Fail: exceeds 3,000 ms |
| Forced cold-worker recovery stress | 1 / 12 | 19,073.93 | Fail |

Explicit prewarm took 22,850.19 ms and the single Rust warmup took 427.41 ms. All warm responses were successful but stale, with source unchanged and refresh queued. The native Rust route's maximum was 3,003.56 ms; `gwtd:search` was 3,004.04 ms. Neither is rounded into a pass. The outer client maximum of 3,700.45 ms also includes process startup/exit; its mean exceeded the native `gwtd:search` mean by 720.50 ms.

The warm burst spans 3,710.27 ms. All clients start within a 1.13 ms interval after the barrier. The first response completes at 1,338.44 ms and the last at 3,707.22 ms. Recorded runner transport totals 2,455.70 ms, including query encoding 2,077.35 ms, index reading 73.70 ms, and querying 218.33 ms. Admission averages 1,449.38 ms and reaches 2,748.59 ms. These overlapping waits are not added to elapsed time.

The failed attempt is preserved in [query-qos/after-search-burst.json](query-qos/after-search-burst.json), with exact query strings, startup values, CIM process evidence, source/runner provenance, and the raw [perf log](query-qos/after-perf-2026-10-16.jsonl). [Its stage summary](query-qos/stage-summary.json) separates the exact warm UTC window from cold/startup samples. That intermediate attempt delivered 12/12 while missing the latency target.

## Complete changes: native warm workload passes

The final static executable includes source snapshot reuse, retained immutable-generation readers, single-query CPU/priority QoS, storage-lock probing before busy-lane fairness scans, and lazy CLI repository initialization. Its SHA-256 is `95e2cbc8328cde57f7c7ecbad43224029ad2a32e91a41320802adeb7ea7751ab`. The embedded runner remains `efd8f94c0c9113ceb8d244a6c11586e643ece8d2a0cd1acfa33363aa359d4914`, verified as actual bytes in that executable. [Build provenance](complete/build-provenance.json) and [the measured protocol record](complete/after-search-burst.json) retain source and binary hashes, including the same driver hash used by the comparable baseline.

| Phase, same 12 distinct queries | Baseline successful / sent | Baseline p95 ms | Complete successful / sent | Complete p95 ms | Complete result |
| --- | ---: | ---: | ---: | ---: | --- |
| Cold worker burst | 0 / 12 | 29,503.85 | 0 / 12 | 29,835.63 | Fail |
| Warm resident worker, fresh clients | 2 / 12 | 13,552.08 | 12 / 12 | 2,748.71 | **Pass** |
| Forced cold-worker recovery stress | 1 / 12 | 27,271.15 | 1 / 12 | 26,706.46 | Fail |

[comparison.json](complete/comparison.json) checks identical distinct-query lists, Issue scope, driver bytes and corpus conditions. Every warm response returns three Issue results. The [post-measurement audit](complete/fixture-after.json) confirms 1,622 documents, unchanged native TTL/metadata/manifest/pointer, matching corpus tables and no failure guard. Responses remain stale with refresh queued; the approved published snapshot is not relabeled as fresh. The baseline has its own HOME while the final measurement reuses the original final snapshot HOME.

Explicit real-model prewarm took 55,981.39 ms, followed by a 368.38 ms single Rust warmup. Cold import/model work is excluded only from the explicitly warm phase and its failed burst remains visible. Read-only CIM confirms the actual warm worker and launcher both use Windows Normal (`32`); ownership-checked cleanup stops the benchmark workers. This remains a warm-worker restart-wave measurement, not a claim that a fully cold application/model launches within three seconds.

The [exact warm stage summary](complete/stage-summary.json) records all 12 samples:

| Stage | Mean ms | Maximum ms |
| --- | ---: | ---: |
| Admission | 1,121.87 | 2,213.66 |
| Runner transport | 177.47 | 246.72 |
| Runner index read | 5.61 | 16.01 |
| Query encoding | 141.88 | 203.95 |
| Chroma query | 23.91 | 44.48 |

Nested stages overlap and are not summed. Host load was observed as-is, including concurrent checkout build/lint work: 65% CPU before and 82% after on 32 logical CPUs, with approximately 23.85 / 19.18 GiB free RAM. These are observed comparisons rather than a controlled-load causal speedup claim. The focused manual TDD regression runs reported GREEN for 54 coordinator tests and 34 CLI tests. Fresh canonical checks and the final headed run are recorded in the PR Verification Run Record.

## CLI startup-only components

[cli-startup-components.json](cli-startup-components.json) uses the same static executable, isolated HOME/environment, and checkout without model or index operations. Each command runs once sequentially and then in a 12-process barrier burst. The unknown JSON operation is an intentional read-only parse rejection; all 12 requests return the expected exit code 2. Both other bursts return exit code 0. Source revision, Issue metadata, active pointer, and manifest hashes are unchanged before/after.

| Command | Sequential wall ms | Burst mean wall ms | Burst p95 wall ms | Burst mean direct-child CPU ms |
| --- | ---: | ---: | ---: | ---: |
| `gwtd --version` | 119.48 | 182.73 | 208.40 | 28.65 |
| Unknown JSON operation | 147.29 | 753.94 | 898.07 | 33.85 |
| `git remote -v` | 95.73 | 347.30 | 371.41 | 14.32 |

CPU is measured with Windows `GetProcessTimes` and excludes descendant-process CPU. The unknown operation's mean burst wall time reproduces most of the earlier approximately 720 ms outside the native search timer, without any admission/model work. Its 571.22 ms mean difference from `--version` supports investigating main environment/repository initialization rather than attributing the whole delay to executable loading. These commands run in a fixed order under changing host load; their durations are not additive causal attributions.

## Recorded stages and remaining waits

[stage-summary.json](stage-summary.json) aggregates the raw [baseline perf samples](baseline-perf-2026-10-16.jsonl) and [cache-only perf samples](after-perf-2026-10-16.jsonl). Collector omissions are not filled. Nested phases overlap and must not be added.

The cache-only warm burst has exact UTC boundaries. Its recorded stage averages are:

| Stage | Samples | Mean ms | Maximum ms |
| --- | ---: | ---: | ---: |
| Admission | 12 | 6,892.76 | 10,055.85 |
| Runner transport | 9 | 1,103.56 | 2,263.75 |
| Runner index read | 7 | 7.12 | 20.27 |
| Query encoding | 7 | 995.24 | 2,024.43 |
| Chroma query | 9 | 32.21 | 206.78 |

All three unavailable after-burst responses named an interactive `search-multi` holder. These samples do not identify a background rebuild holder. Serial query encoding remains a substantial cost after the index-read improvement. Baseline UTC phase boundaries were not recorded, so its stage samples are kept as an all-phase aggregate without guessing a warm-only window.

[query-thread-benchmark.json](query-thread-benchmark.json) and [query-priority-benchmark.json](query-priority-benchmark.json) independently encode 12 distinct queries with the real model. They observe an inherited Windows `BELOW_NORMAL_PRIORITY_CLASS`, intra-op 4, and inter-op 24 before explicit Torch setup. One Normal-priority sequence totals 5,206 ms at intra-op 4, 2,799 ms at 2, and 2,493 ms at 1. These are component-only observations under changing host load, not a passing Rust 12-request benchmark. The temporary diagnostic process restored its priority; live application workers were not changed by the probe.

## Full-source component measurements

[source-components.json](source-components.json) compares fresh module imports and two repeated calls against the same complete 1,622-Issue / 125-SPEC source. It excludes embedding, admission, transport, and browser work. "Cold" means an empty process-local module cache; operating-system file caches were not flushed.

| Component | Baseline cold ms | Baseline warm ms | Cache-only cold ms | Cache-only warm ms |
| --- | ---: | --- | ---: | --- |
| Issue source snapshot | 26,394.27 | 2,214.71 / 1,879.75 | 1,768.77 | 0.52 / 1.96 |
| SPEC source plus record assembly | 1,212.05 | 1,116.24 / 906.64 | 902.54 | 153.93 / 125.34 |

The baseline ran first and host/file-cache conditions were not controlled. The cold 26-second sample must not be turned into a causal improvement ratio. Repeated source reads were removed while complete source counts were preserved.

## Canonical refresh and publication probe

A native `index-issues --mode incremental --respect-ttl --qos interactive` refresh copied all 1,622 vectors but returned `PUBLISH_FAILED` after 23,413.79 ms. Source fingerprints were unchanged and `last_full_refresh` stayed at its original value. No health metadata was edited to call the index fresh. The failure and repair state are retained in [canonical-refresh.json](canonical-refresh.json).

With no benchmark Python worker alive, a separate clone with reader retention disabled could open 1,622 documents, close the client, and rename the directory immediately, both before and after dropping references/GC. [publication-handle-probe.json](publication-handle-probe.json) records that limited check. A second [native incremental clone](publication-clone.json), also with retention disabled, reproduced the failure after 99,431 ms: Windows returned `WinError 5` while renaming `issues.staging` into a generation directory. This reproduces the publication failure outside the retained-reader context; it does not identify the particular outstanding handle. The primary fixture was unchanged by both probes.

The PM's 2026-10-16 04:06 UTC decision assigned the Windows publication failure to [Issue #5611](https://github.com/akiojin/gwt/issues/5611) and authorized a known successful published snapshot for #5598 performance verification. [final-snapshot.json](final-snapshot.json) records the copy of the successful source/generation/meta/manifest tuple into a separate owned HOME. The original failed fixture and evidence remain available; the copied TTL metadata retains its original timestamp.

Separate headed browser attempts on the older memory fixture also missed their latency assertions (dark p95 approximately 8.09 s; light 5.49 s), with zero console/page errors. They are not counted as passing UI/performance evidence here.

## Manual headed browser check after query QoS

The subsequent real Chromium headed run passed **two tests, dark and light**, against the checkout GUI with a fresh isolated HOME and the real model. [query-qos/browser.json](query-qos/browser.json) projects the original attachments, their hashes, executable/runtime provenance, timings, and errors. This is manual headed evidence at the query QoS stage. Consult the PR Verification Run Record for fresh canonical `verify.run` headed evidence on the delivered source.

| Theme | Warm sequential requests | Sequential p95 ms | Eight concurrent responses | Settings / presets response ms | Health panel interaction ms | Console / page errors |
| --- | ---: | ---: | ---: | --- | ---: | ---: |
| Dark | 20 / 20 | 589.30 | 8 / 8 | 17.38 / 17.43 | 232.54 | 0 |
| Light | 20 / 20 | 238.66 | 8 / 8 | 11.18 / 11.23 | 570.73 | 0 |

Both control responses and the real health-panel interaction completed while all eight searches were pending, within the asserted 2,000 ms limit. The fixture has one real memory document and tests the browser route, theme rendering, returned fixture result, and responsiveness. It does not substitute for the primary 12-distinct-query / 1,622-Issue workload. The eight-search response maximum was 1,934.39 ms in dark and 3,457.53 ms in light; that batch has no 3,000 ms search-latency assertion. The native 12-request result is evaluated separately above.

## Reproduction

Build and preserve the checkout baseline/after executables first. Use the private pinned Python described in the provenance files and an already cached real E5 model. The fixed private source snapshot remains machine-local; recreating it from a newer Issue cache can change its fingerprint and document counts.

```powershell
python scripts/diagnostics/issue-5598-search-burst.py --prepare --seed-from-active-index --binary target/issue-5598-baseline/gwtd.exe --home target/issue-5598-new-home --dependency-python target/issue-5598-python/Scripts/python.exe --model-cache "$env:USERPROFILE/.cache/huggingface" --source-ref 0b087f0b79dd5578dacadea51165caf467e43598 --output target/issue-5598-new-evidence --scopes issues

python scripts/diagnostics/issue-5598-search-burst.py --binary target/issue-5598-complete/gwtd.exe --home target/issue-5598-new-home --dependency-python target/issue-5598-python/Scripts/python.exe --model-cache "$env:USERPROFILE/.cache/huggingface" --source-ref HEAD --output target/issue-5598-new-evidence --label after --scopes issues --query-file docs/perf/issue-5598/queries.json
```

Use `--source-components` with the baseline executable while its embedded runtime is materialized to reproduce the source-only component protocol. Keep failed attempts in distinct output directories when changing query lists or query QoS. Fresh verification should write under `target/` so committed evidence does not change during verification.

User Verification Result: n/a (autonomous)

Agent Visual Check: pass (manual headed Chromium dark/light, 2 tests; fresh canonical evidence is in the PR Verification Run Record)

The second unchanged-source attempt is retained in [recovery-second-attempt/after-search-burst.json](recovery-second-attempt/after-search-burst.json): 12/12 warm responses, outer p95 **3,715.63 ms (fail)**, native maximum 3,130.15 ms. CPU was 50% before and 74% after. The mean runner transport of 208.40 ms across 12 serialized requests already accounts for about 2.50 seconds, before outer process startup and handoff costs.

A separate [launch-priority diagnostic](priority-diagnostic.json) alternated the owned driver's BelowNormal / Normal / Normal / BelowNormal class, preserving the binary, corpus, distinct queries, and native search route. All four bursts returned 12/12. Their p95 values were 2,248.98 / 2,159.41 / 1,869.99 / 1,698.38 ms. Because order and host load were not fixed, this does not establish a causal priority improvement, and no additional CLI priority change was made. Both ordinary-priority controls passed. This diagnostic is kept separate from the standard before/after protocol.
