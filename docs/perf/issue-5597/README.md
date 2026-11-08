# Issue #5597: periodic GUI notifications

## Scope and cause

The host capacity sampler runs every five seconds. Its GUI handler previously
read the machine snapshot, resolved repository identity, loaded Monitor prefs,
rebuilt capacity and wrote reduction diagnostics on every notification.
The usage collector runs every thirty seconds (and on explicit refresh); its
GUI handler resolved every eligible repository and transacted Monitor prefs.
Both producers notified the GUI even when the relevant values were unchanged.

Preparation and persistence now run on the existing capacity/usage workers.
Capacity messages carry only an overlay and the captured project/control
revision; the GUI preserves the rest of its current status. Sensor timestamp
renewals do not trigger messages. The capacity expiry transition does.
Account comparison ignores only fetch time and the age of an already-stale
reading. Raw observations still reach durable quota reconciliation every poll.
The existing terminal convergence clock rechecks pending quota candidates.

Initial full status must be ready before a capacity notification is consumed.
Closed/reopened projects and intervening control changes reject old results.
Manual selection, the last accepted incomplete Auto limit, stopped Monitors,
quota account switches and reduction diagnostics retain their existing rules.
Cached capacity timestamps describe the last published observation; admission
uses the independent machine snapshot, and the worker still publishes expiry.

## Before: deployed v9.125.0

The retained raw `gui_dispatch_summary` records are in
[before-dispatch.json](before-dispatch.json). They were extracted from the
machine-local `perf-2026-10-16.jsonl`, GUI PID 34640, timestamp filter
2026-10-16 00:57:00Z through 01:17:00Z (exclusive end). Each handler has 19
minute-summary records. These summaries count only dispatches over 50ms.

| Handler | Slow count | Total ms | Maximum ms | Maximum minute total ms |
| --- | ---: | ---: | ---: | ---: |
| IssueMonitorCapacityChanged | 106 | 6377.2066 | 119.5260 | 840.9718 |
| ProviderUsageSnapshot | 42 | 3200.8500 | 161.8209 | 363.5181 |

This is the reported live-fleet baseline, not a controlled same-fixture
comparison. The after run uses an isolated checkout instance and records its
binary/source hashes rather than claiming an identical fleet workload.

## Reproduction and interpretation

Build this checkout's `gwt` and `gwtd`, then run
`pwsh -NoProfile -File scripts/diagnostics/issue-5597-browser-check.ps1 --headed`.
Canonical verification passes the same command through `verify.run`, selecting
it in `headed_e2e_commands` so gwtd supplies headed Chromium and its reporter.

The harness uses a fresh HOME, disables PM auto-start and Issue Monitor
launches, checks managed-hook convergence and cleans up its exact owned PID.
Each Operator theme observes at least 65 seconds of real capacity and usage
polling. It forces three unchanged usage refreshes, checks continuing snapshot
renewal and UI replies, and asserts zero console/page errors.

Every target-handler debug timing is retained alongside the minute summaries.
The debug integer milliseconds are truncated, so each sample's upper bound is
its value plus one millisecond. Both individual and rolling sixty-second
budgets are checked with these conservative upper bounds (16ms and 100ms).
An absent slow-summary row alone cannot prove the 16ms requirement.

User Verification Result: n/a (autonomous)

## After: isolated candidate

Canonical record `vrr-8639cd04f57f4747b1a3f539efced0e0` passed all eight
commands under Rust 1.99.1: format, workspace all-target/all-feature clippy,
36 focused Rust tests (9 binary and 27 library), test hygiene, compiler check,
checkout binary build and two measured headed Chromium tests.

Raw evidence: [dark](after-dark.json), [light](after-light.json), and
[owned PID cleanup](owned-pid-cleanup.json). Both themes used GUI PID 43128.
The debug rows are cumulative from the same owned GUI startup through each
theme's end; these are not independent fleet trials. Each theme itself
observed more than 67 seconds of real polling, 12/10 usage frames and multiple
renewed capacity snapshots. Codex HOME and Claude config were isolated with no
provider credentials: account observations were unchanged NoData/Disabled.
Account changes and populated quota recovery are covered by the Rust tests.

| Evidence through | Capacity / usage dispatch count | Maximum per dispatch, upper bound | Combined maximum rolling minute, upper bound |
| --- | ---: | ---: | ---: |
| Dark | 11 / 1 | 1ms | 11ms |
| Light | 24 / 1 | 1ms | 11ms |

All captured elapsed values were 0 truncated integer milliseconds. The upper
bounds above add 1ms for every sample. Both satisfy 16ms per dispatch and 100ms
per rolling minute, including the stricter combined-handler interpretation.
Unchanged forced and ordinary usage refreshes enqueued no GUI quota events
during either observation. Console/page errors and dropped summary windows
were zero. The one/two real minute-summary records are retained verbatim in
the evidence JSON; neither contains a slow target-handler entry.

The recorded candidate ran on dirty HEAD
`0b087f0b79dd5578dacadea51165caf467e43598`; source and binary SHA256 values
identify the measured implementation. Final committed-HEAD canonical
verification and required CI are recorded on the delivery PR. Its screenshot
artifacts are retained by the harness. No coverage measurement was performed
locally: required CI applies the 90% gwt-core + gwt and 80% other domain-crate
gates after outer-shell exclusions.

Agent Visual Check: pass
