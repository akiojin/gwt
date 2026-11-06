# Issue #6806: terminal input continuity

Owner: [Issue #6806](https://github.com/akiojin/gwt/issues/6806). Agent: サチ.
Baseline: `a297cd6872bb593440582231ac5e507b6507b69f` (`origin/develop` at intake).
Environment: Windows; checkout `E:/gwt/work/issue-6806`.

## Failure and repair

The deterministic regression holds the physical PTY writer, then admits ordinary
input. Before the repair, ordinary input retains `input_state` while waiting for
the writer. An input observation cannot complete until the writer is released.
In production, a snapshot takes the Pane lock before observing input state; the
output reader also needs Pane. A child blocked writing output while it is not
reading stdin can therefore complete the I/O wait cycle.

`pty::tests::waiting_ordinary_input_keeps_observation_and_protected_ordering_live`:

- RED: one executed test failed with `writer wait blocked input observation`,
  `None` versus `Some(1)`, after the five-second liveness bound (5.07 s total).
- GREEN: the same test passed (0.04 s). Observation and a protected reservation
  complete while the physical writer remains held. Pending input is queued once
  behind that reservation and written exactly once when it is released.

Ordinary chunks retain their own serialization mutex. Input state is released
before acquiring the physical writer, then protection and the active generation
are checked again. Input revision and human activity are recorded once. If a
reservation starts during the writer wait, the input joins that reservation's
queue; this is the fail-closed choice that preserves atomic body/submit writes.
No input content is added to production diagnostics.

The real-PTY test also exposed an independent ordering failure after three
minutes and a browser reconnect. The quiet control pane received mixed ASCII
and paste lines (`ASCII-cortnoPASTE-control-end`, `dl-en`) instead of the three
sent messages. The assertions were retained. The trace showed ordered ASCII,
Enter, IME, Enter and paste onData categories with an open socket and no drops.
The first trace did not contain WebSocket payloads; the fixture now captures
only its synthetic input frames to check that boundary directly.

The GUI dispatched each terminal chunk through an independent blocking HTTP
worker. The owner likewise executes input on workers. A physical writer mutex
cannot recover the browser's order once requests have overtaken one another.
The follow-up regression holds the existing queued worker and requires the
browser adapter to dispatch only one input until its exact owner ACK arrives.
This regression failed with three dispatched workers instead of one, then
passed after the repair. The existing runtime contract module passed all 29
tests, including the four input-lane cases.

The GUI now keeps one input in flight per pane. Its internal UUID distinguishes
that command's ACK from mobile observations or obsolete completions. A healthy
inventory refresh preserves a matching lane; connection/owner/incarnation changes
and dispatch or command failures discard the unwritten tail with failure results.
There is no retry, persisted input queue, or new wire protocol. Other panes keep
their independent dispatch. The unsent-input hint stays busy until the lane has
settled, preventing an intermediate empty composer observation from waking an
automatic prompt between queued human input chunks.

Alternatives considered were making input observations incomplete with try-lock,
moving snapshots to more workers, and removing the I/O/state lock dependency.
The last option removes the cycle without new projection or worker machinery.

## Verification

- `cargo test -p gwt-terminal --lib`: 107 passed, including generation revocation,
  protected input, ordinary revision, cursor responses and the new regression.
- Node regressions: 174 passed across `terminal-viewport-reflow`,
  `terminal-output-buffer`, `socket-project-scope`, `terminal-wheel-scroll`,
  `issue-preview-pane` and `works-surface`.
- `cargo test -p gwt --bin gwt app_runtime::tests::runtime_contract_tests:: -- --test-threads=1`:
  29 passed. The FIFO regressions cover exact ACK sequencing, preservation across
  healthy inventory, unrelated/mobile ACKs, failed tails, worker admission
  failures and owner connection loss.
- Preliminary headed frontend control: disconnected-input dark/light passed.
  An embedded frontend-only load probe ran 182,013 ms with 12 ASCII checkpoints
  and zero console/page errors. This used a WebSocket fixture, not a real PTY.
- The owner Issue's delivery comment records the exact source SHA, fresh
  canonical verification record, real-PTY headed measurements and PR/merge.
  Preliminary failures are retained in the isolated runner artifacts.

The live test reads receipt files written by the child after consuming stdin,
and checks its `ACK` response in the terminal. It covers direct ASCII typing,
Chromium IME composition/commit, and xterm's DOM paste handler, with a streaming
pane and a quiet second pane. It records actual duration and checks input after
resize and a fresh page/WebSocket connection. Clipboard and IME events are
automated browser events, not a physical keyboard or OS clipboard test.

## Limits and delivery

The lock regression is decisive evidence of this failure path. The original
user session's exact trigger was not captured. Real provider TUIs, other OSes,
and durations beyond the measured window remain unmeasured. PM wake ACK retention
and synchronous ConPTY resize were separate unproven candidates and are untouched.
An independent read-only review found no blocking defect in the new lock order,
exact-ACK FIFO, failure discard, snapshot preservation, or existing ownership
checks. The existing protected body-commit path can still hold input state during
its authorized write; it was not reproduced here and is outside this ordinary
writer-wait repair. The PM's production WebSocket/Monitor stall observations do
not establish that every reported freeze has this same cause.

The current successor session re-runs canonical verification rather than adopting
the predecessor's incomplete record. Raw RED logs remain in
`target/issue6806-red.log` and `target/issue6806-fifo-red.log`; their decisive
failures are recorded above so the evidence survives local artifact cleanup.

Issue #6730's history/snapshot repair is separate; no source was copied from it
and its acceptance is not claimed here. Source merge into develop and installation
of a released running build are separate delivery events.

User Verification Result: n/a (autonomous)
Agent Visual Check: recorded separately in the owner Issue's canonical evidence.
Coverage: unmeasured locally; the required CI gates decide coverage for the PR SHA
(gwt-core + gwt 90%, remaining domain crates 80%, configured outer-shell exclusions).
