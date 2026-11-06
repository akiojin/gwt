#Requires -Version 7.0
# Run after a fresh checkout build; verify.run may forward its headed reporter arguments.
[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$SkipBuild,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwt = Join-Path $checkout 'target/debug/gwt.exe'
$gwtd = Join-Path $checkout 'target/debug/gwtd.exe'
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-project-scan-6809-' + [Guid]::NewGuid().ToString('N'))
$version = [IO.File]::ReadAllText((Join-Path $checkout 'scripts/playwright-version.txt')).Trim()
$deps = Join-Path $env:LOCALAPPDATA "Temp/gwt-playwright-win-$version"
$cli = Join-Path $deps 'node_modules/@playwright/test/cli.js'
$resolver = Join-Path $deps 'resolve-playwright.cjs'
$gui = $null
$anchor = $null
$savedRunner = @{}
$exitCode = 1

function Invoke-SourceEnvelope([string]$Operation, [hashtable]$Parameters) {
    $request = @{schema_version=1;operation=$Operation;params=$Parameters} | ConvertTo-Json -Depth 8 -Compress
    $response = $request | & $gwtd | ConvertFrom-Json
    if (!$response.ok) { throw "$Operation failed: $($response.output)" }
    return $response
}
function Assert-Preflight($Doctor) {
    $preflight = $Doctor.preflight
    if ($preflight.schema_version -ne 1 -or $preflight.result -ne 'proceed' -or
        $preflight.context -ne 'current' -or $preflight.normal_completion -ne 'normal' -or
        $preflight.convergence -ne 'converged' -or @($preflight.reasons).Count) {
        throw ('Managed-browser preflight failed: ' + ($preflight | ConvertTo-Json -Depth 8 -Compress))
    }
}
function New-IsolatedStart([string]$File, [string[]]$Arguments) {
    $start = [Diagnostics.ProcessStartInfo]::new($File)
    $start.WorkingDirectory = $checkout
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($key in @($start.Environment.Keys)) {
        if ($key -match '^(GWT_|CODEX_|CLAUDE_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)') { [void]$start.Environment.Remove($key) }
    }
    $start.Environment['HOME'] = $checkHome
    $start.Environment['USERPROFILE'] = $checkHome
    $start.Environment['CODEX_HOME'] = "$checkHome/.codex"
    $start.Environment['TEMP'] = "$checkHome/tmp"
    $start.Environment['TMP'] = "$checkHome/tmp"
    $start.Environment['RUST_LOG'] = 'info'
    $start.Environment['GIT_TERMINAL_PROMPT'] = '0'
    $start.Environment['GH_PROMPT_DISABLED'] = '1'
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    return $start
}
function Invoke-IsolatedScan {
    $start = New-IsolatedStart $gwtd @()
    $start.RedirectStandardInput = $true
    $start.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
    $process = [Diagnostics.Process]::Start($start)
    try {
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.StandardInput.WriteLine('{"schema_version":1,"operation":"workspace.project_scan","params":{}}')
        $process.StandardInput.Close()
        if (!$process.WaitForExit(120000)) { $process.Kill($true); throw 'Isolated project scan timed out.' }
        $response = $stdout.GetAwaiter().GetResult() | ConvertFrom-Json
        if ($process.ExitCode -or !$response.ok) { throw "Isolated project scan failed: $($stderr.GetAwaiter().GetResult())" }
        return ($response.output | ConvertFrom-Json)
    } finally { $process.Dispose() }
}
function Measure-LegacyEnumeration {
    # Reproduce the original full traversal using standard IO. Its elapsed
    # time is a PowerShell algorithm baseline, not timing from an old binary.
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $roots = [Collections.Generic.List[string]]::new()
    $session = [IO.File]::ReadAllText("$checkHome/.gwt/session.json") | ConvertFrom-Json
    foreach ($tab in $session.tabs) { $roots.Add($tab.project_root) }
    $roots.Add($checkout)
    $directoryReads = 1
    $entriesEnumerated = 0
    $endpointsRead = 0
    foreach ($project in [IO.Directory]::EnumerateFileSystemEntries("$checkHome/.gwt/projects")) {
        $entriesEnumerated++
        $directoryReads++
        try { $descriptors = [IO.Directory]::GetFileSystemEntries((Join-Path $project 'runtime/daemon')) }
        catch {
            if ($_.Exception -is [IO.DirectoryNotFoundException] -or $_.Exception.InnerException -is [IO.DirectoryNotFoundException]) { continue }
            throw
        }
        foreach ($descriptor in $descriptors) {
            $entriesEnumerated++
            if ([IO.Path]::GetExtension($descriptor) -ne '.json') { continue }
            $endpointsRead++
            $endpoint = [IO.File]::ReadAllText($descriptor) | ConvertFrom-Json
            if ($endpoint.scope.target -eq 'host') { $roots.Add($endpoint.scope.project_root) }
        }
    }
    $uniqueRoots = @($roots | Sort-Object -Unique)
    $clock.Stop()
    return @{algorithm='legacy full project/daemon directory enumeration (PowerShell reproduction; no identity resolution)';
        directory_reads=$directoryReads;entries_enumerated=$entriesEnumerated;endpoints_read=$endpointsRead;
        root_count=$uniqueRoots.Count;elapsed_ms=$clock.Elapsed.TotalMilliseconds}
}

Push-Location -LiteralPath $checkout
try {
    if (!$SkipBuild) {
        & cargo build -p gwt --bin gwt --bin gwtd --target-dir (Join-Path $checkout 'target')
        if ($LASTEXITCODE -ne 0) { throw 'Build this checkout before the browser check.' }
    }
    foreach ($file in @($gwt, $gwtd, $cli, $resolver)) {
        if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Required checkout binary or pinned Playwright dependency missing: $file" }
    }
    $doctor = (Invoke-SourceEnvelope 'hook.doctor' @{purpose='managed_browser'}).output | ConvertFrom-Json
    Assert-Preflight $doctor
    $hook = $doctor.managed_hook_bin
    if (!$hook -or $hook -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable managed-hook fallback missing.' }
    Assert-Preflight ((Invoke-SourceEnvelope 'hook.doctor' @{purpose='managed_browser';repair=$true;expected_hook_bin=$hook}).output | ConvertFrom-Json)
    $repoHash = (Invoke-SourceEnvelope 'issue.monitor.status' @{}).project_store.hash
    if ($repoHash -notmatch '^[0-9a-f]{16}$') { throw 'Canonical project_store.hash missing.' }
    $state = Join-Path $checkHome ".gwt/projects/$repoHash/project-state"
    [void](New-Item -ItemType Directory -Path $state, "$checkHome/.codex", "$checkHome/tmp" -Force)
    [IO.File]::WriteAllText("$checkHome/.codex/config.toml", '')
    [IO.File]::WriteAllText("$checkHome/.gwt/config.toml", "auto_apply_updates = false`n")
    [IO.File]::WriteAllText("$state/pm.json", '{"settings":{"auto_start":false}}')
    [IO.File]::WriteAllText("$state/issue-monitor.json", '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
    [IO.File]::WriteAllText("$checkHome/.gwt/session.json", (@{
        active_tab_id='project-scan-6809'; tabs=@(@{id='project-scan-6809';title='Project scan';project_root=$checkout;kind='git'});recent_projects=@()
    } | ConvertTo-Json -Depth 8))
    $runtime = Join-Path $env:USERPROFILE '.gwt/runtime'
    if (Test-Path -LiteralPath $runtime -PathType Container) {
        [void](New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target $runtime)
    }
    # Synthetic endpoints name nonexistent roots and no process or capability.
    # Never import production project stores, runtime endpoints, or Host tokens.
    for ($index = 0; $index -lt 13000; $index++) {
        $hash = '{0:x16}' -f ([int64]0x6809000000000000 + $index)
        if ($hash -eq $repoHash) { throw 'Synthetic project hash unexpectedly collides with the checkout.' }
        $directory = Join-Path $checkHome ".gwt/projects/$hash/runtime/daemon"
        [void][IO.Directory]::CreateDirectory($directory)
        $endpoint = @{protocol_version=4;daemon_version='fixture';startup_intent='standalone';pid=0;
            bind='127.0.0.1:0';auth_token='fixture-only';updated_at_unix_ms=0;
            scope=@{repo_hash=$hash;worktree_hash=$hash;project_root="$checkHome/missing/$hash";target='host'}}
        [IO.File]::WriteAllText("$directory/$hash.json", ($endpoint | ConvertTo-Json -Depth 8 -Compress))
    }
    $baseline = Measure-LegacyEnumeration
    [IO.File]::WriteAllText("$checkHome/scan-legacy-baseline.json", ($baseline | ConvertTo-Json -Depth 8))
    Write-Output ('Legacy full-enumeration algorithm baseline (PowerShell reproduction): ' + ($baseline | ConvertTo-Json -Compress))
    $cold = Invoke-IsolatedScan
    $warm = Invoke-IsolatedScan
    [IO.File]::WriteAllText("$checkHome/scan-cold.json", ($cold | ConvertTo-Json -Depth 8))
    [IO.File]::WriteAllText("$checkHome/scan-warm.json", ($warm | ConvertTo-Json -Depth 8))
    [IO.File]::WriteAllText("$checkHome/scan-comparison.json", (@{fixture_stores=13000;
        legacy_algorithm_baseline=$baseline;checkout_cold=$cold;checkout_warm=$warm;
        note='Baseline elapsed time uses PowerShell standard IO; checkout metrics are emitted by Rust. Both use the same fixture, and no identity resolution is performed by the baseline.'
    } | ConvertTo-Json -Depth 10))
    if ($cold.cache_hit -or !$warm.cache_hit -or $cold.entries_enumerated -lt 13000 -or
        $warm.directory_reads -ne 0 -or $warm.entries_enumerated -ne 0 -or
        $warm.eligible_roots -ne 0 -or $warm.endpoints_read -ne 13000) { throw 'Cold/warm catalog scan metrics did not satisfy the bounded-cache contract.' }
    Write-Output ('Cold project scan: ' + ($cold | ConvertTo-Json -Compress))
    Write-Output ('Warm project scan: ' + ($warm | ConvertTo-Json -Compress))

    $start = New-IsolatedStart $gwt @('--no-tray','--no-open')
    $start.Environment['GWT_HOOK_BIN'] = $hook
    $start.Environment['GWT_BROWSER_URL_FILE'] = "$checkHome/url.txt"
    $start.Environment['GWT_DISABLE_BACKGROUND_INDEX'] = '1'
    $launchedAt = [DateTime]::UtcNow.ToString('o')
    $gui = [Diagnostics.Process]::Start($start)
    $stdout = $gui.StandardOutput.ReadToEndAsync()
    $stderr = $gui.StandardError.ReadToEndAsync()
    Write-Output "Owned checkout GUI PID: $($gui.Id); isolated HOME: $checkHome"
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    $base = ''
    do {
        if ($gui.HasExited) { throw 'Owned GUI exited during startup.' }
        if (Test-Path -LiteralPath "$checkHome/url.txt" -PathType Leaf) {
            $candidate = [IO.File]::ReadAllText("$checkHome/url.txt").Trim()
            try { if ((Invoke-WebRequest -Method Head -Uri $candidate -TimeoutSec 2).StatusCode -eq 200) { $base=$candidate; break } } catch {}
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    if (!$base) { throw 'Fresh checkout readiness timed out.' }
    $anchor = [Net.WebSockets.ClientWebSocket]::new()
    $anchor.Options.SetRequestHeader('Origin', $base.TrimEnd('/'))
    $anchor.ConnectAsync([Uri](($base -replace '^http','ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
    Assert-Preflight ((Invoke-SourceEnvelope 'hook.doctor' @{purpose='managed_browser';expected_hook_bin=$hook}).output | ConvertFrom-Json)
    [IO.File]::WriteAllText("$checkHome/issue-6809-fixture.json", (@{checkout=$checkout;repo_hash=$repoHash;store_count=13000;
        owned_gui_pid=$gui.Id;launched_at=$launchedAt;url=$base} | ConvertTo-Json -Depth 8))
    $runner = @{GWT_PLAYWRIGHT_BASE_URL=$base;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;
        GWT_PLAYWRIGHT_ISSUE_6809='1';GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
        NODE_OPTIONS=('--require "' + $resolver.Replace('\','/') + '" ' + $env:NODE_OPTIONS)}
    foreach ($key in $runner.Keys) {
        $savedRunner[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $runner[$key])
    }
    & node $cli test project-store-scan-live.spec.ts --config crates/gwt/playwright/playwright.config.ts --workers=1 --retries=0 --headed --output "$checkHome/playwright" @PlaywrightArguments
    $exitCode = $LASTEXITCODE
} finally {
    foreach ($key in $savedRunner.Keys) { [Environment]::SetEnvironmentVariable($key, $savedRunner[$key]) }
    if ($anchor) { $anchor.Abort(); $anchor.Dispose() }
    $stopped = $false
    if ($gui) {
        if (!$gui.HasExited) { $gui.Kill($true) }
        $stopped = $gui.WaitForExit(10000)
        if ($stopped) {
            [IO.File]::WriteAllText("$checkHome/stdout.log", $stdout.GetAwaiter().GetResult())
            [IO.File]::WriteAllText("$checkHome/stderr.log", $stderr.GetAwaiter().GetResult())
        }
        $gui.Dispose()
    }
    if (Test-Path -LiteralPath $checkHome) {
        [IO.File]::WriteAllText("$checkHome/shutdown.json", (@{owned_gui_stopped=$stopped;test_exit_code=$exitCode;stopped_at=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json))
        Write-Output "Evidence: $checkHome"
    }
    Pop-Location
    if ($gui -and !$stopped) { throw 'Owned GUI failed to stop.' }
}
exit $exitCode
