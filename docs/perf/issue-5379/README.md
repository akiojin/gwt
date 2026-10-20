# Issue #5379 delivery evidence

This delivers the bounded PTY reader, worker-captured screen and lazy quota-clock improvement. Hook-health aggregation already runs outside the GUI loop in develop. Final AC-1 remains **pending after Issue #5816** under PM decision `33031353-c4f9-4cad-963b-afd327090da2`; the latency budget is still 100ms.

Both arms use five real custom Agent PTYs, each emitting 8192 ASCII bytes per second. Dark is a 1800-second observation; light is a separate 10-second functional observation. These deterministic Agents do not call provider models/APIs.

| Dark observation | Before | Fresh after |
| --- | --- | --- |
| Actual interval | 1,801,461ms | 1,801,013ms |
| Producer payload | 73,728,000 bytes | 73,728,000 bytes |
| Completed sequences | 1800 for all five PTYs | 1800 for all five PTYs |
| All native dispatch: stalls / maximum | 19 / 437ms | 7 / 435ms |
| RuntimeOutput: count / stalls / maximum | 27,974 / 9 / 407ms | 18,000 / 3 / 345ms |
| Independently timed output batches: count / stalls / maximum | Not instrumented | 9,002 / 2 / 432ms |

Stalls mean at least 100ms. Both runs received every output sequence; browser errors are empty. Before and after are different revisions, including intervening develop changes through `f16d12430` before the treatment changes, so these differences are descriptive rather than a single-change causal estimate.

The fresh treatment was clean source `a4e24a94fe1b693d91b84d26b29e11a23411cfc6`, binary SHA256 `836f34a3b57ea69d3bb6ef9a0544d772d13ef31378c7eba7fa4fe6e86a1f82d6`. The 11 measured source fingerprints include `crates/gwt-core/src/usage/limit_notice.rs`. `provenance.json` binds that source and the before/after binary identities; the checker accepts LF/CRLF differences only.

## Preserved failure and environment

The original performance test is unchanged. Its dark result remains `passed: false`, `agent_visual_check: fail`, with runner exit 1 for `RuntimeOutput dispatch stays below 100ms` (345ms). Light passed. Both themes completed their five PTYs, with zero browser errors; owned GUI cleanup succeeded with no cleanup errors.

Read-only Job observations identify the same GUI PID 33388 and exact start `2026-10-20T09:18:15.9595753Z`. They observed an enabled 8% hard cap at 09:19:55Z, 09:35:14Z and 09:48:23Z. These are snapshots, not a continuous scheduling trace or proof that the cap caused an individual delay. The earlier clean 132ms failure's GUI cap was not measured. Its cause remains unknown.

`after-environment.json` contains the exact identity, cap readbacks, clean source capture and original Playwright test error. `after-run-result.json` preserves the runner result. The full 2,065,101,664-byte trace is retained at the recorded machine-local path with SHA256 `00c7517b3743d727932a659c5d2b5226c865c756f351704efa2e80c178d94ccb`; it is not committed. `retained-failures.json` preserves the earlier clean 132ms and other failed diagnostic records; dirty probes are not treatment evidence.

## Verification scope

From the repository root, run:

```text
node docs/perf/issue-5379/verify.mjs
node docs/perf/issue-5379/verify.test.mjs
```

The first command validates source/binary provenance, complete payloads, browser errors, cleanup and every original timing row against its aggregate. It reports `record_integrity: pass` and `latency_acceptance: pending`, owner 5816. This is not a latency PASS. The explicit pending disposition requires the PM decision, 100ms budget, hashed retained failures, matching measured source, exact GUI/cap evidence and original latency error. Passing theme records must still agree with the budget. The second command proves rejection of missing disposition, altered exact start, non-latency runner failure and missing output sequence using an isolated copied fixture.

The final canonical headed check reuses the existing functional real-PTY test:

```text
CARGO_TARGET_DIR=target pwsh -NoProfile -File scripts/diagnostics/issue-5379-browser-check.ps1 -DurationSeconds 10 project-tabs.spec.ts --grep "live checkout bootstraps a project-scoped connection and accepts terminal input"
```

This selects the existing project-scoped connection/input/output test in both themes, including zero console/page errors. It does not rerun or waive the residual latency criterion. The original 30-minute performance FAIL remains separate. Issue #5379 stays open for final AC-1 remeasurement after Issue #5816, while this independently releasable source slice proceeds through fresh canonical verification, required CI and develop delivery.
