# Collector allocation comparison reproduction

Run from the checkout root on the measured Windows/MSVC host:

```powershell
pwsh -NoProfile -File target/issue-5561-current/collector-reproduce.ps1
```

The script links the preserved probe against existing dependencies without a Cargo rebuild:

```powershell
rustc --edition=2021 target/issue-5561-current/collector-probe.rs `
  -L dependency=target/debug/deps `
  --extern gwt_core=target/debug/deps/libgwt_core-0df1c2168f9284e5.rlib `
  --extern serde_json=target/debug/deps/libserde_json-3085a25fd445b373.rlib `
  --extern tracing=target/debug/deps/libtracing-e49ed03754391b73.rlib `
  <native search arguments> `
  -o target/issue-5561-current/collector-probe.exe
```

`<native search arguments>` is one `-L native=<absolute directory>` pair for every Windows MSVC registry `lib` directory and every `target/debug/build/*/out` containing a `.lib`. The exact expanded argument array is saved to `collector-build-arguments.json`. In particular `windows_x86_64_msvc-0.53.0/lib` supplies `windows.0.53.0.lib`. No optimization flag is added. Dependency fingerprints matter: selecting the newest serde_json rlib produced incompatible serde_core traits. Missing dependency files cause an explicit failure rather than silently changing the measured build. Dependency hashes are recorded per run; review decoder/WorkEvent changes if rebuilt dependencies differ from `collector-allocation.json`.

The exact six fixture invocations are:

```powershell
./target/issue-5561-current/collector-probe.exe old 64 4 701 1024
./target/issue-5561-current/collector-probe.exe grouped 64 4 701 1024
./target/issue-5561-current/collector-probe.exe accumulator 64 4 701 1024
./target/issue-5561-current/collector-probe.exe old 2000 4 8 2048
./target/issue-5561-current/collector-probe.exe grouped 2000 4 8 2048
./target/issue-5561-current/collector-probe.exe accumulator 2000 4 8 2048
```

Positional dimensions are unique content count, events per content, aliases per content, and title bytes. Aliases share their content's Arc. Every unique event ID is `evt-{group}-{line}`, Work ID is `work-{group}`, kind is `start`, timestamp is fixed, title is repeated ASCII `x`, and explicit event container is `work/probe`. Source diagnostic containers are `None`. Input generation occurs before allocator reset. No provider process or user data is read.

For each input, the driver asserts identical dimensions, sorted full serialized-key FNV1a digest, full serialized-key byte count, expected incoming count, expected skipped duplicate count, and unique-line decode count across all modes. This is fixture equivalence evidence, not a collision-proof semantic theorem. Expected counts are 512 incoming/178944 skipped for the alias-heavy input and 16000 incoming/48000 skipped for the unique-content-heavy input. `collector-allocation.json` preserves the initial six observations. `collector-reproduced.json` saves the reproduction run and assertions, with source/binary/dependency hashes.

## Product and instrument differences

`old` is an exact copy of baseline `collect_shared_work_event_sources_with_decoder` after undoing only these changes: rename to `old`, increment CLONES immediately before the existing clone, increment SERIALIZES immediately before the existing serde call, and `capture()` immediately before return. `collector-old-function.rs` stores the restored body. Whitespace-normalized comparison against both HEAD and origin/develop succeeded before implementation; those facts and baseline HEAD are in `collector-allocation.json`.

Harness substitutions: local SharedWorkEventsSource has the same two Arc fields; CachedSharedWorkEventLine has the same variants; local report contains the four collection counters only. WorkEvent, decoder, error type and serde behavior come from the linked production crate. The probe's `is_close_kind` omits private `!event.is_successor_supersession()`. All inputs are ordinary `start` events, making this difference unreachable. Close/supersession behavior is not measured.

`old` retains existing tracing calls but installs no tracing subscriber. Both candidates omit missing-container and malformed-line warnings and use a shorter serde-error string. Inputs have explicit event containers and no opaque, invalid or unserializable lines, making those differences unreachable. Candidate logging and opaque/invalid semantics require production regressions; this probe does not validate them.

`grouped` models first-appearance source multiplicity grouping with a Vec and identity-to-index map. It owns Arcs against pointer reuse, drops the identity map after grouping, decodes each unique content once, caps identical full stable keys at two copies, moves the canonical event, and clones an accepted second copy. CLONES/SERIALIZES/capture use the same measurement points as `old`. The counter map still owns an extra stable key. The second returned key is cloned from the first serialization, trimming String capacity compared with serializing another alias; lower returned allocation includes that effect. The measured copy drops the original source Vec iterator after grouping. A product loop over a previously collected Vec may keep its source-vector allocation until return; remeasure exact bytes against the final product body.

`accumulator` is a separate alternative, not the planned minimum implementation. It stores owned full-key-to-(event,copies) entries and builds incoming at the end, removing the counter's extra key allocation. Enumeration is in HashMap order; downstream full-key sorting and production regressions remain necessary.

The allocator forwards unchanged to System and counts successful requested alloc/dealloc/realloc byte sizes. Snapshots precede output digest/JSON. Values are Rust live/peak/cumulative requests, not OS RSS/private bytes. Input allocations are included in live/peak; cumulative requests reset after input creation. GUI, rescan, fold, publication, real Agent fleet and four-hour behavior are excluded. Wall times are descriptive under uncontrolled host load.

## Exact product-candidate remeasurement

The original probe and six-mode observations above are preserved. A separate extraction, `collector-product-probe.rs`, replaces only `grouped` with the exact current production `collect_shared_work_event_sources_with_decoder` body. Undoing the function rename, CLONES/SERIALIZES increments and final capture produces a byte-for-byte UTF-8/LF match with the production function. Both exact and whitespace-normalized comparison results and function SHA-256 values are stored in `collector-product-source.json` and embedded in `collector-product.json`. The original instrumented `old` body is byte-for-byte unchanged.

This product extraction preserves the actual source `for` iterator lifetime, warning calls and serde error text. The modeled-grouped iterator lifetime limitation in the earlier comparison does not apply to this extraction. The common harness still has the limited close helper and no tracing subscriber; all measured records are known `start` events with explicit containers, so those differences are unreachable in the fixtures. The extracted function does not establish GUI or close/opaque/invalid correctness.

Rebuild and rerun both inputs for `old` and the exact candidate:

```powershell
pwsh -NoProfile -File target/issue-5561-current/collector-product-reproduce.ps1
```

The script records exact expanded rustc arguments in `collector-product-build-arguments.json`; dependencies and native search directories follow the original script. Source/output paths change to `collector-product-probe.rs` and `collector-product.exe`. Exact invocations are:

```powershell
./target/issue-5561-current/collector-product.exe old 64 4 701 1024
./target/issue-5561-current/collector-product.exe grouped 64 4 701 1024
./target/issue-5561-current/collector-product.exe old 2000 4 8 2048
./target/issue-5561-current/collector-product.exe grouped 2000 4 8 2048
```

The same dimension, full-key digest, full-key byte count and expected incoming/duplicate/decode assertions apply. Results go to `collector-product.json`, preserving `collector-allocation.json` and `collector-reproduced.json`.
