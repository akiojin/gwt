# Works telemetry and terminal layout (2026-11-02)

This slice removes unnecessary terminal layout work from data-only Works
telemetry updates. The reported Mac wheel and typing failure remains
unconfirmed and unverified on this Windows host.

The app's telemetry frame subscriber called Works synchronization with forced
rehosting. That refreshed terminal mounts and requested layout even when the
native terminal and its host were unchanged. The app's layout callback then
scheduled persisted terminal fits. The activation helper sends geometry for
an explicit persisted fit even when its mock grid stays unchanged.

The fix exposes the existing render path's rehost argument with its original
default, and passes false only from telemetry updates. Metrics still render.
Host changes and reveal/recovery callers retain ordinary synchronization and
its explicit rehost/layout behavior. The container ResizeObserver remains
responsible for actual container size changes. Identity, pane, incarnation,
input acknowledgements and lifecycle classification are unchanged.

The alternatives were global geometry deduplication, which could discard
intentional repair, and a parser-only guard, which leaves frontend and HTTP
work intact. Removing forced layout at the telemetry caller is smaller and
preserves those other contracts.

The finite probe imports the production modules, executes the actual app
telemetry subscription, and uses one frozen copy of the existing linkedom
fixture before and after. Its source SHA256 is
`ba1026e9f94596b74e6ede6c77ab2b85345ae182083a89b65278fbd94732e2df`.

| Case | Before | After |
| --- | --- | --- |
| 20 telemetry updates in one burst | 1 frame, 1 layout, 2 mount calls | 1 frame, 0 layouts, 0 mount calls |
| 100 equal catalog publications | 0 notifications | 0 notifications |
| 100 equal status publications | 0 notifications | 0 notifications |
| 100 catalog version updates | 1 frame, 0 layouts, 0 mount calls | Unchanged |
| Pending frames after updates stop | 0 | 0 |

No closed, self-sustaining catalog/geometry loop was found. The unnecessary
work is driven by external updates. A separate activation-helper control
continues to produce ten fits and ten geometry callbacks for ten explicit
persisted activations. That control is not a complete-app or HTTP count.

The single regression was RED: telemetry caused five total mount calls where
three were expected. After the fix, all 105 related Works, wheel, viewport
and state-store tests passed. It also checks visible metrics, the retained
native host, the app subscription wiring and ordinary host synchronization's
positive behavior. Independent review reported zero blocking findings.
Probe sources, outputs, dependencies, hashes and test logs are in
[the evidence artifact](ui-telemetry-fit.json).

Intermediate canonical record `vrr-0bc801c3c9ae4d9ca9ed89cbe58947eb`
passed 14/14 commands, including all 1,716 frontend tests, 101 Rust tests
(one existing helper ignored), strict workspace Clippy, formatting and both
binaries. `all_passed` and `plan_covered` are true. The record file SHA256 is
`1b4b6700be27261f24033f040d98e4515edb92716d8c6e85d226e89daeec8609`.
Execution status immediately afterwards was active/bound/fresh/autonomous.
It verifies the dirty fix on `fb0d4f0409757182fa89f70be75d47edc868ba41`
before this checkpoint's documentation and commit, with zero headed passes.
Overall remains incomplete because the actual UI and delivery gates are open.

The preceding record `vrr-21f00da6e8924edea089478c71ea8c5e` remains a
raw FAIL: frontend 1,708/1,716, then fail-fast skipped nine planned commands.
Bare bash resolved to WSL Node 18.19.1 and WSL Git in a Windows worktree.
Errors included unavailable crypto/File, the older mock-timer API and an
invalid Windows gitdir interpretation. The successful rerun uses the explicit
Git Bash executable with native Node 24.11.0; CI specifies Node 22. It reruns
the same complete test script and registered matrix without changing unrelated
source or excluding failures. The original record, output hashes and failure
categories are preserved in the artifact.

Existing Windows observations guided the investigation, without establishing
Mac causation. The shared log window 05:10–05:26 UTC contains 14,894 pane
resize records and nine actual PTY resize records. It also contains 369,027
PM preference-lock records; these are lock acquisitions rather than preference
writes. Those rows have no process/request correlation. They do not prove
that this telemetry caller caused all observed resizes or the original
notifications.

A separate 5.100-second sample of GUI PID 22480 measured 319.55% CPU and
1,018,466,304 bytes RSS. Read-only thread descriptions identify main thread
11824 with 2.390625 CPU seconds; the two hottest threads were Tokio workers.
Completed-dispatch summaries show Board reads and other synchronous work,
but retain only handlers over 50 ms. They do not measure browser input
receipt or rule out shorter repeated work. No CPU reduction is claimed.

This fixture uses two live entries and zero real Agents. It does not exercise
Chromium, a PTY, wheel/keyboard event delivery, actual container reflow, Mac
recovery or installed rollout. The Mac acceptance remains both scrolling and
typing working without recurring errors. Required managed-browser preflight,
headed dark/light/error-free checks, final-head verification and PR/CI/coverage
delivery remain open, together with EO3/EO4/RPC4 and parent fleet criteria.
After verification, current managed-browser doctor remains
current/converged/normal but unknown with `ledger_unavailable`. Fresh fetched
develop remains `0bffeba7`; it contains no error-ledger repair diff, and shared
GUI PID 22480 is the same launch. No shared GUI stop, registration stamp,
ledger edit or guard relaxation was used. Coverage is unmeasured because no
PR or required CI coverage has run; it is not a threshold failure.

User Verification Result: n/a (autonomous).
Agent Visual Check: not run (canonical runtime/ledger prerequisites unmet).
