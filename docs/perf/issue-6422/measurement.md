# Issue #6422: Windows progress after the window ACK

This is a gwt runtime investigation applicable to every managed project.
It delivers measurements and a decision packet; it does not change priority,
CPU caps, launch deadlines, initial-activity monitoring or product behavior.
The 120-second interval below is an observation window, not a provider SLA.

## Finding for the PM

The evidence supports scheduling/priority contention in the BelowNormal
launch and hook process tree. It does not establish one exclusive cause.
An exact native window and ACK can precede useful provider/hook progress
by minutes. A provider process can also exist before its first receipt is
written. In the timeout run, SessionStart reached the GUI, but its CLI
waited on Git descendants and never reached UserPromptSubmit before the
existing 180-second monitor closed the candidate.

No product optimization was made between the timeout and recovering runs.
Recovery therefore is not an improvement result. The shared host's other
work remained uncontrolled; these are paired observations, not a randomized
benchmark or a shipping/fleet performance claim.

## Source, policy and method

All three retained runs used production HEAD
`4b34cd44ada8bbe71ea1cd1f2da78ce19e3ed1e6` and identical binaries:

- gwt SHA256: `6CB704BCB51675AAEA03FA138B587C4F47ADFA00076AF1111CFB411ECF075406`
- gwtd SHA256: `4916D5AC5C89F2933AAA3BC469CFF9D665E943BF18FC1B184729A40D64EF8A31`

The repository's existing offline Monitor fixture replays already-admitted
claim/delivery identities through the production locked prefs mutation and
scan wake. It launches real native PTYs, PowerShell, a native recorder
provider and real managed-hook commands in a disposable repository. It
does not contact a model or mutate external Issues. Neither upstream claim
creation nor actual model/provider internals are measured.

The opt-in `PostAckDiagnostics` mode leaves the existing fixture's tests
and assertions intact. Each theme gets a fresh HOME and GUI. Within it,
low and high use the same binaries and policy. A real Cargo build script
starts before measurement with its CPU work paused. Its 30 workers are
enabled after browser readiness and remain enabled throughout the measured
high-load interval. PID/start identity, work counters and OS CPU deltas
distinguish this work from a sleeping Cargo parent. Earlier low-load
recorders remain alive and idle; they are included in OS samples.

The separate Normal-priority observer starts before either delivery. It
uses Toolhelp process ancestry, ProcessThread state/wait/CPU accounting,
and query-only named Job handles. Thread and process birth times expose
identity reuse. Job handles close during each sample. Registry/query errors
and missing directories are recorded. The observer does not suspend or
reprioritize any target. Samples contain the GUI, daemon, gate, shell,
provider, hook/Git descendants, Cargo/build script and observer. Later runs
also include the Playwright CLI Node parent, but not its test worker or
Chromium renderer threads.

The target Agent tree was read back as BelowNormal with CPU flags 5,
CpuRate 800 (8%) and limit flags `0x2000` (kill-on-close). GUI, observer and
Cargo/build script were Normal. The retained canonical measurement reads
also show the verification runner's tracked ancestor cap disabled; no
target Agent cap was disabled. An empty remote `in_job` detail is only a
membership observation, not proof that every possible ancestor is uncapped.

## Joined chronology

Times are UTC. ACK is the first observation of matching delivery, claim,
binding and launched-window identities. The test additionally observes that
exact window's bound running Session and native terminal runtime under load.
Provider receipt creation/observation and hook authentication are separate
boundaries. Observation timestamps are upper bounds, not exact instruction
execution times.

| Run/load | Enqueue to ACK | ACK observed | Provider receipt created | First UserPromptSubmit authentication |
| --- | ---: | --- | --- | --- |
| exploratory low | 5.937s | 00:33:17.334 | 00:33:20.861 | 00:33:21.359 |
| exploratory high | 7.318s | 00:33:29.836 | 00:33:40.518 | absent before monitored close |
| paired dark low | 4.686s | 00:46:06.822 | 00:46:09.937 | 00:46:10.297 |
| paired dark high | 18.662s | 00:46:30.101 | 00:48:30.511 | 00:48:31.328 |
| paired light low | 4.727s | 00:49:00.369 | 00:49:03.481 | 00:49:03.858 |
| paired light high | 27.162s | 00:49:32.030 | 00:50:12.287 | 00:51:32.418 |

