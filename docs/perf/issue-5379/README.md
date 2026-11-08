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

The delivery headed check uses a dedicated functional real-PTY test:

```text
CARGO_TARGET_DIR=target pwsh -NoProfile -File scripts/diagnostics/issue-5379-browser-check.ps1 -DurationSeconds 10 runtime-output-functional-live.spec.ts --grep "one Agent delivers fixed output and restores its screen"
```

It uses the existing isolated wrapper and custom Node Agent without modification. In each theme, one real Agent emits ten sequences of 8192 bytes. The test checks every sequence in live `terminal_output` frames, at least 81,920 received bytes, the final visible terminal buffer, and a fresh same-pane snapshot after page reload opens a new WebSocket. Producer session/run identity, zero console/page errors, screenshot capture and owned cleanup are also checked. Evidence is written to separate `functional-dark.json` / `functional-light.json` files; it does not overwrite the performance observations. This verifies output functionality, not shell input or latency acceptance.

The first canonical selection, the existing `project-tabs.spec.ts` shell test, failed in record `vrr-d23217f5c57e429e8b6ba3550b45e6ee` (15 commands passed; the headed command failed). Its visibility assertion targets the legacy terminal container parked in a hidden cache rather than the displayed Agents tile. The same selector exists on develop. A separate existing shell test also encountered PowerShell 7.6.6 update-notification `PadRight` FailFast; with notification disabled for that diagnostic process only, its first `node` input became `ode`. The input failure reproduced against both the preserved before binary and the after binary, so it does not require this change. A PATH-limited diagnostic still selected PowerShell and failed. These diagnostics do not establish the input failure's root cause, and no product or generic shell-test repair is included here.

A separate ten-second run of the unchanged performance test failed the dark output-batch assertion at 432ms; light passed. That short diagnostic overlapped another isolated diagnostic during its light phase, so it is not the primary performance comparison. All these failed results and traces are retained with hashes in the machine-local `work-notes/evidence/issue-5379/sena-20261020-thread-profile/headed-functional-diagnostics` manifest. They are not retried until green or reclassified as performance passes.

The original 30-minute performance FAIL and its 100ms assertions remain unchanged. Issue #5379 stays open for final AC-1 remeasurement after Issue #5816, while this independently releasable source slice proceeds through fresh canonical verification, required CI and develop delivery.
