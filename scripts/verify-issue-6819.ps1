#Requires -Version 7.0
[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$RequireRecovery,
    [switch]$IncludeInputMatrix,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$gwt = Join-Path $checkout 'target/debug/gwt.exe'
$gwtd = Join-Path $checkout 'target/debug/gwtd.exe'
$checkHome = Join-Path $checkout ('target/issue-6819-home-' + [Guid]::NewGuid().ToString('N'))
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
    if ($response.ok -isnot [bool] -or !$response.ok) { throw "$Operation failed: $($response.output)" }
    return $response
}
function Assert-Preflight($Doctor) {
    $preflight = $Doctor.preflight
    if ($preflight.schema_version -ne 1 -or $preflight.result -ne 'proceed' -or
        $preflight.context -ne 'current' -or $preflight.normal_completion -ne 'normal' -or
        $preflight.convergence -ne 'converged' -or @($preflight.reasons).Count) {
        throw ('Managed-browser preflight failed: ' + ($preflight | ConvertTo-Json -Depth 8 -Compress))
    }
    if ($preflight.diagnostics -isnot [Array]) { throw 'Managed-browser diagnostics are missing.' }
    foreach ($diagnostic in $preflight.diagnostics) {
        if ([string]::IsNullOrWhiteSpace($diagnostic.id) -or
            $diagnostic.classification -notin @('recovered', 'fail_open', 'foreign')) {
            throw 'Managed-browser diagnostic is unclassified.'
        }
    }
}

