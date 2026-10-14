# Live PM Board acceptance probe. Builds this checkout's normal library, then
# runs 100 prompts with a disposable Session cursor. Existing PM state is kept.
$ErrorActionPreference = 'Stop'
$probeRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$priorProfilePath = $env:GWT_HOOK_PROFILE_PATH
Push-Location $probeRoot
try {
    $artifacts = & cargo build -p gwt --lib --bin gwtd --message-format=json
    if ($LASTEXITCODE -ne 0) { throw 'checkout build failed' }
    $libraries = @{}
    foreach ($line in $artifacts) {
        $artifact = $line | ConvertFrom-Json
        if ($artifact.reason -ne 'compiler-artifact') { continue }
        foreach ($file in $artifact.filenames) {
            if ($file.EndsWith('.rlib')) { $libraries[$artifact.target.name] = $file }
        }
    }
    $probeDir = Join-Path ([IO.Path]::GetTempPath()) ('gwt-board-probe-5392-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $probeDir | Out-Null
    $probeExe = Join-Path $probeDir 'measure-board-prompt-5392.exe'
    $rustArgs = @('--edition=2021', '-C', 'linker=rust-lld',
        (Join-Path $PSScriptRoot 'measure-board-prompt-5392.rs'),
        '-L', ('dependency=' + (Join-Path $probeRoot 'target/debug/deps')), '-o', $probeExe)
    foreach ($name in @('gwt', 'gwt_agent', 'gwt_core', 'gwt_git', 'chrono')) {
        if (-not $libraries.ContainsKey($name)) { throw "missing library artifact: $name" }
        $rustArgs += @('--extern', ($name + '=' + $libraries[$name]))
    }
    $registry = Join-Path $env:USERPROFILE '.cargo/registry/src'
    foreach ($source in Get-ChildItem -LiteralPath $registry -Directory) {
        foreach ($version in @('0.53.1', '0.52.6')) {
            $native = Join-Path $source.FullName "windows_x86_64_msvc-$version/lib"
            if (Test-Path -LiteralPath $native) { $rustArgs += @('-L', ('native=' + $native)) }
        }
    }
    & rustc @rustArgs
    if ($LASTEXITCODE -ne 0) { throw 'probe compilation failed' }
    $paneResponse = '{"schema_version":1,"operation":"pane.list","params":{}}' |
        & (Join-Path $probeRoot 'target/debug/gwtd.exe') | ConvertFrom-Json
    if (-not $paneResponse.ok) { throw 'live agent census unavailable' }
    $panes = @($paneResponse.output -split '\r?\n' | Where-Object { $_ -match '\t(running|idle)\t' })
    if ($panes.Count -lt 8) { throw "eight live agent panes required, observed $($panes.Count)" }
    Write-Output "Live agent panes (including resident PM): $($panes.Count)"
    $env:GWT_HOOK_PROFILE_PATH = Join-Path $probeDir 'hook-profile.jsonl'
    & $probeExe $probeRoot
    $probeExitCode = $LASTEXITCODE
    $reads = @(Get-Content -LiteralPath $env:GWT_HOOK_PROFILE_PATH | ForEach-Object {
        $_ | ConvertFrom-Json
    } | Where-Object { $_.handler -eq 'board-reminder/board-read' })
    $times = @($reads | Sort-Object duration_ms)
    $failed = @($reads | Where-Object { $_.status -notin @('ok', 'partial') })
    if ($times.Count -gt 0) {
        Write-Output ('Board reads={0} failures={1} p50_ms={2:F3} p95_ms={3:F3} max_ms={4:F3}' -f
            $times.Count, $failed.Count, $times[[Math]::Floor(($times.Count - 1) * 0.5)].duration_ms,
            $times[[Math]::Floor(($times.Count - 1) * 0.95)].duration_ms, $times[-1].duration_ms)
    }
    Write-Output "Content-free hook profile: $($env:GWT_HOOK_PROFILE_PATH)"
    if ($probeExitCode -ne 0) {
        $failed | ForEach-Object { Write-Output ("Board failure reason: " + $_.status) }
        throw 'live Board acceptance probe failed'
    }
} finally {
    $env:GWT_HOOK_PROFILE_PATH = $priorProfilePath
    Pop-Location
}
