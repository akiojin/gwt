param([Parameter(Mandatory)][ValidateSet('before', 'after')][string]$Label)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../../../..')).Path
Push-Location $checkout
try {
    $artifacts = @(& cargo build -p gwt-core --lib --message-format=json)
    if ($LASTEXITCODE -ne 0) { throw 'Core library build failed.' }
    $core = @($artifacts | ForEach-Object { $_ | ConvertFrom-Json } |
        Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'gwt_core' } |
        ForEach-Object { $_.filenames } | Where-Object { $_.EndsWith('.rlib') })[-1]
    $nativeArgs = @()
    $cargoRoot = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    Get-ChildItem (Join-Path $cargoRoot 'registry/src') -Directory |
        ForEach-Object { Get-ChildItem -LiteralPath $_.FullName -Directory -Filter 'windows_x86_64_msvc-*' } |
        ForEach-Object { $nativeArgs += @('-L', ('native=' + (Join-Path $_.FullName 'lib'))) }
    Get-ChildItem target/debug/build -Directory |
        ForEach-Object { Join-Path $_.FullName 'out' } |
        Where-Object { Test-Path -LiteralPath $_ } |
        ForEach-Object { $nativeArgs += @('-L', "native=$_") }
    $run = Join-Path $checkout ('target/issue-5561-save-' + $Label + '-' + [guid]::NewGuid().ToString('N'))
    [void](New-Item -ItemType Directory -Path $run)
    $binary = Join-Path $run 'probe.exe'
    & rustc --edition=2021 (Join-Path $PSScriptRoot 'probe.rs') --extern "gwt_core=$core" -L dependency=target/debug/deps @nativeArgs -o $binary
    if ($LASTEXITCODE -ne 0) { throw 'Allocation probe build failed.' }
    $result = & $binary (Join-Path $run 'intake.json')
    if ($LASTEXITCODE -ne 0) { throw 'Allocation probe failed.' }
    [IO.File]::WriteAllText((Join-Path $run 'result.json'), $result)
    $hash = { param($path) (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
    $metadata = @{
        label = $Label; head = (& git rev-parse HEAD).Trim(); observed_at = [DateTime]::UtcNow.ToString('o')
        rustc = (& rustc --version).Trim(); product_sha256 = & $hash 'crates/gwt-core/src/work_events_intake.rs'
        probe_sha256 = & $hash (Join-Path $PSScriptRoot 'probe.rs'); rlib_sha256 = & $hash $core
        binary_sha256 = & $hash $binary
    }
    $metadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $run 'metadata.json')
    Write-Output $result
    Write-Output "Evidence: $run"
} finally { Pop-Location }
