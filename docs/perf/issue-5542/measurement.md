# Issue #5542: hook live transport

Classification: gwt feature. Hook transport and Monitor observations apply to
every project opened in gwt. Spec Status: IMPLEMENTATION-GAP, owned by
Issue #5542 under SPEC #5390.

The fetched baseline was `490a2b310ca3e4440144faa2bc59b58a60af3ab2`, identical
to this branch at launch. There was no undelivered source to adopt.

## Root cause and change

The host ledger snapshot through 2026-10-15T22:20:13Z contains 579 failures with
detailed attempt fields. Of these, 256 expired before any HTTP attempt; another
41 began with less than 20ms remaining. The diagnostic elapsed clock started **after** cold
reqwest client initialization, hiding that cost behind `elapsed_ms=0` and an
incorrect readiness-exhausted message. Timed-out attempted requests took
104ms median, 266ms p95 and 476ms maximum wall time. These are elapsed failure
times including scheduling, not measurements of receiver acknowledgement time.

The real receiver authenticates, queues a RuntimeHook and returns HTTP 204.
GUI processing and SessionStart readiness completion happen subsequently;
the HTTP handler does not wait for them. Existing successful-204 access-log
suppression prevents reconstructing individual receiver durations from the old
ledger. See `host-failure-snapshot.json` for counts and primary examples.

The change reuses the existing bounded plain HTTP path for all local HTTP hook
events, avoiding a new reqwest runtime/pool in every short-lived process. The
ordinary attempt ceiling is 500ms, based on the observed wall-time tail.
UserPromptSubmit's stricter outer deadline and SessionStart's one-second total
remain in force. Ordinary hooks still make one attempt. Readiness retry,
capability authentication, endpoint validation and pending-state recovery are
preserved. IPv6 Host formatting and Unix WouldBlock timeout classification are
covered before broadening the existing path.

Content-free opt-in profiles (`GWT_HOOK_PROFILE_PATH`) now distinguish address
resolution, connection, send and acknowledgement on local HTTP, plus client
initialization/combined transport on the retained HTTPS fallback and readiness
retry wait. The measurement child uses the dispatcher's diagnostic
buffering and flushes after capturing elapsed time, excluding profile-file I/O
from the transport interval.

## Experiment design and provenance

This is a **controlled transport workload, not a deployed real-agent fleet**.
Twelve short-lived Rust test processes send a small RuntimeHook to a private
axum/Tokio receiver, using a fixture-only bearer token. HOME/USERPROFILE are
temporary and managed GWT identity/capability variables are removed. No real
Session capability or production app is used. The receiver omits the production
capability registry and GUI queue, so those costs are not measured.

Each round runs the current transport and the direct-socket control with twelve
concurrent children per mode, reversing mode order on alternating rounds and
requesting a five-second cadence. Both before/after long runs request 3,600s.
They overlap on the same shared Windows host; other agents, compilation and
browser verification continue. This is a stressed observational comparison,
not an isolated CPU-load-controlled experiment or a real twelve-window run.
Recorded transport time excludes process startup and the Session journal;
`process_ms` records the complete child lifetime separately.

`baseline-paired` uses the baseline production sender plus the ignored
measurement helper; `after-paired` uses the corrected production sender. The
binary SHA-256 in each summary is authoritative. Source base/producing commits
and the command are recorded with the final result. Legacy raw summaries capture
`source_sha` at measurement end, not at binary build: the baseline summary's
`2c6571534` is therefore not its production source. Use `sender-provenance.json`
for baseline base `490a2b310` plus its test-only helper, after producing commit
`2c6571534`, source blob IDs and binary SHA-256. Baseline and after use identical
native receiver behavior. Test-only budget overrides are absent in long runs.

## Preliminary comparisons and rejected fixtures

- A Python receiver with default backlog 5 was unsuitable for twelve concurrent
  connects. Backlog 128 still added Python receiver/thread scheduling noise.
  Preserve `pilot-backlog5.jsonl.gz` and `pilot-backlog128.*` as rejected fixtures;
  they do not establish improvement or satisfy an acceptance criterion.
- The native 90s pilot recorded current 100ms failures 10/216, direct-socket
  100ms failures 0/216. Longer sampling then found failures on both 100ms paths,
  disproving the initial assumption that cold-client removal alone suffices.
- A separate **test-only 500ms override** pilot recorded 0/216 failures on each
  path. It motivated the finite ordinary budget adjustment; it is not evidence
  of the final binary's default or the real fleet.
