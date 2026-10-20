# Issue #5504: idle clock and pane exit workers

## Scope and result

The status clock now uses one timeout aligned to the next wall-clock second.
Hidden documents cancel the timeout; returning to a visible document renders
the current time immediately. The colon styling and reduced-motion CSS remain
unchanged.

Pane status workers block on process exit instead of taking the pane lock every
100 ms. Input and kill remain available while waiting. Unix waits preserve the
unreaped child until its receipt is published under the state lock, preventing
PID reuse between exit observation and control operations. The output reader is
reserved before status observation can release the PTY descriptors.

Both themes passed the real headed Chromium test, including clock updates,
hidden/visible transitions, eight real Shell panes, and zero console/page errors.
The clock's recurring RAF callbacks fell to zero. These samples do **not** prove
an improvement in total GUI CPU: both process measurements were higher in the
controlled after sample.
The wider fleet CPU target remains owned by Issue #5385 / SPEC #5390.

## Conditions

- Windows, debug builds from this checkout, 1440 x 900, no reduced motion.
- Fresh HOME per process; PM auto-start and Issue Monitor disabled.
- Eight real Shells left idle at their prompts, then a five-second settling wait
  and a requested ten-second observation, in dark and light themes.
- Automatic background indexing is disabled for the primary comparison with
  the existing `GWT_DISABLE_BACKGROUND_INDEX=1` opt-out. This prevents unrelated
  index bootstrap from maintaining the shared runtime during this test.
  Runtime-health sampling and other default background services remain enabled.
  This is an idle terminal workload, not a claim that all background work is
  quiescent. Product defaults are unchanged.
- One frontend page; renderer CPU is the sum of stable renderer PIDs in the
  owned Chromium instance. The current checkout removed its native WebView;
  these values measure the browser implementation of the same embedded frontend.
- Percentages use one logical core as 100%. The PowerShell process snapshots
  bracket a longer interval than the renderer sample; each percentage uses its
  own measured elapsed time. Raw CPU seconds alone are not comparable across
  different intervals.
- Both variants use the same callback-name counters and DOM observer, without
  allocating stack traces on each frame.

The producing baseline is commit `e7778994a2cf451f9913208407425819380920ff`
plus a behavior-preserving export of the clock function for its regression test.
The baseline executable embeds that exact 51,525-byte clock source, SHA-256
`51a642d4b6bf60333feb3d742c9b46a0dd7aa6afc322ff301f46b0de8a94a4cc`.
The after binary contains the working-tree fix. Executable hashes and clock
asset hashes identify each variant independently of the common starting HEAD.
The final committed source is subsequently rebuilt and verified canonically.

## Controlled measurements

The before and after executables were run consecutively with background index
bootstrap disabled, no additional build/test job started by this execution,
and the other conditions above held constant. Raw evidence is under
[controlled](controlled/).

| Theme / variant | gwt CPU seconds / elapsed | gwt one-core % | Renderer CPU seconds / elapsed | Renderer one-core % | Clock RAF / timeout callbacks |
| --- | --- | --- | --- | --- | --- |
| Dark before | 17.718750 / 13.117 s | 135.08% | 1.280427 / 10.003 s | 12.80% | 388 / 0 |
| Dark after | 21.250000 / 12.000 s | 177.08% | 1.918138 / 10.013 s | 19.16% | 0 / 10 |
| Light before | 21.484375 / 12.292 s | 174.78% | 1.605024 / 10.016 s | 16.02% | 503 / 0 |
| Light after | 21.171875 / 11.860 s | 178.51% | 1.798432 / 10.015 s | 17.96% | 0 / 10 |

The before samples observed ten/nine clock text changes; both after samples
observed ten. Aggregate CPU did not improve in this comparison. The wide spread
between repeated before measurements also prevents attributing the difference
to this patch. No total-CPU reduction is claimed. The accepted mechanism changes
are zero clock RAF callbacks and removal of periodic pane status wakeups, with
normal exit/input/output behavior preserved. Full fleet performance remains a
separate acceptance target in Issue #5385.

