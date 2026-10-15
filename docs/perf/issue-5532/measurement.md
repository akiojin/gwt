# Issue #5532: Settings autostart responsiveness

## Cause and scope

The owner Issue reports Settings dispatch stalls of 8.07 seconds (dark) and
11.62 seconds (light). Those observations are the original incident evidence,
not measurements reproduced on this Windows host.

At baseline `85b2253c8`, both the status query and toggle/readback call the OS
autostart manager synchronously from the GUI handler. They now run through the
existing blocking worker and return through the existing client-scoped dispatch.
OS registration mechanisms and the wire protocol are unchanged.

## Controlled comparison

Command: `cargo test -p gwt --bin gwt autostart_ -- --nocapture`.

The regression uses the real frontend handler and its queued worker. While the
query is queued, a System Settings request must complete and no autostart reply
may have arrived. The queued task then runs with an injected 200 ms delay on a
separate thread and must reply to the original client. The comparison models the
previous synchronous placement with the same delay and read-only OS query.
The update regression drops its queued task without executing it, so it never
changes the host's login settings.

Reopening Settings reuses an outstanding status query and does not start a query
while an update is pending. A failed refresh preserves the last confirmed value;
only a failed update rolls back the optimistic toggle. These boundaries prevent
an older query reply from completing a newer update.

| Measurement | Duration | GUI stall classification |
| --- | ---: | --- |
| Synchronous query model | 202.929 ms | true |
| Worker-backed frontend dispatch | 0.421 ms | false |
| Delayed worker operation | 202.832 ms | outside GUI dispatch |

The classification uses the existing 30 ms `gui_event_loop_stall_warning`
diagnostic. Timings are observations, not sub-30 ms test deadlines: regression
assertions check queueing, response ordering and client routing, avoiding a
scheduler-dependent latency assertion on a shared CI host. This controlled
comparison proves removal of the OS wait from dispatch; it does not claim an
equivalent reduction in OS query latency or whole-application CPU use.

## Regression evidence

Before production edits, the actual-handler test failed with
`OS status must arrive from a worker` (0 passed, 1 failed). After the worker
change, both Rust regressions passed. The existing System tab suite passed
34/34 tests through Windows Node with linkedom 0.18.12 and acorn 8.15.0.

The live browser test uses the checkout Windows GUI with a fresh HOME and
Windows Chromium. Its transport holds only autostart requests while forwarding
other traffic. It checks Supported Agents hydration and language saves while
the query is pending, local checking/unavailable messages, update failure and
retry, and reopening during an update without racing a fresh query. Both themes
must pass with zero console/page errors in canonical verification.

The headed preflight passed both themes. Its first light run reached all feature
assertions but timed out on a redundant final close used only for cleanup. The
test now leaves that final cleanup to the page fixture and the wrapper's owned
process shutdown. The three close/reopen acceptance steps remain unchanged.
Unrelated `ListIssueMonitor` stalls were recorded separately on Issue #5537;
the final close failure's direct cause was not established.

User Verification Result: n/a (autonomous)
