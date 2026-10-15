# Compile only the diagnostic program against an already-built checkout library.
param(
    [switch]$Instrument,
    [switch]$Worker,
    [string]$OutputName = 'comparison.jsonl',
    [string]$GwtLibraryName = 'libgwt-2d0217b5aa2a4e4e.rlib'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../../..')).Path
$deps = Join-Path $repo 'target/debug/deps'
$scratch = Join-Path $repo 'target/issue-5489-probe'
$baseline = Join-Path $repo 'target/issue-5489-baseline/session_inventory.rs'
$current = Join-Path $repo 'crates/gwt/src/session_inventory.rs'
$output = Join-Path $PSScriptRoot $OutputName
if (Test-Path -LiteralPath $output) { throw 'Refusing to overwrite a comparison.' }
if (-not (Test-Path -LiteralPath $baseline)) { throw 'Saved baseline module is required.' }
New-Item -ItemType Directory -Path $scratch -Force | Out-Null
foreach ($entry in @(@($baseline, 'before.rs'), @($current, 'after.rs'))) {
    $source = Get-Content -LiteralPath $entry[0] -Raw
    if ($Instrument) {
        # These wrappers preserve the called operations; only temporary copies change.
        $source = $source.Replace('SessionRuntimeState::load(', 'crate::measure_runtime_load(')
        $source = $source.Replace('Session::load(', 'crate::measure_session_load(')
        $source = $source.Replace('system.refresh_processes_specifics(', 'crate::measure_process_refresh(&mut system, ')
    }
    [IO.File]::WriteAllText((Join-Path $scratch $entry[1]), $source, [Text.UTF8Encoding]::new($false))
}
if ($Worker) {
    if ($Instrument) { throw 'Worker comparison must be uninstrumented.' }
    foreach ($entry in @(@((Join-Path $repo 'target/issue-5489-baseline/monitor_duplicate_runtime.rs'), 'monitor-before.rs', 'before_inventory'), @((Join-Path $repo 'crates/gwt/src/monitor_duplicate_runtime.rs'), 'monitor-after.rs', 'after_inventory'))) {
        $source = Get-Content -LiteralPath $entry[0] -Raw
        $source = $source.Replace('crate::session_inventory::', "crate::$($entry[2])::")
        $source = $source.Replace('monitor_runtime_uncertainty_affects_issue,', '')
        $source = $source.Replace('monitor_runtime_uncertainty_affects_issue(', 'crate::census_uncertainty(')
        [IO.File]::WriteAllText((Join-Path $scratch $entry[1]), $source, [Text.UTF8Encoding]::new($false))
    }
}
$externs = @()
$gwtLibrary = Get-Item -LiteralPath (Join-Path $deps $GwtLibraryName)
if (-not $gwtLibrary) { throw 'Missing built gwt library.' }
$gwtSuffix = $gwtLibrary.BaseName.Substring('libgwt-'.Length)
$fingerprints = Join-Path $repo 'target/debug/.fingerprint'
$gwtFingerprint = Get-Content -LiteralPath (Join-Path $fingerprints "gwt-$gwtSuffix/lib-gwt.json") -Raw | ConvertFrom-Json
$requiredCrates = @('gwt', 'gwt_agent', 'gwt_core', 'serde', 'serde_json', 'sysinfo', 'dunce', 'chrono')
if ($Worker) { $requiredCrates += @('gwt_terminal', 'tracing', 'sha2') }
foreach ($crate in $requiredCrates) {
    if ($crate -eq 'gwt') {
        $externs += @('--extern', "gwt=$($gwtLibrary.FullName)")
    } else {
        # Build/test feature sets can coexist. Match the selected gwt's exact
        # dependency fingerprint instead of mixing newest-mtime rlibs.
        $dependency = @($gwtFingerprint.deps | Where-Object { $_[1] -eq $crate })
        if ($dependency.Count -ne 1) { throw "Missing gwt dependency: $crate" }
        $wanted = [Convert]::ToHexString([BitConverter]::GetBytes([uint64]$dependency[0][3])).ToLowerInvariant()
        $matches = @()
        foreach ($candidate in Get-ChildItem -LiteralPath $deps -Filter "lib$crate-*.rlib" -File) {
            $suffix = $candidate.BaseName.Substring("lib$crate-".Length)
            foreach ($package in @($crate, $crate.Replace('_', '-')) | Select-Object -Unique) {
                $fingerprint = Join-Path $fingerprints "$package-$suffix/lib-$crate"
                if ((Test-Path -LiteralPath $fingerprint) -and ((Get-Content -LiteralPath $fingerprint -Raw).Trim() -eq $wanted)) {
                    $matches += $candidate.FullName
                }
            }
        }
        $matches = @($matches | Select-Object -Unique)
        if ($matches.Count -ne 1) { throw "Missing or ambiguous matching library: $crate" }
        $externs += @('--extern', "$crate=$($matches[0])")
    }
}
$probeSource = if ($Worker) { 'worker-probe.rs' } else { 'probe.rs' }
$exe = Join-Path $scratch ([IO.Path]::ChangeExtension($probeSource, 'exe'))
$nativePaths = @()
foreach ($directory in Get-ChildItem -Path "$env:USERPROFILE/.cargo/registry/src/*/windows_x86_64_msvc-*/lib" -Directory) {
    $nativePaths += @('-L', "native=$($directory.FullName)")
}
& rustc --edition=2021 (Join-Path $PSScriptRoot $probeSource) --crate-name issue_5489_probe `
    '-C' 'debuginfo=0' '-C' 'opt-level=0' '-C' 'linker=rust-lld' '-L' "dependency=$deps" @nativePaths @externs -o $exe
if ($LASTEXITCODE -ne 0) { throw 'Probe compilation failed.' }
$roots = @((Get-Content -LiteralPath "$env:USERPROFILE/.gwt/session.json" -Raw | ConvertFrom-Json).tabs.project_root)
if ($roots.Count -ne 5) { throw 'Expected the five actual open roots.' }
$sessions = Join-Path $env:USERPROFILE '.gwt/sessions'
$context = [ordered]@{
    observed_at = [DateTime]::UtcNow.ToString('o')
    instrumented = [bool]$Instrument
    profile = 'diagnostic opt-level=0, existing target/debug dependencies'
    baseline_sha256 = (Get-FileHash -LiteralPath $baseline -Algorithm SHA256).Hash.ToLowerInvariant()
    current_sha256 = (Get-FileHash -LiteralPath $current -Algorithm SHA256).Hash.ToLowerInvariant()
    sessions_dir = $sessions
    roots = $roots
    instrumentation_note = 'Instrumented CPU includes GetProcessTimes overhead per load; use an uninstrumented comparison for the improvement ratio.'
    gwt_library = $gwtLibrary.FullName
}
if ($Worker) {
    $context['monitor_baseline_sha256'] = (Get-FileHash -LiteralPath (Join-Path $repo 'target/issue-5489-baseline/monitor_duplicate_runtime.rs') -Algorithm SHA256).Hash.ToLowerInvariant()
    $context['monitor_current_sha256'] = (Get-FileHash -LiteralPath (Join-Path $repo 'crates/gwt/src/monitor_duplicate_runtime.rs') -Algorithm SHA256).Hash.ToLowerInvariant()
}
[IO.File]::WriteAllText("$output.context.json", ($context | ConvertTo-Json -Depth 4) + "`n")
if ($Worker) {
    $fixture = Join-Path $scratch ("prefs-" + [Guid]::NewGuid().ToString('N'))
    & $exe prepare $fixture @roots
    if ($LASTEXITCODE -ne 0) { throw 'Prefs fixture preparation failed.' }
    [IO.File]::Copy((Join-Path $fixture 'manifest.json'), (Join-Path $PSScriptRoot 'worker-prefs-manifest.json'), $false)
    foreach ($mode in @('before', 'after')) {
        & $exe $mode $fixture $sessions @roots | Tee-Object -FilePath $output -Append
        if ($LASTEXITCODE -ne 0) { throw "Worker probe failed: $mode" }
    }
} else {
    & $exe $sessions @roots | Tee-Object -FilePath $output
    if ($LASTEXITCODE -ne 0) { throw 'Probe failed.' }
}
