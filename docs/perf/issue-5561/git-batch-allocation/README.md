# Issue #5561 Git batch allocation probe

This transient diagnostic measures the real `work_event_blobs_batch` implementation
with one identical frozen list of 749 origin refs / 589 unique commits. It uses a
select-all callback. It does not include worktree scanning, caller source-context
fingerprint maps, projection folding, persistence, or GUI memory.

The counting allocator delegates to `System`; counters are captured before each
JSON record is formatted. `live_bytes` and `peak_bytes` describe successful Rust
allocation requests, not Windows working set or allocator reserved pages.

The only copied implementation changes are stage capture calls and binding the
existing return expression long enough to capture `output_done`. The worktree
resolver stub returns the supplied checkout, whose `.git` must exist. This probe
does not claim workspace-home layout coverage.

## Reproduce

Extract this archive into a gwt checkout at
`5767b7cfa52db4a9990485a9c831f5612f10180d`. Archive entries live
under `target/`; they do not replace product files. Use Rust 1.99.1 and the recorded
Cargo.lock. All commit objects named in the frozen input must remain available in
the repository. The preparation script restores the temporary example and creates
the instrumented implementation from the selected saved product snapshot.

Run from the checkout root in PowerShell:

```powershell
$env:RUSTUP_TOOLCHAIN='1.99.1'
$checkout = (Get-Location).Path
python target/issue-5561-ref-prepare.py old
cargo run -p gwt-git --example issue_5561_ref_allocation_probe -- $checkout target/issue-5561-ref-probe-input.json
python target/issue-5561-ref-prepare.py new
cargo run -p gwt-git --example issue_5561_ref_allocation_probe -- $checkout target/issue-5561-ref-probe-input.json
Remove-Item -LiteralPath 'crates/gwt-git/examples/issue_5561_ref_allocation_probe.rs'
```

Both measured runs use the same frozen-input reader and callback. The initial
live-ref discovery run is not part of the controlled comparison. The saved
metadata records input, product, instrumented-copy, example, Cargo.lock and binary
SHA-256 values. The shared example is `target/issue-5561-ref-frozen-example.rs`.
These hashes identify the original captured bytes; Git normalizes repository
text artifacts to LF. Replay with the input and source snapshots inside the ZIP,
which preserves their original bytes.

## Interpretation

Both variants return 873,959 descriptors and select 8,514 unique payload OIDs.
Peak live allocation is 594,029,390 bytes before and 290,731,928 bytes after.
Returned live allocation is almost identical (242,069,839 / 242,063,239 bytes).
This demonstrates removal of overlapping generations, not a whole-GUI or real
model-agent fleet acceptance result. Wall times (10.018 / 9.571 seconds) are
observations under an uncontrolled host load, not a reliable speed ratio.

The product change transfers descriptor rows on the last occurrence of each
input commit, cloning only required duplicate input aliases. It then consumes
matrix rows and moves path/OID strings into output blobs. Public types, the full
selector matrix, input order and duplicate rows, unsafe/missing validation, and
global one-process/one-read-per-OID behavior remain unchanged. No raw tree cache
drop, ref chunking, streaming, or public API change was added.
