# Issue #5543: Issue cache-load critical path

The first Issue list waited for historical Session and Work enrichment, even
though the cache read already ran outside the GUI thread. With a copied real
corpus, the read and enrichment took 121,725 ms, exceeding the unchanged
60,000 ms frontend deadline. See [the direct baseline](baseline-load.json).

The controlled headed baseline also timed out at 60,010 ms and displayed no
rows while Work decoding was held. See [the headed baseline](baseline-headed.json).
That rendezvous deliberately holds decoding to demonstrate the dependency;
its timing is not an ungated production latency measurement.

Correlated cache loads now publish cached rows with the latest available
association and Monitor snapshots before scanning historical data. The full
enrichment follows with the same request ID. An explicitly selected detail
refreshes when those associations change. Uncorrelated legacy requests retain
one reply; explicit remote refresh behavior and the frontend deadline stay
unchanged. This reuses the existing snapshots and worker instead of adding a
cache layer or extending the timeout.

Telemetry uses target `gwt::knowledge_load`, keyed by window and request IDs:
`queue`, `cache_read`, `assembly`, `dispatch_enqueue`, `related_assembly`, and
`send`. The first-response stages record elapsed and duration values and mark
deadline overruns. `send` measures serialization and socket-queue enqueue,
not socket-write completion; the browser measures actual receipt. Worker logs
can be project-scoped, while transport logs can be in the global log directory.

## Reproduction and acceptance

Build the checkout's `gwt` and `gwtd`, then run
`scripts/diagnostics/issue-5543-browser-check.ps1` with `-Label after`, the same
`-ProjectRoot` and `-SeedHome`, and `-ObserveSeconds 3600`. Register that command
in `verify.plan` and in both `verify.run.commands` and
`verify.run.headed_e2e_commands` for canonical evidence. Extra Playwright
arguments, including the canonical reporter, are forwarded by the script.

The fixture copies the Issue cache, Sessions, Work ledger and Issue links into
a fixed seed. Each GUI receives a fresh HOME and an independent copy. PM and
Issue Monitor are disabled, background indexing is disabled, and the startup
restore acknowledgement is suppressed so copied Sessions cannot launch real
agents. Application load requests and replies use the real WebSocket/backend.
Only the runtime-code directory is shared. The fixture stops its own GUI PID.

Both headed Chromium themes verify cached rows before the held Work decode,
the later enrichment's visible update, a warm request, and twelve concurrent
requests. A `list_windows` response must arrive while those requests are
pending. Dark mode observes bounded bursts for at least one hour; light mode
observes at least one minute. Every load must succeed within the real
60-second budget, with zero console/page errors and zero timeout-ledger rows.
The first request must have completed timing records for all five initial
response stages. JSON, screenshots and raw logs remain in the fixture's local
artifact directory; the canonical run records the source fingerprint and
measured headed results.

The fixed corpus has 1,613 Issues, approximately 5,920 Session files and 1,250
Work items (19 MB ledger). This is a debug build on a shared host with twelve
concurrent reads, not a repeat of the installed v9.123.2 GUI with twelve live
agent panes. The baseline and fixed-source runs share input data; their host
load and OS filesystem-cache state are not controlled. Do not attribute all
wall-time differences to CPU savings or claim production-fleet coverage.

User Verification Result: n/a (autonomous). The separate Agent Visual Check
is established by the fresh canonical headed run, not by this document.
