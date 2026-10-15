# Issue #5531: background WebSocket receive recovery

Owner: [Issue #5531](https://github.com/akiojin/gwt/issues/5531).
Parent: [SPEC #5390](https://github.com/akiojin/gwt/issues/5390).
The comparison dispatcher is pinned to `bc175f05a2b730775e1f5302b3af0d588ba4470a`.
The fetched develop `73eddff7311c0e042ab9ec90702e6745982e8e86` still had the
identical dispatcher blob `e0f304b0131896cacc824027e2e70ed08ef5d1d1`, including
the repeated `unshift`, and no receive-queue cap or foreground resynchronization.

## Change

Budget deferral previously prepended every remaining entry separately. The
regression test measures copied/shifted elements rather than runner wall time:
64 queued entries required 2,080 operations before the fix. The bulk-copy
implementation requires 190, 766 and 3,070 operations for 64, 256 and 1,024
entries respectively, preserving delivery order across flushes.
One bulk prepend keeps the existing scheduler and ordering with less change
than introducing a deque or a second cursor-based queue representation.

The receive queue now has a 2,048-entry limit. Overflow discards obsolete
traffic and requests authoritative snapshots instead of replaying a stream
with gaps. A native foreground visibility event also discards the backlog and
requests fresh state. Old socket generations cannot recover a replacement
socket. Recovery reloads mounted read surfaces and preserves drafts; it does
not retry mutations. A lost File Tree save or unconfirmed Board post remains
explicitly uncertain so the user can decide whether to retry.
Late File Tree read replies cannot overwrite a new edit made during recovery.
Settings recovery preserves explicit-save inputs while synchronizing the
autostart checkbox's value and pending state independently.

## Measurement method

- Run `scripts/diagnostics/issue-5531-browser-check.ps1` with `-Label before`
  and `-Label after`, each with `-BackgroundMilliseconds 1800000` and the
  pinned `-BaselineRef` above. Each invocation runs both Operator themes.
- Use this checkout's `target/debug/gwt.exe` with a fresh HOME, disabled PM
  and Issue Monitor, stable managed-hook fallback, and recorded owned PID.
  All ten changed frontend assets are compared with the served assets.
- Use real headed Chromium and a native minimized window. A private copy of
  Playwright 1.49.1 disables its forced focus emulation; Chromium's default
  background timer policy is restored. Every background sample checks the
  browser's actual `document.visibilityState`.
- Stream eight agent panes over a native loopback WebSocket (`ws@8.18.3`):
  terminal output and previews on a 50ms target tick, workspace state every
  20 ticks, and a 4,096-message burst every 600 ticks (nominally 30 seconds).
  Actual throughput is recorded in the samples; host scheduling can delay ticks.
  The controller uses real elapsed
  time, not an accelerated clock. Traffic counters are sampled throughout.
- Compare the original dispatcher with the changed dispatcher in the same
  checkout frontend. Before mode adds passive instrumentation and no-op
  adapters for its missing resume/dispose API; its queue and scheduler remain
  unchanged. The rest of the frontend and the workload stay the same.
- Freeze the backend snapshot revision before foregrounding. Require all
  eight fresh terminal snapshots and a keyboard input echo within two seconds
  of the controller's native foreground request. Map its wall clock to the
  browser's `performance.timeOrigin`; a delayed visibility handler cannot
  reset the deadline. Windows helper compilation happens in setup.
- Record the maximum Long Tasks API duration in the initial 0–2s interval
  and in the settled 2–4s interval. AC-4 requires settling within two seconds:
  the latter maximum must be below 200ms. An initial transient is reported,
  even when it ends before the deadline. A value of zero means no reported
  task of at least 50ms, not zero CPU use.
- Collect foreground long tasks independently of snapshot/input recovery, so
  the old implementation can report its 0–2s and 2–4s windows even when it
  does not obtain a fresh snapshot. Bound recovery collection to 15 seconds.
  On timeout, preserve background samples and the latest completed long-task
  maximum, record the stage, and reclaim only the owned browser PID.
  If the renderer cannot supply foreground samples, those metrics remain
  `null`, with `foreground_metrics_complete: false`. This is a negative
  reference outcome, not a successful recovery or a zero-duration long task.
  Reclaiming the baseline browser also makes Playwright's
  browser fixture report `Target ... has been closed`; the before command
  therefore exits nonzero. Its preserved JSON, rather than a passing baseline
  test result, is the comparison evidence. The after command must pass.

Before/after invocations use separate owned instances with staggered starts
so foreground operations do not overlap. Both instances are cleaned up.

## Evidence and verification

All four real background periods lasted 1,800,013–1,800,117ms, with
659,531–668,114 WebSocket frames sent during each period.

| Run | Peak queue | Completed task max (ms) | Tasks 0–2s / 2–4s (ms) | Snapshot / input (ms) |
| --- | ---: | ---: | --- | --- |
| Before, dark | 630,792 | 14,782 | unobserved / unobserved | timeout at 15,004 / unobserved |
| Before, light | 657,976 | 41,961 | unobserved / unobserved | timeout at 15,005 / unobserved |
| After, dark | 2,048 | 979 | 301 / 0 | 546 / 793 |
| After, light | 2,048 | 627 | 140 / 0 | 361 / 518 |

Both baseline timeouts occurred while waiting for native visibility. The
completed-task maximum is the last observable value, not the duration of the
unfinished foreground task. No foreground duration is inferred from timeout.
The baseline light observation gap reached 68,238ms while its main thread
blocked; after dark/light gaps stayed within 57,726/30,249ms. Both after runs
passed, with zero console/page/fixture errors. Their initial tasks ended
before the two-second deadline; both settled windows had no reported tasks.

![Dark theme after recovery](after-dark-recovered.png)

![Light theme after recovery](after-light-recovered.png)

The raw `before-dark.json`, `before-light.json`, `after-dark.json` and
`after-light.json` include real elapsed duration, traffic samples, native
visibility events, long tasks, snapshot/input latency and console/page errors.
The two launch manifests record the owned process, isolated HOME and
LF-normalized SHA-256 digests of the ten frontend files and both test inputs.
They are preserved outside Playwright's output directory during startup.
These measurements preceded the final source commit: `head_sha` records the
parent commit, while the file digests identify the measured working tree and
are checked against the delivered files.

`node docs/perf/issue-5531/verify.mjs` checks all four 30-minute runs, their
source identity and the acceptance thresholds. It also requires increasing
traffic ticks/frames in each observed interval, so a one-off initial burst
cannot prove continuous streaming. After runs must have no observation gap
over one minute. The baseline can delay page-side observations for longer
while its main thread is blocked; those gaps are reported with the negative
reference and never count as passing recovery evidence.
The changed implementation must complete recovery; a timeout is permitted
only for the old dispatcher as an explicitly recorded negative reference.
Final canonical verification
also runs a fresh headed dark/light smoke against the same source. Short
smokes explicitly record `acceptance_duration: false`; they never replace
the 30-minute measurements.

The unit regressions force overflow deterministically. Native browser delivery
may or may not reach the cap in a particular live run; its actual peak and
recovery counts are recorded without modifying the production scheduler.

## Scope and limits

The workload emulates eight agent event streams; it does not start eight
models or PTYs. The checkout serves the real frontend while a fixture owns
the WebSocket backend. This measures GUI receive recovery, not fleet CPU or
native WebView process performance. The shared host can introduce scheduling
noise; initial and settled long-task values are both retained.

Keyboard input follows the fresh-snapshot wait so a subsequent snapshot cannot
erase the echo. When the baseline never obtains a fresh snapshot, its input
latency remains unavailable; this is not an isolated keyboard-latency
comparison. The changed implementation must complete both operations within
the same two-second deadline.

Windows full frontend testing also encounters the pre-existing dynamic import
failure tracked by [Issue #5470](https://github.com/akiojin/gwt/issues/5470).
That unchanged test is outside this fix. Scoped frontend tests and the actual
headed runs cover this change; delivery additionally checks Linux frontend CI.

User Verification Result: n/a (autonomous)