- Final-default buffered profiling: 144/144 deliveries. Resolve p95 0.027ms,
  connect p95 7.975ms/max256.879ms, send p95 0.200ms, ack p95 9.500ms/max35.912ms.
  No `hook-live/client-init` occurred on HTTP. This attributes the observed tail
  to connection/scheduling rather than a readiness wait in the receiver.

## Verification history

Two regression tests first failed for the expected missing phase records and
the ordinary zero-attempt readiness misclassification. On the CI-matched
Rust 1.99.1 toolchain, the corrected sender passed 27 daemon-runtime unit tests,
seven runtime integration tests and three hook diagnostic tests. These include
existing retry/status/authentication and recovery cases; the ignored measurement
helper is not a normal test verdict. The degraded prompt-budget regression also
passed independently, retaining its stricter outer deadline.

The initially reused Issue #5042 browser stress suite exposed the new launcher's
WARN-only logging mismatch, which omitted fast DEBUG samples. With that fixture
fixed, it passed dark (p50 68ms) but failed its separate 500-viewport p50<=200ms
criterion in light (632ms). That test, threshold and GUI production code remain
unchanged. This owner instead has a scoped headed test of actual twelve-sender
transport plus independent real checkout GUI responses, both themes and zero
console/page errors. Its first run delivered 24/24 per theme with 16/20 GUI
responses and zero browser errors. It does not assert real Session-to-Monitor
convergence.

An unpinned `bunx --bun @commitlint/cli` attempt crashed Bun. The repository's
pinned CLI 20.4.1 through its normal runtime passed. An early Rust integration
build also hit Windows access denied while the browser launcher used the same
checkout gwtd.exe; serialize rebuilding that binary with operational CLI use.
Neither failed attempt is reported as a successful check.

The host's default Rust 1.95 clippy also rejected seven existing expressions in
unchanged `gwt-github/src/client/http.rs`. Successful CI run 38821257608, job
116718104255 records Rust 1.99.1. Install that toolchain separately and use it
explicitly for gate verification, leaving the machine default unchanged. Both
paired measurement binaries were built with 1.95, preserving their compiler
comparison conditions. Register the canonical plan only after raw measurements
stop changing: the preliminary derive correctly refused the moving worktree.

## Final evidence and acceptance

Both runs completed 720 rounds, 8,640 samples per mode. Baseline duration was
3,600.006s; after duration was 3,600.004s. These are the binary defaults, with
no test-only budget override.

| Sender | Attempt budget | Samples | Failures | p50 ms | p95 ms | Max ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Baseline production | 100ms | 8,640 | 332 | 10.300 | 84.742 | 511.073 |
| After production | 500ms | 8,640 | 0 | 5.501 | 39.426 | 804.362 |
| Baseline direct-socket control | 100ms | 8,640 | 215 | 5.671 | 56.656 | 820.250 |
| After direct-socket control | 500ms | 8,640 | 0 | 5.337 | 27.144 | 438.047 |

The observed production failure rate changed from 3.84% to 0% in this controlled
workload. The maximum after elapsed time exceeded the configured 500ms timeout;
it is not a hard wall-clock guarantee on this shared host. This long run did not
profile every request, so the 804ms outlier cannot be assigned to an individual
phase. The saved raw rows retain it. Do not infer zero latency outliers from
zero reported transport failures.

The driver command for each recorded sender binary was:

```powershell
python docs/perf/issue-5542/measure.py --binary target/issue-5542-baseline.exe --receiver target/issue-5542-receiver.exe --output docs/perf/issue-5542/baseline-paired.jsonl --seconds 3600 --interval 5
python docs/perf/issue-5542/measure.py --binary target/issue-5542-after.exe --receiver target/issue-5542-receiver.exe --output docs/perf/issue-5542/after-paired.jsonl --seconds 3600 --interval 5
```

Raw long-run data and rejected pilots are stored as deterministic gzip files.
`compressed-raw-manifest.json` records their uncompressed SHA-256 and byte counts;
decompression was checked byte-for-byte before removing the original raw files.
Run `python docs/perf/issue-5542/verify-evidence.py` to verify sample counts,
durations, actual defaults, failure counts and buffered phase attribution.
The fresh canonical record, exact delivered SHA, required CI coverage counts
and develop inclusion are recorded in the PR and owner delivery comment.

Real deployed twelve-agent/sixty-minute evidence remains required for owner
AC-2 and the corresponding real-fleet portion of AC-3. Keep those unchecked and
the owner open until the fixed version is deployed and actually measured.
The controlled measurements must not be relabelled as that real-fleet evidence.

Launch mode: autonomous

User Verification Result: n/a (autonomous)

Agent Visual Check: pass (scoped headed transport/UI test; repeat canonically
on the delivered snapshot)
