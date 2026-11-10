param([string]$BaselineRef = '53ce8f07808c661ea604ad87b664e70efca669e9')
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../../../..')).Path
$output = Join-Path $repo 'target/issue-5561-pending-probe'
[void](New-Item -ItemType Directory -Path $output -Force)
$relative = 'crates/gwt/src/work_events_ingest.rs'
$before = (& git -C $repo show "${BaselineRef}:$relative" | Out-String).Replace("`r`n", "`n")
if ($LASTEXITCODE -ne 0) { throw 'Baseline source unavailable.' }
$after = [IO.File]::ReadAllText((Join-Path $repo $relative)).Replace("`r`n", "`n")
function Extract-Types([string]$Source) {
    $start = $Source.IndexOf('#[derive(Debug)]' + "`n" + 'struct PendingWorkEventsSource')
    $end = $Source.IndexOf('type SourceFingerprints', $start)
    if ($start -lt 0 -or $end -le $start) { throw 'Product type boundaries changed.' }
    return $Source.Substring($start, $end - $start)
}
$deps = Join-Path $repo 'target/debug/deps'
$dependencies = @{}
foreach ($name in @('gwt_core', 'hex', 'sha2')) {
    $dependencies[$name] = (Get-ChildItem -LiteralPath $deps -Filter "lib$name-*.rlib" | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
    if (!$dependencies[$name]) { throw "Build checkout dependencies first: $name" }
}
$nativeArgs = @()
$cargoRoot = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
Get-ChildItem (Join-Path $cargoRoot 'registry/src') -Directory |
    ForEach-Object { Get-ChildItem -LiteralPath $_.FullName -Directory -Filter 'windows_x86_64_msvc-*' } |
    ForEach-Object { $nativeArgs += @('-L', ('native=' + (Join-Path $_.FullName 'lib'))) }
Get-ChildItem (Join-Path $repo 'target/debug/build') -Directory |
    ForEach-Object { Join-Path $_.FullName 'out' } |
    Where-Object { Test-Path -LiteralPath $_ } |
    ForEach-Object { $nativeArgs += @('-L', "native=$_") }
$provenance = @{
    baseline_ref=$BaselineRef; product=$relative; head=(& git -C $repo rev-parse HEAD).Trim();
    compiler=(& rustc -vV | Out-String); probe_sha256=(Get-FileHash "$PSScriptRoot/probe.rs").Hash.ToLowerInvariant();
    dependencies=@($dependencies.Values | ForEach-Object { @{filename=[IO.Path]::GetFileName($_);sha256=(Get-FileHash $_).Hash.ToLowerInvariant()} });
    limits='Same generated metadata only; shared empty content; no discovery/fold/store/GUI; requested allocations exclude allocator/OS overhead.';
    runs=@()
}
foreach ($mode in @('before','after')) {
    $source = if ($mode -eq 'before') { $before } else { $after }
    $types = Extract-Types $source
    $typesPath = Join-Path $output "$mode-types.rs"
    [IO.File]::WriteAllText($typesPath, $types)
    $env:ISSUE_5561_PRODUCT_TYPES = $typesPath
    $executable = Join-Path $output "$mode.exe"
    $arguments = @('--edition=2021', '--crate-name', 'pending_probe', '-O', '-A', 'dead_code', '-A', 'unused_imports', "$PSScriptRoot/probe.rs", '-L', "dependency=$deps", '-o', $executable)
    if ($mode -eq 'before') { $arguments += @('--cfg', 'before') }
    foreach ($name in $dependencies.Keys) { $arguments += @('--extern', "$name=$($dependencies[$name])") }
    & rustc @arguments @nativeArgs
    if ($LASTEXITCODE -ne 0) { throw "$mode compile failed." }
    $result = & $executable
    if ($LASTEXITCODE -ne 0) { throw "$mode probe failed." }
    [IO.File]::WriteAllText((Join-Path $output "$mode.json"), $result)
    $provenance.runs += @{
        mode=$mode; types_sha256=(Get-FileHash $typesPath).Hash.ToLowerInvariant();
        source_sha256=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($source))).ToLowerInvariant();
        executable_sha256=(Get-FileHash $executable).Hash.ToLowerInvariant(); result=($result | ConvertFrom-Json)
    }
}
$old = $provenance.runs[0].result
$new = $provenance.runs[1].result
if ($old.identity_sha256 -ne $new.identity_sha256 -or $new.retained_bytes -ge $old.retained_bytes) {
    throw 'Identity equality or allocation reduction failed.'
}
[IO.File]::WriteAllText((Join-Path $output 'comparison.json'), ($provenance | ConvertTo-Json -Depth 8))
Write-Output ($provenance | ConvertTo-Json -Depth 8)
