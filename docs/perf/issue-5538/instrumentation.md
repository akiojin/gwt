# Completed GUI handler profiling

Issue #5538 adds `type: gui_dispatch_summary` records to the existing
`~/.gwt/logs/perf/perf-YYYY-MM-DD.jsonl`. Existing watchdog and diagnostic
warnings continue to detect an unfinished handler; this collector records
completed handlers strictly longer than 50ms.

The outer dispatch guard includes callback preparation and early returns.
Frontend events use their specific event kind, including the fast input path.
One row per observed kind is emitted when a completed callback crosses the
60-second monotonic window boundary. `window_ms` records the actual interval;
an idle or blocked loop can therefore produce an interval longer than 60 seconds.
An empty interval emits a zero-count `gui:event_loop` liveness row. Its absence
must not be interpreted as zero stalls.

The `dispatch` object contains GUI `pid`, `window_ended_at`, `window_ms`,
`count`, `total_ms`, `p95_ms`, `max_ms`, `pending_max`,
`pending_unknown_count`, and `dropped_windows`. A nonempty row's target is
`handler:<event kind>`. The percentile is exact nearest-rank p95 over the
**completed handlers longer than 50ms in that window**, not over all dispatches.
Do not average minute p95 values to estimate a fleet-wide percentile.

Pending is sampled after the measured handler, excluding that active event.
It counts the application's foreground, layout and background queue entries,
including their existing coalescing behavior. It does not claim to count the OS
window-message queue. A busy queue mutex produces an unknown sample instead of
waiting; the summary preserves its count separately from `pending_max`.

The GUI stores slow durations in memory and makes one nonblocking bounded
channel transfer per interval. Serial handlers longer than 50ms naturally limit
samples to fewer than 1,200 per ordinary minute. Sorting, serialization and
appending run on one dedicated worker with its own perf sink, without taking
the global mutex used by foreground route timers. The queue holds at most two
batches. When full, the collector drops that interval and reports the number
of dropped windows in its next delivered summary. The existing perf kill switch
disables this worker. No event payload, prompt or terminal output enters these
records.

`collector-cost.json` preserves an alternating ten-round comparison of identical
handler work with and without the fast collection path. No increase was detected
in that sample; the negative delta is timing noise and is not a speedup claim.
This is a scoped cost measurement, not a hard latency bound or a measurement of
whole-fleet CPU. Slow aggregation and writer backpressure have separate behavioral
tests. Real-provider results and the historical fleet baseline are separate
artifacts so their workloads and collector versions cannot be confused.

Reproduction uses `cargo build -p gwt --bin gwt --bin gwtd`, followed by
`pwsh -NoProfile -File scripts/verify-issue-5538.ps1 --headed`. The opt-in
`-Fleet` variant uses eight real builtin Codex agents in disposable Git projects,
existing Codex authentication in a private temporary home, and at least thirty
complete collector windows after all eight providers start their fixture
commands. Those commands print heartbeats and write only private progress
receipts; progress is independent of Codex's delayed tool-result delivery.
It never replaces the installed GUI. Its finalizer stops only owned
process identities and removes the private authentication copy. Artifacts stay
in the reported temporary directory; only sanitized measurement evidence belongs
in this repository.
