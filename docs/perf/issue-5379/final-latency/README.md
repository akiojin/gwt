# Issue #5379 final AC-1 measurement

This is the AC-1 remeasurement. Issue #5816 had already landed its CPU cap relief before it ran. The workload, wrapper, and 100ms assertions match the earlier observations: five real custom Agent PTYs, 8192 ASCII bytes per second each, a 1800-second dark observation, and a separate 10-second light functional observation. The binary was the checkout `target/debug/gwt.exe`, the same dev profile used for the earlier runs.

| Observation (dark, 1800s) | Run 1: snapshot encoded on GUI loop | Run 2: deferred snapshot encoding |
| --- | --- | --- |
| Checkout HEAD | `8ad906a8978f3c5592985931379413f8d7e15174` | `08d1c23069bde8ecff817467eb4676dcb631457d` |
| Binary SHA256 | `57632C91…7E67DE6` | `0EDBAFA0…CDAA9B9` |
| Actual interval | 1,801,055ms | 1,800,880ms |
| Completed sequences | 1800 for all five PTYs | 1800 for all five PTYs |
| All native dispatch: samples / stalls / maximum | 84,264 / 5 / 225ms | 74,104 / 0 / 14ms |
| ProjectOwnerSnapshot: count / stalls / maximum | 3,354 / 5 / 225ms | 2,464 / 0 / 7ms |
| ProjectOwnerEvent maximum | ≤ 3ms | 2ms |
| Independent output batches: count / stalls / maximum | 5,645 / 0 / 3ms | 6,536 / 0 / 0ms |
| Result | FAIL (`output dispatch stays below 100ms`) | PASS |

A stall is one event loop dispatch of 100ms or more. Both runs had zero browser errors and zero cleanup errors. In both runs the light functional observation passed.

## Root cause of the Run 1 failure

Each stall in Run 1 was a `ProjectOwnerSnapshot` dispatch. Outside the stalls, the per-minute summaries show that callback at 50–90ms. A snapshot replays up to 5,000 scrollback lines, which is about 850KB of JSON. The tao thread serialized that JSON once for every target of the snapshot. A standalone measurement in the dev profile took 31ms per serialization and 3.5ms for the base64 encoding. Run 2 routes `TerminalSnapshot` through the deferred encoding path that Active Work patches already used (Issue #4411). Each client sender now encodes the snapshot once, outside the queue mutex, and the routing identity and stream position are unchanged.

## Files

- `run1-snapshot-on-gui-after-{dark,light}.json.gz` and `run1-shutdown.json` keep the failing run.
- `run2-deferred-snapshot-after-{dark,light}.json.gz` and `run2-shutdown.json` keep the passing run.
- `preparation-failures.json.gz` keeps earlier short harness-preparation failures. They are not acceptance observations.

To reproduce, run this from the repository root after `cargo build -p gwt --bin gwt --bin gwtd`. Launch PowerShell directly. A `timeout` wrapper on Git Bash breaks the process ancestry that the managed-browser preflight requires.

```text
CARGO_TARGET_DIR=target pwsh -NoProfile -File scripts/diagnostics/issue-5379-browser-check.ps1 -DurationSeconds 1800 runtime-output-batch-live.spec.ts
```

The deterministic Agents do not exercise provider inference, hooks or builds. Each arm is a single observation, and the two runs differ only by the deferred-encoding commit and the evidence-only commits between them.
