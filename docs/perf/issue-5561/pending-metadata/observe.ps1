# Diagnostic run only. The normal product receives a separate canonical run.
param([ValidateRange(60, 14400)][int]$Seconds = 3600)
$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../../../..')).Path
$capture = Join-Path $checkout ('target/issue-5561-observation-' + [guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $capture)
$settings = @{
    ISSUE_5561_CAPTURE_DIRECTORY=$capture
    GWT_PLAYWRIGHT_MEMORY_SECONDS=[string]$Seconds
    GWT_PLAYWRIGHT_MEMORY_PANES='4'
    GWT_PLAYWRIGHT_MEMORY_BUDGET_BYTES='1500000000'
    GWT_PLAYWRIGHT_MEMORY_ISSUE='5561'
    GWT_PLAYWRIGHT_MEMORY_RECEIPT=(Join-Path $capture 'fixture-home.txt')
}
$previous = @{}
$files = @('crates/gwt/src/main.rs', 'crates/gwt/src/work_events_ingest.rs',
           'docs/perf/issue-5561/pending-metadata/live-allocator.rs',
           'scripts/verify-runtime-memory-budget.ps1',
           'crates/gwt/playwright/tests/runtime-memory-budget-live.spec.ts')
$metadata = @{
    head=(& git -C $checkout rev-parse HEAD).Trim()
    started_at=[DateTime]::UtcNow.ToString('o')
    binary_sha256=(Get-FileHash "$checkout/target/debug/gwt.exe").Hash.ToLowerInvariant()
    source_sha256=@{}
    limitation='Four deterministic command PTYs and changing checkout history; not real-model fleet. Compare requested Rust bytes separately from native/JS/private commitment. Prior observation used a different diagnostic allocator and base.'
}
foreach ($relative in $files) {
    $metadata.source_sha256[$relative]=(Get-FileHash (Join-Path $checkout $relative)).Hash.ToLowerInvariant()
}
[IO.File]::WriteAllText((Join-Path $capture 'metadata.json'), ($metadata | ConvertTo-Json -Depth 5))
Write-Output "Capture: $capture"
try {
    foreach ($key in $settings.Keys) {
        $previous[$key]=[Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $settings[$key])
    }
    & (Join-Path $checkout 'scripts/verify-runtime-memory-budget.ps1') --headed
    $result=$LASTEXITCODE
} finally {
    foreach ($key in $previous.Keys) { [Environment]::SetEnvironmentVariable($key, $previous[$key]) }
    $receipt=Join-Path $capture 'fixture-home.txt'
    if (Test-Path -LiteralPath $receipt) {
        $checkHome=[IO.File]::ReadAllText($receipt).Trim()
        foreach ($name in @('runtime-memory-dark.json', 'runtime-memory-light.json',
                            'runtime-memory-dark.png', 'runtime-memory-light.png',
                            'shutdown.json', 'fixture.json')) {
            $source=Join-Path $checkHome $name
            if (Test-Path -LiteralPath $source) { Copy-Item -LiteralPath $source -Destination $capture }
        }
    }
    Write-Output "Preserved observation: $capture"
}
exit $result
