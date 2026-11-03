$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

# Build first so Cargo's build-directory lock cannot serialize the test runs.
cargo nextest run -p gwt --bin gwt --test hook_contracts --profile gwt-verify --retries 0 --no-run
if ($LASTEXITCODE) { throw 'Acceptance target build failed.' }

$artifacts = Join-Path (Get-Location) ('target/verification/issue-5829-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $artifacts -Force | Out-Null
$stdout = Join-Path $artifacts 'concurrent-health.stdout.txt'
$stderr = Join-Path $artifacts 'concurrent-health.stderr.txt'
$load = $null
try {
    # Existing compatibility tests supply real concurrent verification work.
    $load = Start-Process cargo -ArgumentList @(
        'nextest', 'run', '-p', 'gwt', '--test', 'hook_contracts',
        '--profile', 'gwt-verify', '--retries', '0',
        '-E', 'test(hook_health_test)', '--success-output', 'immediate'
    ) -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    do {
        if ($load.HasExited) { throw 'Concurrent verification exited before acceptance started.' }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Concurrent verification did not start.' }
        Start-Sleep -Milliseconds 100
        $started = (Get-Content -LiteralPath $stderr -Raw) -match 'Starting \d+ tests?'
    } until ($started)
    Write-Output "Concurrent hook-health verification PID: $($load.Id); logs: $artifacts"

    for ($iteration = 1; $iteration -le 3; $iteration++) {
        if ($load.HasExited) { throw 'Concurrent verification ended before all three iterations.' }
        Write-Output "Issue #5829 acceptance iteration $iteration/3 (concurrent PID $($load.Id) running)"
        cargo nextest run -p gwt --bin gwt --profile gwt-verify --retries 0 `
            -E 'test(managed_hook_health_for_one_hundred_forty_rows_reuses_session_json_within_one_second)' `
            --success-output immediate
        if ($LASTEXITCODE) { throw "Acceptance iteration $iteration failed." }
        if ($load.HasExited) { throw "Concurrent verification ended during iteration $iteration." }
    }
    $load.WaitForExit()
    Get-Content -LiteralPath $stdout
    Get-Content -LiteralPath $stderr
    if ($load.ExitCode) { throw "Concurrent compatibility tests failed: $($load.ExitCode)" }
    Write-Output 'Issue #5829 acceptance: three consecutive passes under concurrent verification load; compatibility tests passed.'
} finally {
    if ($load -and !$load.HasExited) { $load.Kill($true); $load.WaitForExit() }
}
