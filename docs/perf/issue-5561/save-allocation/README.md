# Persisted source-group lifetime

Issue #5561 remains open. Existing source in PRs #5742 and #5802 was already
contained in fetched `origin/develop` at `570a1a78fd20126d4f3bfdee4895a9e7001de5ec`.
This independent slice releases each loaded source map after its group file is
successfully written, before saving later groups and the index.

## Same-input allocation measurement

The standalone [probe](probe.rs) links the Cargo-built product library and calls
`WorkEventsIntakeStore::save`. It creates 24 ref groups with 8,000 sources each,
then verifies every persisted key/value through a sorted digest after saving.
The counting allocator records successful System allocation requests, including
the store and filesystem wrapper allocations; these are not GUI RSS/private bytes.

| Measurement | Before | After |
| --- | ---: | ---: |
| Source count | 192,000 | 192,000 |
| Live bytes before save | 66,702,157 | 66,702,316 |
| Live bytes after save, store still alive | 66,700,125 | 6,348 |
| Peak live bytes | 68,800,387 | 68,800,772 |
| Total requested bytes | 207,012,068 | 207,001,053 |
| Persisted bytes | 37,443,726 | 37,443,726 |
| Groups written | 24 | 24 |
| Sorted key/value digest | `f8d397899072b6b0` | `f8d397899072b6b0` |

Results: [before](before.json), [after](after.json),
[before provenance](before-metadata.json), [after provenance](after-metadata.json).
The save-boundary retained allocation fell by 66,693,777 bytes. Peak allocation
did not decrease: all maps still exist at save entry and the first group still
needs a serialization buffer. This is a lifetime reduction, not a GUI memory
budget result or a reduction in total allocation traffic.

Different disposable path lengths explain the small setup allocation difference.
The single runs took 2,162ms and 5,195ms with uncontrolled shared-host/cache load;
do not infer a throughput improvement or regression from that comparison. The
measurement excludes discovery, event collection/fold, real provider agents,
and four-hour operation. AC-2 and the same-condition AC-3 remain unproven.

## Reproduction and regression

On Windows with the checkout's Rust/MSVC environment and the standard
`target/debug` directory (`CARGO_TARGET_DIR` unset), run from the repository:

```powershell
pwsh -NoProfile -File docs/perf/issue-5561/save-allocation/measure.ps1 -Label after
```

For a baseline, put the probe and measurement script in the unmodified base and
use `-Label before`. The script builds the actual library, records source/library/
binary SHA256 values, and uses a unique disposable directory under `target`.
It never overwrites an existing fingerprint store. The published baseline was
collected before production edits with the same allocator and inputs; its rlib
hash was not recorded. Formatting of the published probe changed afterwards.
Initial native-link-search and digest-order mistakes are recorded in the baseline
metadata; the successful results include the corrected exact-value assertion.

The existing store regression was first run unchanged (4 PASS), then its
post-save assertion was changed to require release of persisted maps:
[RED](red.log) failed for that assertion. After the minimal product change,
[GREEN](green.log) reports 57 PASS, 2 diagnostic tests ignored, 0 FAIL. The same
test still verifies unsaved migration data, reloading exact saved values, and
retaining failed-read negative cache. Index-last publication, disk format,
public API and event semantics are unchanged. Rollback is a revert of this slice.

User Verification Result: n/a (autonomous).
