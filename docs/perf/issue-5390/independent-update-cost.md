# Issue #5390: unchanged graph updates and PR inventory reads

## Scope and preserved evidence

This work resumes the existing performance scope independently of the shared
GUI restart. It does not change process lifecycle, notification cleanup, state
classification, identity, claim, or measurement admission contracts.

The starting source is `4bcfa037874ea8be7ba026018d572cf6dcf52d6e`.
The earlier source `0869655a3f59098eebffb033292ab523983bb9f0`, its
17-command verification record and successful caller doctor remain separate
evidence. New build outputs and raw logs use
`target/independent-perf-20261103/`; the earlier debug candidates and recovery
packet are preserved:

| Artifact | SHA-256 |
| --- | --- |
| Previous `target/debug/gwt.exe` | `9ccdfbd30c443a56d91896bf68f7b5b5125a0895cf37b9074d4416bf26813a50` |
| Previous `target/debug/gwtd.exe` | `083e45ad0475d978148740d4df602238f1390ee0b82e2557cf15b5413b0c4bd5` |
| Recovery `packet.md` | `41f717d5221fddd9ef4c3e03f79a987a86b57bb3fbf737f88f20d88dcc2d8a71` |
| Earlier verification record | `145e89fa39dd242c9b32ff5cb6e8701589caf23aded7f26bedbebda8eeacd7a7` |

## Unchanged sparkline mutation

The existing Works fixture renders two live agents and their fleet graph.
Each of 20 telemetry flushes supplies fresh arrays containing identical values.
Counters observe the original three SVG elements and `createElementNS` after
initial rendering. These are deterministic operation counts, without elapsed
time assertions.

| Operation over 20 flushes | Before | After |
| --- | ---: | ---: |
| Original SVG `replaceChildren` | 60 | 0 |
| SVG nodes created | 1,860 | 0 |
| Host mounts | 0 | 0 |
| Layout requests | 0 | 0 |
| Pending animation frames | 0 | 0 |

`drawSpark` previously removed and rebuilt its children on every render.
It now retains the normalized 30 values and effective maximum in a WeakMap
per SVG. It skips drawing only when both inputs match. Updating a bucket
redraws the affected graph and any sibling whose shared scale changes;
introducing a missing bucket still updates the known-value count and circles.
The cache is stored only after a completed draw and does not retain SVG keys.

Validation: existing related baseline 13 passed; operation-count RED observed
60 clears and 1,860 created nodes; corrected related suite 14 passed;
full frontend suite 1,717 passed; independent review found no blocking issue.
The bare Node invocation initially lacked linkedom. The first full wrapper run
used a temporary directory outside the checkout and failed the existing
Git merge-base contract (1,716 passed, one failed). The unchanged full suite
passed with its task-specific temporary directory inside this checkout.
Those setup failures remain in the raw logs; no test was skipped.

The counters exclude preview clones, other DOM work, and native rendering.
They do not prove that this change resolves the observed shared gwt CPU load.

## Failed PR inventory reuse

Candidate discovery and autonomous progress both request the open-PR branch
inventory. The existing successful response cache can conceal the duplicate
call; failed reads are not cached there. The existing Unix-only 150-in-flight
scan fixture measures the failure case in an isolated Linux target.

| Measurement | Before | After |
| --- | ---: | ---: |
| Failed inventory calls in the first scan | 2 (RED: expected 1) | 1 |
| Cumulative calls after a second failed scan | Not reached by the RED assertion | 2 |

The candidate loader now reports its inventory Result to a server-local slot.
Autonomous progress reuses that Result within the same scan. The public loader
and progress APIs remain compatible wrappers. No persistent state or wire format
changes. An unread slot still performs the original budgeted read; successful
empty inventories and errors remain distinct. Errors retain the typed stale
degradation and previous Implementing phases. A new scan starts with a new slot.

The actual two-scan failure regression passed with the original 150 in-flight
issues, preserving their phases and the initial launch proposal. The second
scan's pending proposal is retained from the first; it is not evidence of a
newly generated launch. The original successful-empty inventory test also
passed. Independent review found no blocking issue. Raw baseline, RED and GREEN
logs remain in the isolated task directory.

The observed ledger cadence near 45 seconds is not correlated to a particular
set of three 15-second operations. No timeout is extended, and no failure is
cached across scans.

## Remaining acceptance

Actual isolated headed Chromium dark/light checks, zero console/page errors,
Mac input/scroll behavior, installed delivery, the fleet duration/CPU/RSS
criteria, current-head canonical verification, and Ready/CI delivery remain
unproven for this slice. The shared GUI restart conditions still apply.

User Verification Result: n/a (autonomous)

Agent Visual Check: fail(existing shared-GUI ancestry admission; no new headed evidence)
