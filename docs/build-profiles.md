# Development build profiles

Issue [#4824](https://github.com/akiojin/gwt/issues/4824) reduces development
and test build cost without changing optimization levels or release profiles.
The test profile inherits the development profile.

## Settings and tradeoffs

- Workspace crates use `debug = "line-tables-only"`: backtraces retain source
  file and line locations, but full debugger variable/type information is omitted.
- Dependencies use `debug = false`. Dependency source locations and full debug
  information are deliberately omitted; workspace backtrace locations remain.
- `x86_64-pc-windows-msvc` uses the Rust toolchain's `rust-lld` linker. This
  target setting applies to release builds too. Other targets keep their default
  linker. The existing `incremental = false` setting is unchanged.

For a debugging session needing full symbols, temporarily override the relevant
Cargo profile settings instead of making full dependency symbols the default.

The existing panic-catching test
`cli::hook::envelope::tests::stop_additional_context_panics_in_debug_and_falls_back_in_release`
passes with `RUST_BACKTRACE=full`. Its backtrace retains workspace frames at
`crates/gwt/src/cli/hook/envelope.rs:178`, `:246`, and `:360` under the adopted
profile and linker, confirming file/line information beyond the panic header.

## Local profile comparison

Measured on Windows on 2026-10-02, source `31d91a720`, Rust 1.95.0,
LLVM 22.1.2, and `CARGO_BUILD_JOBS=8`. Each cold run used a separate, empty
target directory with the registry and toolchain already cached. The command
was `cargo test -p gwt --no-run --locked` with that directory and the candidate
profile overrides. Warm runs immediately repeated the same command.
All four profile candidates used the default MSVC linker during this comparison.

| Candidate | Cold seconds (two runs) | Warm seconds (two runs) | Mean target bytes |
| --- | --- | --- | ---: |
| Original full debug | 291.199 / 347.237 | 1.173 / 1.112 | 22,614,207,216 |
| Workspace line tables only | 240.946 / 256.411 | 1.089 / 1.076 | 13,015,222,492 |
| Dependency debug disabled | 265.313 / 272.186 | 1.151 / 1.122 | 15,522,693,670 |
| Both profile changes | 210.419 / 208.102 | 1.111 / 1.073 | 5,537,599,791 |

The combined profile reduces mean cold time from 319.218 to 209.260 seconds
(34.45%) and target size by 75.51%. Workspace-only and dependency-only mean
cold reductions are 22.10% and 15.81%. The combined candidate exceeds the 10%
adoption threshold in both paired samples (27.74% and 40.07%); no extra
borderline samples were needed. The PM's 2026-10-01 ruling permits the two
profile settings to be evaluated as one candidate. No candidate was retained
below the threshold. Warm timings near one second do not establish a meaningful
speed improvement.

Size is the sum of file lengths under the target directory, counting hard-linked
paths separately, rather than allocated disk blocks. The formal host-exclusive
run recorded zero external compiler CPU time and zero external compiler duration
for all eight trials. Record: `vrr-e7290e38c9a94845a880f3af4754da80`.

## Windows linker comparison

After adopting the profiles, two alternating trials linked the `gwtd` binary
with MSVC 14.50.35717 and `rust-lld`. Each trial removed its own executable
outputs to force a real link. Dependency compilation was warmed beforehand.
Both linkers received `/TIME`; the numbers below are their native total elapsed
link time, excluding Rust compilation and Cargo overhead.

| Linker | First link (seconds) | Second link (seconds) | Mean (seconds) |
| --- | ---: | ---: | ---: |
| MSVC | 2.484 | 2.500 | 2.492 |
| rust-lld | 1.736 | 1.761 | 1.7485 |

The 29.84% mean link-time reduction exceeds the 10% adoption threshold, so
`rust-lld` is retained. Each sample requires an actual, positive native timing
and successful link; a cached Cargo invocation cannot qualify. External build
monitoring accepted all trials. Record: `vrr-c1bfce5838104597b6eff7bfbff0e604`.

With both profiles and `rust-lld` enabled, a final independent empty-target
run on the same source took 200.51 seconds cold and 1.31 seconds warm, with
5,597,305,665 target bytes. External compiler CPU/duration remained zero.
This final-set sample is 37.19% faster than the original two-run cold mean;
the separate profile and native linker comparisons above determine adoption.

## CI observation and reproduction

Linux Build and Windows Check compare profile/configuration changes using
`python scripts/measure-build-profile.py measure --base HEAD^`. On pull requests,
the checkout is GitHub's merge commit and its first parent is the base branch.
For another local comparison, pass an explicit baseline revision with `--base`.

The script applies baseline profiles and Cargo configuration to the same current
source and lockfile, uses independent empty target directories, and measures
cold/warm `cargo test -p gwt --no-run --locked` time and target bytes for both
settings. It restores the original file bytes, records toolchain/settings and
exit codes, and removes only its own temporary directories. Results appear in
the job summary and `build-profile-Linux` / `build-profile-Windows` artifacts.

These observations have a 15-minute limit and are non-blocking. They add no
test execution suite and do not replace the existing required checks on Linux,
Windows, and macOS. A timed-out, failed, incomplete, or unrestored comparison
is not acceptance evidence; inspect `valid_for_comparison` and all four runs.
The local acceptance suite includes full Windows tests for `gwt-core` and `gwt`
with all features, a workspace backtrace location check, and a release build
to exercise the linker outside the development profile.