In exploratory high, SessionStart authenticated at 00:34:30.108, about
60.27 seconds after ACK. The CLI continued waiting on descendants. At
00:37:30.750, `monitor_startup_timeout` reported no initial prompt/tool
activity for 180 seconds. ACK disappeared between observations
00:37:30.840 and 00:37:30.966. Owned cleanup completed successfully.

Paired high observations continued for 120.333s (dark) and 120.345s (light)
after ACK. The load-idle state was observed at 00:48:30.585 and
00:51:32.406 respectively. First activity was authenticated just after
these idle observations. Do not report either as a first-hook success
under sustained high load. Dark's provider receipt lies near the stop
transition, whose exact OS scheduling time is not known.

The runtime has no direct PTY release timestamp. Source order is child
identity publication, policy application, then release in
`project_runtime::spawn_bound_pane`; `PendingPty::release` writes the gate
byte. Retained child birth times, correlated launch phases and the
`gwt.process.summary` ready stage bracket this transition. GUI phase
`captured_at` is the measurement time; its log timestamp can include worker
delay. A precise release-to-provider duration cannot be recovered from these
logs and is not claimed. The gate must exist before release and its shell
child is created only after release, giving the following OS birth-time
bounds. Exact PID ancestry and full timestamps are in `summary.json`.

| Run/load | Gate birth (release lower bound) | Target shell birth (release upper bound) |
| --- | --- | --- |
| exploratory low | 00:33:16.806 | 00:33:16.898 |
| exploratory high | 00:33:29.262 | 00:33:29.374 |
| paired dark low | 00:46:06.276 | 00:46:06.365 |
| paired dark high | 00:46:29.099 | 00:46:29.221 |
| paired light low | 00:48:59.874 | 00:48:59.955 |
| paired light high | 00:49:30.913 | 00:49:31.020 |

For example, paired dark's provider process was born at 00:47:39.646 but
its receipt was created at 00:48:30.511. Process birth therefore does not
establish when its first user-mode diagnostic instruction executed.

## OS observations and missing evidence

Exploratory hook child Git PID 46116/TID 10364 was observed Ready 40 times
with zero process CPU increment over the retained high interval. The full
OS archive continues beyond that interval: the process remained visible
until the monitored teardown. Another Git child, PID 47536, was also Ready
with zero CPU increment. The provider mostly waited with UserRequest;
its hook parent and Git ancestry are retained with exact PID/start times.
This supports runnable-work deprivation and downstream process waits,
rather than a busy provider-internal computation for this reproduction.
It is not a continuous scheduler trace or proof about an actual model CLI.

| Instrument/run | Samples | Median start gap | p95 start gap | Maximum start gap |
| --- | ---: | ---: | ---: | ---: |
| OS exploratory | 441 | 825ms | 1855ms | 2471ms |
| OS paired dark | 72 | 1694ms | 2076ms | 54298ms |
| OS paired light | 102 | 1843ms | 2055ms | 2097ms |

The exploratory controller had a 63.099s observation gap while requesting
load pause; workers=0 was not observed until 00:37:30.792. Its earlier
`loaded_observation_ended_at` is not the actual load-stop time. Paired dark
also has a 52.510s controller gap and a 54.298s OS sampling gap. Normal
priority does not guarantee an uninterrupted observer. No absence during
these gaps is evidence that the target did not run. The observer itself
consumed nonzero CPU (roughly 0.1-0.4 core over selected intervals), which
is retained and is part of both load conditions.