Push-Location -LiteralPath $checkout
try {
    $cargoArtifacts = & cargo build -p gwt --bin gwt --bin gwtd --target-dir (Join-Path $checkout 'target') --message-format=json
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed; checkout binaries are not fresh.' }
    $executables = @{}
    foreach ($line in $cargoArtifacts) {
        $artifact = $line | ConvertFrom-Json
        if ($artifact.reason -eq 'compiler-artifact' -and $artifact.target.kind -contains 'bin' -and $artifact.executable) {
            $executables[$artifact.target.name] = [IO.Path]::GetFullPath($artifact.executable)
        }
    }
    foreach ($expected in @($gwt, $gwtd)) {
        $name = [IO.Path]::GetFileNameWithoutExtension($expected)
        if (!$executables.ContainsKey($name) -or $executables[$name] -ne [IO.Path]::GetFullPath($expected)) {
            throw "Fresh Cargo artifact for $name must be $expected; reported path: $($executables[$name])."
        }
    }
    foreach ($file in @($gwt, $gwtd, $cli, $resolver)) {
        if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Required checkout binary or pinned Playwright dependency missing: $file" }
    }
    $doctor = (Invoke-SourceEnvelope 'hook.doctor' @{purpose='managed_browser'}).output | ConvertFrom-Json
    Assert-Preflight $doctor
    Write-Output ('Managed-browser preflight: ' + ($doctor.preflight | ConvertTo-Json -Depth 8 -Compress))
    Write-Output ('Windows recovery descriptor inherited: ' + [bool]$env:GWT_VERIFICATION_RECOVERY_SOURCE)
    if ($RequireRecovery -and !$env:GWT_VERIFICATION_RECOVERY_SOURCE) {
        throw 'This acceptance run must execute through canonical Windows Job recovery.'
    }
    $hook = $doctor.managed_hook_bin
    if (!$hook -or $hook -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable managed-hook fallback missing.' }
    Assert-Preflight ((Invoke-SourceEnvelope 'hook.doctor' @{purpose='managed_browser';repair=$true;expected_hook_bin=$hook}).output | ConvertFrom-Json)
    $repoHash = (Invoke-SourceEnvelope 'issue.monitor.status' @{}).project_store.hash
    if ($repoHash -notmatch '^[0-9a-f]{16}$') { throw 'Canonical project_store.hash missing.' }
    [void](New-Item -ItemType Directory -Path $checkHome -Force)
    [IO.File]::WriteAllText("$checkHome/preflight.json", ($doctor.preflight | ConvertTo-Json -Depth 8))
    $state = Join-Path $checkHome ".gwt/projects/$repoHash/project-state"
    [void](New-Item -ItemType Directory -Path $state, "$checkHome/.codex", "$checkHome/tmp" -Force)
    [IO.File]::WriteAllText("$checkHome/.codex/config.toml", '')
    [IO.File]::WriteAllText("$state/pm.json", '{"settings":{"auto_start":false}}')
    [IO.File]::WriteAllText("$state/issue-monitor.json", '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
    [IO.File]::WriteAllText("$checkHome/.gwt/session.json", (@{
        active_tab_id='verification-6819'; tabs=@(@{id='verification-6819';title='Verification source';project_root=$checkout;kind='git'});recent_projects=@()
    } | ConvertTo-Json -Depth 8))
    $runtime = Join-Path $env:USERPROFILE '.gwt/runtime'
    if (Test-Path -LiteralPath $runtime -PathType Container) {
        [void](New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target $runtime)
    }

    $tests = @('terminal-input-disconnect.spec.ts', 'agent-window-scroll.spec.ts')
    $config = Join-Path $checkout 'crates/gwt/playwright/playwright.config.ts'
    if ($IncludeInputMatrix) {
        # Reuse Issue #6806's exact acceptance test without importing its input
        # implementation or changing the existing expectations/timeouts.
        $blob = '71d67db6a85f40afd9d087db77a62796182c78e4'
        $matrix = Join-Path $checkHome 'input-matrix'
        [void](New-Item -ItemType Directory -Path $matrix)
        $lines = & git cat-file blob $blob
        if ($LASTEXITCODE) { throw 'Fetch Issue #6806 source e09cacc2678d8e0abb5eb738a3cd4dc066cf1924 before running its matrix.' }
        $continuity = Join-Path $matrix 'terminal-input-continuity-live.spec.ts'
        [IO.File]::WriteAllText($continuity, (($lines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
        if ((& git hash-object --no-filters $continuity).Trim() -cne $blob) { throw 'Original input acceptance blob changed.' }
        foreach ($testName in $tests) { [IO.File]::Copy((Join-Path $checkout "crates/gwt/playwright/tests/$testName"), (Join-Path $matrix $testName)) }
        [void](New-Item -ItemType Junction -Path "$matrix/_helpers" -Target (Join-Path $checkout 'crates/gwt/playwright/tests/_helpers'))
        $baseConfig = $config.Replace('\', '/') | ConvertTo-Json -Compress
        $config = Join-Path $matrix 'playwright.config.ts'
        [IO.File]::WriteAllText($config, "import base from $baseConfig; export default { ...base, testDir: __dirname };")
        $tests = @('terminal-input-continuity-live.spec.ts') + $tests
        Write-Output "Original Issue #6806 acceptance blob: $blob"
    }

    $start = [Diagnostics.ProcessStartInfo]::new($gwt)
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
    $start.Environment['GWT_HOOK_BIN'] = $hook
    $start.Environment['GWT_BROWSER_URL_FILE'] = "$checkHome/url.txt"
    $start.Environment['GWT_DISABLE_BACKGROUND_INDEX'] = '1'
    $start.Environment['RUST_LOG'] = 'info'
    $start.Environment['GIT_TERMINAL_PROMPT'] = '0'
    $start.Environment['GH_PROMPT_DISABLED'] = '1'
    $start.ArgumentList.Add('--no-tray')
    $start.ArgumentList.Add('--no-open')
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
    $assets = @{'app.js'='app.js'; 'terminal-wheel-scroll.js'='terminal-wheel-scroll.js'}
    foreach ($asset in $assets.Keys) {
        $served = (Invoke-WebRequest -Uri ($base.TrimEnd('/') + '/' + $asset)).Content
        if ($served -cne [IO.File]::ReadAllText((Join-Path $checkout "crates/gwt/web/$($assets[$asset])"))) {
            throw "Stale embedded asset: $asset"
        }
    }
    $anchor = [Net.WebSockets.ClientWebSocket]::new()
    $anchor.Options.SetRequestHeader('Origin', $base.TrimEnd('/'))
    $anchor.ConnectAsync([Uri](($base -replace '^http','ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
    Assert-Preflight ((Invoke-SourceEnvelope 'hook.doctor' @{purpose='managed_browser';expected_hook_bin=$hook}).output | ConvertFrom-Json)
    $runner = @{GWT_PLAYWRIGHT_BASE_URL=$base;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;
        GWT_PLAYWRIGHT_PROJECT_STATE=$state;GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
        NODE_OPTIONS=('--require "' + $resolver.Replace('\','/') + '" ' + $env:NODE_OPTIONS)}
    foreach ($key in $runner.Keys) {
        $savedRunner[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $runner[$key])
    }
    & node $cli test @tests --config $config --workers=1 --retries=0 --headed --output "$checkHome/playwright" @PlaywrightArguments
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
