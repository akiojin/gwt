# Issue #5502 PM Reports idle CPU measurement

Build `gwt` and `gwtd` in this checkout before each measurement. On Windows,
run `node scripts/verify-issue-5502.mjs --label before --measure-only` before
the source change, then rebuild and run
`node scripts/verify-issue-5502.mjs --label after` after the change.

The harness launches only this checkout's `target/debug/gwt.exe` with
`--no-tray --no-open`, a fresh HOME, PM auto-start off, and Issue Monitor
disabled. It uses the stopped PM Reports fixture from `verify-issue-5023.sh`
and the Windows launch and hook-authority conventions used by existing
browser checks. It retains the shared runtime directory only; project and
session state are synthetic. It never starts an implementation agent.

Each run seeds a 524 KiB Issue Monitor preferences file, 691 dead host
directories containing 4,047 runtime sidecars, and 4,047 foreign historical
Session ledgers. Once the Reports pane is ready and startup has settled, it
reseeds the fixed input and sets a valid `claim_diagnostics` entry's
`last_failure` string to fix the JSON byte count. Its synthetic Issue has no
inbox or queue row, so the diagnostic survives preferences serialization
without appearing as a live error or creating work. The harness checks the
diagnostic payload's exact size and sidecar counts before and after sampling.
The preferences file starts at exactly 524 KiB; its live census timestamp
may change the envelope size by at most 32 bytes as fractional digits vary.
Both actual byte counts are retained. It counts
only directories containing sidecars against 691 and records the GUI's
additional empty host directories separately. The fixed old process identities do not describe live
agents. A fresh release cache prevents GitHub queries during the sample.

A headed Chromium opens the PM Reports pane using the dark theme. After
five seconds of settling, the harness records six samples at five-second
intervals over 30 seconds. Windows `GetThreadDescription` selects only
threads named exactly `tokio-rt-worker`; `GetThreadTimes` supplies their
kernel plus user CPU seconds. Retained handles preserve the final CPU times
of workers that exit; newly observed workers use their creation FILETIME to
separate recycled thread IDs. Samples record the observed and exited worker
counts and thread-discovery races. The report also includes whole-process
CPU seconds as context. A worker created and exited entirely between two
polls cannot be discovered, so the named-worker measurement is a sampled
lower bound; compare the whole-process counter as a cross-check.
This measures the owned GUI process only, with browser work and other gwt
instances excluded from the CPU counter. Machine load can still affect the
timing and comparison, so retain both raw reports.

Outputs default to `target/issue-5502/before.json` and `after.json`, with a
screenshot and startup log beside each. JSON records the checkout SHA,
binary size and modification time, fixture, owned PID, actual elapsed times,
worker thread IDs, and CPU seconds. Compare the final sample's
`worker_cpu_seconds` between reports from the same machine and fixture;
`worker_cpu_seconds / elapsed_seconds * 100` is one-core CPU utilization.

After the CPU sample, the default command runs the existing
`pm-chat-live.spec.ts` tests in headed Chromium for both dark and light
themes. Those tests assert the visible Reports behavior and zero console or
page errors. `--measure-only` omits that E2E run. Add another existing test
with `--spec <filename.spec.ts>`. Arguments after `--` go unchanged to the
Playwright runner, including the canonical verification reporter. Pinned
Playwright and its Chromium browser must already be installed, as with the
other Windows verification scripts. The default dependency cache is
`%LOCALAPPDATA%/Temp/gwt-playwright-<pinned-version>/node_modules`, independent
of the command's isolated TEMP; `GWT_PLAYWRIGHT_NODE_MODULES` or
`GWT_PLAYWRIGHT_DEPS_DIR` can override it.

The harness records agent evidence. The autonomous launch uses
`User Verification Result: n/a (autonomous)`. It closes its browser and
terminates only its recorded GUI PID and children on success or failure;
the isolated HOME remains available for diagnosis.

The reader checks a fixed set of source-file metadata, a semantic preferences
fingerprint, daemon facts, and live child identities. Session and Issue-cache
trees use filesystem notifications rather than a recursive scan on every poll.
The preferences fingerprint ignores scan time and host-census observation time;
counts, windows, queue decisions, and other preferences remain inputs. Peer and
PR refresh deadlines trigger re-evaluation; status age advances from cached
status. Unavailable filesystem watches fall back to uncached reads and retry.
Each new connection receives a snapshot, and repeated identical content is not
sent again.

Early unfiltered trials measured 21.234375 and 25.0625 worker CPU seconds.
Those runs included unrelated frontend pollers and automatic index provisioning,
so they are retained as investigation notes rather than the final comparison.

The final CPU comparison disables automatic background indexing with the existing
`GWT_DISABLE_BACKGROUND_INDEX` option, directs pip cache into the isolated HOME,
and forwards only `load_pm_reports` during the sample. Raw JSON records forwarded
Reports requests and each excluded request kind. Backend background work remains
in the worker aggregate. After sampling the measurement browser closes; all six
E2E cases run on normal unfiltered connections in dark and light themes.

| Controlled 30-second sample | Before | After |
| --- | ---: | ---: |
| Worker CPU seconds | 25.109375 | 13.203125 |
| Elapsed seconds | 30.540 | 30.029 |
| Reports requests forwarded (including collector startup) | 8 | 7 |

Worker CPU seconds decreased by 47.42% in this one pair. This is a controlled
route comparison, not an all-fleet CPU or 60-minute operational claim. Both files were measured
from this checkout before its delivery commit; their binary timestamps and sizes
identify the original and changed build. The final canonical verification record
and mandatory CI identify the delivered source SHA. Raw evidence is in
[before.json](before.json) and [after.json](after.json).

A later canonical run (`vrr-b172edcfc6c34abdaf0c2227739a56d2`) used the same
fixture and request controls and measured **24.421875 worker CPU seconds over
30.014 seconds**, with seven forwarded Reports requests. Its raw result is
[canonical-after.json](canonical-after.json). It also passed all six headed
E2E cases. This repeat has no contemporaneous before sample, so it is not a
second paired improvement estimate. The spread between changed-build samples
shows substantial aggregate CPU variability; the first pair does not establish
a stable 47.42% reduction. The regression tests establish the narrower contract:
unchanged inputs skip store/census reloads and unchanged content is not resent.
