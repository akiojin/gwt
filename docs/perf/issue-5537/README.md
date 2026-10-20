# Issue #5537 measurements

Measured on Windows on 2026-10-16. The baseline is commit `38c0a2829`;
the following measurements use the implementation in this report.
These are controlled reader measurements, not a whole-fleet performance claim.

## Six-second reader regression

Command: `cargo test -p gwt --bin gwt --all-features list_issue_monitor_ -- --nocapture`.

| Measurement | Before | After |
| --- | ---: | ---: |
| List dispatch with the same six-second reader | 6410.189 ms | 0.598 ms |
| Reader calls inside dispatch | 1 | 0 |
| Focused tests | 3 passed, 1 expected RED | 6 passed |

The test uses the real dispatch watchdog. After dispatch returns, the queued
worker remains in the reader for six seconds; no `gui_event_loop_stall` is
written to its isolated error ledger. The assertion checks deferred reader
execution, rather than relying on a tight scheduling threshold. Separate
regressions cover out-of-order replies, parallel clients, newer daemon
status/inbox, and rereading after a control changes the source.

## Real headed Chromium

Command: `pwsh -NoProfile -File scripts/diagnostics/issue-5537-browser-check.ps1 -Label after --headed --trace on`.

The checkout binary runs with a fresh HOME, disabled PM/Monitor auto-start,
and exact owned-PID cleanup. An armed Git shim delays the local origin read;
its existing operation deadline bounds the wait. The browser sends List and
the other GUI request, in that order, on the same WebSocket in one JavaScript
turn. The test then checks that the real reader entered the matching gate
generation and that the other reply arrived before the delayed inbox. The
old synchronous handler processes the inbox before the next request, so it
fails this ordering assertion. This browser delay is distinct from the
six-second Rust reader above.

| Theme | Other request reply latency | List status after request | List inbox after request |
| --- | ---: | ---: | ---: |
| Dark | 1.5 ms | 1156.5 ms | 1156.7 ms |
| Light | 2.3 ms | 1079.1 ms | 1079.1 ms |

Both themes passed. The other request replied before the delayed inbox;
status and inbox completed, Settings remained visible, and console/page
errors were zero. The reply latency is measured from the other request's
send time; List times are measured from the List request's send time.
Raw observations are in [dark](list-worker-after-dark.json) and
[light](list-worker-after-light.json).

An earlier traced run exposed a harness race: polling the filesystem gate
before a separate browser evaluate sent the other request after the inbox
(dark: 1048.6 ms inbox, 1058.1 ms send; light: 1170.8 ms inbox, 1171.1 ms
send). Both replies took about 1.4 ms after sending. Submitting both requests
in one browser turn removes that race without changing production deadlines.

User Verification Result: n/a (autonomous).
Agent Visual Check: pass (headed dark/light).

## Implementation boundary

List prepares daemon/local prefs, cache candidates, capacity, profiles,
update-drain blockers, status/inbox, and the normalized Knowledge root on
the existing worker. The GUI applies current results and merges newer
published status/inbox from memory. Controls trigger a fresh read;
ordinary projection updates do not cause retry starvation.

The offline List is a read model. Scheduled/daemon paths retain durable
admission, launch expiry, and PM wake ownership. The protocol, settings
schema, Monitor scheduling, claim authority, and UI layout are unchanged.