## Earlier paired diagnostic measurements

The saved before and after executables were run consecutively from this
checkout, with no additional build or test job started by this execution.
Each run uses a new isolated home and cleans up its owned process. Raw snapshots,
timestamps, hashes, conditions, and callback counts are under [paired](paired/).

| Theme / variant | gwt CPU seconds / elapsed | gwt one-core % | Renderer CPU seconds / elapsed | Renderer one-core % | Clock RAF / timeout callbacks |
| --- | --- | --- | --- | --- | --- |
| Dark before | 21.437500 / 15.174 s | 141.28% | 1.490448 / 10.001 s | 14.90% | 439 / 0 |
| Dark after | 15.546875 / 13.963 s | 111.34% | 2.060228 / 10.016 s | 20.57% | 0 / 10 |
| Light before | 19.562500 / 13.561 s | 144.26% | 1.547121 / 10.004 s | 15.46% | 457 / 0 |
| Light after | 27.281250 / 20.105 s | 135.69% | 2.155818 / 10.038 s | 21.48% | 0 / 10 |

All four samples observed ten clock text changes. gwt's normalized CPU share
was lower after the change, while renderer share was higher. This pair kept
automatic indexing enabled. Durable refresh-broker records showed runtime
maintenance attempts by this isolated baseline process between its CPU windows;
they do not establish CPU attribution or overlap with those windows. This
unnecessary side effect motivated the controlled opt-out above. Neither CPU
difference is attributed solely to this patch.

## Earlier diagnostic measurements

The four JSON files in this directory preserve the first attempt. Its baseline
overlapped an already-started RED dependency compilation, whereas its after run
did not. It is excluded from the primary before/after comparison.

| Theme | Before gwt / renderer one-core % | After gwt / renderer one-core % |
| --- | --- | --- |
| Dark | 83.61% / 6.90% | 155.58% / 19.46% |
| Light | 85.51% / 10.32% | 116.56% / 17.31% |

The stronger apparent regression in that attempt is not used as evidence of a
code effect. The repeated baseline itself reached 141–144% gwt CPU. Both sets
are retained instead of discarding unfavorable observations.

## Reproduction and acceptance

Build this checkout with `cargo build -p gwt --bin gwt --bin gwtd`.
Run `pwsh -NoProfile -File scripts/verify-issue-5504.ps1 --headed`.
The driver uses the checkout executables, an isolated home, stable managed
hooks, PM/Monitor opt-outs, an owned websocket anchor, and exact-PID cleanup.
It hashes the clock asset actually served by that fresh process.

The default evidence directory is `target/verification/issue-5504`, so canonical
verification does not rewrite tracked comparison files. For an intentional
comparison capture, pass `-EvidenceDirectory <directory>` and use `-Phase
baseline` only with a known before executable built by this checkout. Restore
the after executable before verification and delivery.

The driver disables automatic background indexing only in its isolated process;
PM/Monitor remain disabled too. The shared-runtime maintenance evidence was
added to existing Issue #5429, without changing the product indexing policy.

- AC-1: the existing frontend runtime suite tests second boundaries, delayed
  callback re-alignment, cancellation, immediate resume, and initially hidden
  startup; live headed tests cover both themes.
- AC-2: terminal tests cover concurrent wait/control and shared success/failure,
  actual kill/exit receipts, and descriptor cleanup. Pane runtime tests preserve
  input, stream ordering, confirmed exit, and output draining independently of
  EOF. Linux/macOS CI also exercises the Unix non-reaping wait path.
- AC-3: the raw before/after CPU measurements and their limits are recorded
  above. No total-CPU improvement threshold is asserted by this Issue.

User Verification Result: n/a (autonomous)

Agent Visual Check: pass
