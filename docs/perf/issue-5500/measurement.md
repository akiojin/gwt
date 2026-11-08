# Issue #5500: capacity sampler CPU

## Method

- Windows, debug checkout binaries, `--no-tray --no-open`, fresh HOME.
- One open checkout project; PM auto-start and Issue Monitor disabled.
- Both runs copy the same frozen Session fixture: 3,435 runtime JSON files,
  1,612,683 bytes. The source Session directory is never modified.
- Identify the owned GUI's `gwt-agent-capacity` thread using
  `GetThreadDescription`. Measure its `TotalProcessorTime` delta for 60 seconds.
- The owned process is stopped in `finally`; production GWT remains running.
- Reproduce with `GWT_CAPACITY_SESSIONS_FIXTURE` pointing at a copied sessions
  directory and `pwsh -NoProfile -File scripts/verify-issue-5500.ps1 -MeasureOnly
  -MeasurementPath <output.json>`.

The baseline is checkout commit `166dfefc94b13f4b3124b02e015c7e61a05f2c68`.
Raw observations, binary hashes, PID/TID, and timestamps are in `before.json`
and `after.json`. CPU seconds measure this sampler
thread, rather than all GUI worker threads or wall-clock response time.

## Results

| Observation | Elapsed seconds | Thread CPU seconds | Average cores |
| --- | ---: | ---: | ---: |
| Fixed fixture, before | 60.152982 | 1.140625 | 0.018962 |
| Fixed fixture, after | 60.524044 | 0.984375 | 0.016264 |
| Fixed fixture, after develop integration | 60.336575 | 0.703125 | 0.011653 |

Sampler thread CPU decreased by 0.156250 seconds (13.70%); elapsed-normalized
average cores decreased by 14.23%. This is one before/after observation with
continuing OS/compiler activity, rather than a statistical throughput estimate.
Whole-process CPU was 63.562500 and 80.015625 seconds respectively; this change
does not establish a reduction for the other GUI worker threads.

The original `after.json` predates the Issue #5489 census integration. A fresh
observation at `38770ebcb` is preserved in `integrated-after.json`, including
the binary hash and hashes of all four production source files. It uses the
same 3,435-file, 1,612,683-byte fixture, disabled PM/Monitor, and fresh HOME
on the original profile volume. Sampler CPU was 0.703125 seconds over
60.336575 seconds: 38.36% less CPU time and 38.54% fewer average cores than
the original baseline. Whole-process CPU was 40.828125 seconds. These runs
occurred amid other compiler/OS activity; this additional single observation
does not isolate the effects of this Issue from the integrated census change.

An independent production observation (PID 32708, TID 29756,
2026-10-15T07:52:46.5812700Z to 07:53:46.7665736Z) consumed 7.921875 CPU seconds
in 60.175254 seconds, or 0.131647 average cores. Its live project/Session state
differs from the fixed fixture, so it is context rather than the comparison
baseline.

## Scope and interpretation

One host tick now refreshes the complete open-project set once, with one target
walk budget and one atomic snapshot write. A machine sample can be reused at the
next five-second tick, for less than ten seconds since completion, while its
original 30-second sensor expiry still applies.

The capacity path shares one explicit `ProcessesToUpdate::All` refresh between
GUI CPU, liveness evidence, and the Session census, after all sidecars are read.
Named PID resource/role refreshes remain necessary. On Windows, sysinfo 0.39.6
internally enumerates PIDs even for `ProcessesToUpdate::Some`; this change does
not claim that every underlying OS enumeration has disappeared.

RSS, role, and legacy attribution use fresh scalar probes and matching process
start identities. Missing scalar metadata cannot inherit an old positive RSS
or erase a legacy child's snapshot presence and associated uncertainty.

The duplicate-stop census keeps its fresh authority checks. Its Issue #5489
cache is not replaced with capacity's time-limited observation. Prefs parse
caching belongs to Issue #5499. Full fleet CPU and the 60-minute acceptance
remain with Issue #5385; this sampler measurement does not establish them.