ProcessThread WaitReason values do not identify a particular file, lock,
pipe or child. Ready is evidence of being runnable at that sampled instant;
it cannot exclude intervening I/O or Job quota windows. Small aggregate CPU
use weakens a simple continuously-exhausted-quota explanation, but does not
rule out short throttling periods. The earlier WPR policy refusal
(`0xc5585011`, Issue #5390) was not bypassed or elevated; this packet uses
prestarted native OS snapshots, not native stacks/CSwitch events.

## Candidate assessment and smallest next step

| Candidate | Supporting / contrary evidence | Scope and next evidence |
| --- | --- | --- |
| Priority/scheduler contention | Repeated Ready with no CPU in the BelowNormal tree; Normal load advances; progress returns around pause | Strongest supported candidate, not exclusive proof; collect prestarted CSwitch/ReadyThread trace when policy permits |
| I/O, lock or child-process wait | Provider and hook parent wait while Git descendants exist | Stack/object waits are missing; a non-invasive owned-process stack during the stall is the smallest complementary probe |
| Provider internal CPU work | Native recorder has almost no CPU and is waiting on its hook | Does not explain this fixture; actual external-provider internals remain outside evidence |
| ConPTY response wait | Prior Issue #6393 evidence proved CPR delivered in 59us; this run progresses into SessionStart and Git waits | Not sufficient as the sole explanation; per-query causal timing remains unmeasured |
| CPU Job quota exhaustion alone | Actual 8% cap is retained, but stalled descendants accrue little CPU | Cap/priority interaction is unresolved; do not disable caps based on this observation |

PM default ruling (override available): retain current product policy for
this investigation. A static Normal-priority-before-release comparison with
the same cap, kill-on-close and deadlines is the smallest policy experiment
to consider under a separately approved owner/SPEC ruling. It is not
implemented here. Raising priority after release would miss existing
descendants and is not proposed. Any policy change needs GUI input/paint
responsiveness and descendant inheritance measurements; equal-priority
CPU contention can harm GUI responsiveness even with per-tree caps.

## Evidence and reproduction

`summary.json` contains exact claim/delivery/Session/window identities,
source blob hashes, process ancestry, per-interval CPU/state counts,
missing-data summaries and archive SHA256 values. The gzip files retain
unaggregated OS rows, Cargo progress, original test evidence and selected
non-secret launch/hook boundaries. `canonical-paired.json` retains selected
tool-generated evidence for `vrr-0c5963bbb3dc4c9bb6e6954ceac58f1b`:
five commands PASS, plan covered, headed dark 1/light 1, console/page errors
zero and owned cleanup errors zero. Screenshots were visually inspected.

The earlier `vrr-84338f573def4a19aca8cd2c7326f456` is failed evidence.
Its exploratory test incorrectly required eventual recovery after pause;
the product's existing monitor had already expired the candidate. That new
investigation-only condition was corrected; ordinary legacy launch tests
and product thresholds were not weakened. Its timeout and instrumentation
gaps are retained, not converted into product PASS.

Run `node scripts/diagnostics/issue-6422-packet.cjs` to verify retained
archive integrity and paired acceptance facts. For fresh measurements,
build the checkout's gwt/gwtd, then register two `verify.run` headed commands
using `scripts/verify-issue-5596.ps1 -Sequential -CargoLoad -LaunchCount 2
-PostAckDiagnostics`, distinct evidence directories and respectively
`--project=chromium-dark` / `--project=chromium-light`. Preserve inline
`GWT_VERIFY_PERFORMANCE=1` and the explicit checkout `CARGO_TARGET_DIR`.
Use a task-local TEMP/TMP/TMPDIR on a volume that meets admission floors;
this run used `target/gwt-verify-tmp` on E:.

AC-1 maps to the joined identity chronology and explicit release uncertainty;
AC-2 to paired raw OS/load rows and missing intervals; AC-3 to unchanged
policy plus prestarted read-only observation and the candidate limits;
AC-4 to the assessment and proposed next experiment. Investigation delivery
does not establish a performance improvement or a 120-second guarantee.

User Verification Result: n/a (autonomous)
Agent Visual Check: pass (measured headed dark/light; performance remains unresolved)
