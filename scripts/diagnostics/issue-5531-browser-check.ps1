[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('before', 'after')][string]$Label = 'after',
    [ValidateRange(1000, 7200000)][int]$BackgroundMilliseconds = 1800000,
    [switch]$AllowSmoke,
    [string]$BaselineRef = 'origin/develop',
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwtBinary = Join-Path $checkout 'target/debug/gwt.exe'
$gwtdBinary = Join-Path $checkout 'target/debug/gwtd.exe'
$deps = Join-Path $env:LOCALAPPDATA 'Temp/gwt-playwright-win-1.49.1'
$realProfile = $env:USERPROFILE
$runId = [Guid]::NewGuid().ToString('N')
$checkHome = Join-Path $checkout "target/issue-5531-home-$runId"
$artifacts = Join-Path $checkout "crates/gwt/playwright/test-results/issue-5531-$Label-$runId"
$process = $null
$keeper = $null
$savedEnvironment = @{}

function Invoke-CheckoutEnvelope($operation, $parameters) {
    $envelope = @{schema_version=1;operation=$operation;params=$parameters} | ConvertTo-Json -Depth 12 -Compress
    $result = $envelope | & $gwtdBinary | ConvertFrom-Json
    if (!$result.ok) { throw "$operation failed: $($result.output)" }
    return $result
}
function Assert-HookHealth($health) {
    $blocking = @($health.issues | Where-Object { $_ -notlike 'managed hook failure:*state=fail-open*' })
    if ($health.status -eq 'inactive' -or $blocking.Count) { throw "hook convergence failed: $($blocking -join ', ')" }
}
function Set-CheckEnvironment($variables) {
    foreach ($key in $variables.Keys) {
        if (!$savedEnvironment.ContainsKey($key)) { $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key) }
        [Environment]::SetEnvironmentVariable($key, $variables[$key])
    }
}
function Get-SourceDigest($path) {
    $bytes = [Text.Encoding]::UTF8.GetBytes([IO.File]::ReadAllText($path).Replace("`r`n", "`n"))
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
}

try {
    Set-Location $checkout
    if ($BackgroundMilliseconds -lt 1800000 -and !$AllowSmoke) { throw 'A shortened run requires -AllowSmoke and does not prove the 30-minute acceptance criterion.' }
    if (!(Test-Path $gwtBinary) -or !(Test-Path $gwtdBinary)) { throw 'Build checkout gwt and gwtd before browser-check.' }
    if (!(Test-Path "$deps/node_modules/@playwright/test/cli.js") -or !(Test-Path "$deps/resolve-playwright.cjs")) {
        throw "Install pinned Playwright dependencies and resolver at $deps before browser-check."
    }
    $wsPackage = Join-Path $checkout 'target/issue5531-node/node_modules/ws'
    if (!(Test-Path "$wsPackage/package.json") -or (Get-Content "$wsPackage/package.json" -Raw | ConvertFrom-Json).version -ne '8.18.3') {
        throw 'Install the pinned native WebSocket fixture: npm install --prefix target/issue5531-node --no-audit --no-fund ws@8.18.3'
    }
    $baselineSha = (& git rev-parse --verify "$BaselineRef^{commit}").Trim()
    if ($LASTEXITCODE -ne 0 -or !$baselineSha) { throw 'Cannot pin the baseline Git commit.' }
    New-Item -ItemType Directory -Path "$checkHome/.gwt", $artifacts -Force | Out-Null
    # Playwright 1.49.1 pins every page visible in its own CDP session.
    # A second CDP session cannot release that session's capturer handle.
    # Use a private runner copy with only that emulation disabled, so native
    # visibilitychange and background throttling remain measurable.
    $runDeps = Join-Path $checkHome 'playwright'
    New-Item -ItemType Directory -Path "$runDeps/node_modules/@playwright" -Force | Out-Null
    Copy-Item -LiteralPath "$deps/node_modules/@playwright/test" -Destination "$runDeps/node_modules/@playwright/test" -Recurse
    foreach ($package in @('playwright', 'playwright-core')) {
        Copy-Item -LiteralPath "$deps/node_modules/$package" -Destination "$runDeps/node_modules/$package" -Recurse
    }
    $playwrightVersion = (Get-Content "$runDeps/node_modules/playwright/package.json" -Raw | ConvertFrom-Json).version
    if ($playwrightVersion -ne '1.49.1') { throw 'Native visibility setup requires the audited Playwright 1.49.1 runner.' }
    $chromiumDriver = "$runDeps/node_modules/playwright-core/lib/server/chromium/crPage.js"
    $driverSource = [IO.File]::ReadAllText($chromiumDriver)
    $focusPattern = "(this\._client\.send\('Emulation\.setFocusEmulationEnabled', \{\s*enabled: )true(\s*\}\))"
    if ([regex]::Matches($driverSource, $focusPattern).Count -ne 1) { throw 'Unexpected Playwright focus-emulation source; refuse unverifiable background evidence.' }
    [IO.File]::WriteAllText($chromiumDriver, [regex]::Replace($driverSource, $focusPattern, '${1}false${2}'))
    if (Test-Path "$realProfile/.gwt/runtime") {
        New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target "$realProfile/.gwt/runtime" | Out-Null
    }
    $monitor = Invoke-CheckoutEnvelope 'issue.monitor.status' @{}
    $repoHash = $monitor.project_store.hash
    if (!$repoHash) { throw 'project_store.hash missing' }
    $projectState = "$checkHome/.gwt/projects/$repoHash/project-state"
    New-Item -ItemType Directory -Path $projectState -Force | Out-Null
    '{"settings":{"auto_start":false}}' | Set-Content "$projectState/pm.json" -Encoding utf8NoBOM
    '{"enabled":false,"max_active_agents":1,"priority_order":[]}' | Set-Content "$projectState/issue-monitor.json" -Encoding utf8NoBOM
    @{tabs=@(@{id='socket-receive-5531';title='Receive load';project_root=$checkout;kind='git'});recent_projects=@()} |
        ConvertTo-Json -Depth 8 | Set-Content "$checkHome/.gwt/session.json" -Encoding utf8NoBOM
    $hook = (Invoke-CheckoutEnvelope 'hook.doctor' @{}).output | ConvertFrom-Json
    $hookBinary = $hook.managed_hook_bin
    if (!$hookBinary -or $hookBinary -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable managed-hook fallback missing' }
    $hookParams = @{repair=$true;expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    $repaired = (Invoke-CheckoutEnvelope 'hook.doctor' $hookParams).output | ConvertFrom-Json
    Assert-HookHealth $repaired.health
    $childVariables = @{HOME=$checkHome;USERPROFILE=$checkHome;GWT_BROWSER_URL_FILE="$checkHome/url.txt";
        GWT_HOOK_BIN=$hookBinary;GIT_TERMINAL_PROMPT='0';GH_PROMPT_DISABLED='1';GWT_BIN_PATH=$null}
    Set-CheckEnvironment $childVariables
    $process = Start-Process -FilePath $gwtBinary -ArgumentList '--no-tray','--no-open' -WorkingDirectory $checkout -WindowStyle Hidden -PassThru -RedirectStandardOutput "$checkHome/stdout.log" -RedirectStandardError "$checkHome/stderr.log"
    foreach ($key in $childVariables.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
    Write-Output "Owned checkout gwt PID: $($process.Id); isolated HOME: $checkHome; baseline SHA: $baselineSha"
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    while (!(Test-Path "$checkHome/url.txt")) {
        if ($process.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw "Checkout startup failed; inspect $checkHome" }
        Start-Sleep -Milliseconds 200
    }
    $baseUrl = (Get-Content "$checkHome/url.txt" -Raw).Trim()
    if ((Invoke-WebRequest -Method Head -Uri $baseUrl).StatusCode -ne 200) { throw 'Fresh checkout URL is not ready' }
    foreach ($asset in @('app.js', 'socket-receive-dispatcher.js', 'agents-surface.js', 'board-logs-surface.js', 'knowledge-kanban-surface.js',
        'branches-cleanup-surface.js', 'control-explanations.js', 'profile-window-surface.js', 'project-index-search-surface.js', 'settings-surface.js')) {
        $served = (Invoke-WebRequest -Uri ($baseUrl.TrimEnd('/') + '/' + $asset)).Content
        $expected = [IO.File]::ReadAllText((Join-Path $checkout "crates/gwt/web/$asset"))
        if ($served -cne $expected) { throw "Checkout binary embeds stale $asset; rebuild gwt and gwtd before retrying." }
    }
    # Keep this exact owned server alive across dark/light browser replacement.
    $keeper = [System.Net.WebSockets.ClientWebSocket]::new()
    $keeper.Options.SetRequestHeader('Origin', $baseUrl.TrimEnd('/'))
    $null = $keeper.ConnectAsync([Uri](($baseUrl -replace '^http', 'ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    $healthParams = @{expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    $health = (Invoke-CheckoutEnvelope 'hook.health' $healthParams).output | ConvertFrom-Json
    Assert-HookHealth $health
    $testVariables = @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PID=[string]$process.Id;GWT_PLAYWRIGHT_CHECKOUT_ROOT=$checkout;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;GWT_SOCKET_LOAD_PROJECT_KEY=$repoHash;
        GWT_SOCKET_LOAD_LABEL=$Label;GWT_SOCKET_LOAD_DURATION_MS=[string]$BackgroundMilliseconds;
        GWT_SOCKET_WS_PACKAGE=$wsPackage;
        GWT_SOCKET_LOAD_ALLOW_SMOKE=$(if ($AllowSmoke) {'1'} else {'0'});GWT_SOCKET_LOAD_BASELINE_SHA=$baselineSha;
        GWT_PLAYWRIGHT_NODE_MODULES="$runDeps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs";NODE_PATH="$deps/node_modules"}
    Set-CheckEnvironment $testVariables
    $sourceHashes = @{}
    foreach ($source in @('app.js', 'socket-receive-dispatcher.js', 'board-logs-surface.js', 'branches-cleanup-surface.js',
        'agents-surface.js', 'control-explanations.js', 'knowledge-kanban-surface.js', 'profile-window-surface.js',
        'project-index-search-surface.js', 'settings-surface.js')) {
        $sourceHashes[$source] = Get-SourceDigest (Join-Path $checkout "crates/gwt/web/$source")
    }
    $testHashes = @{}
    foreach ($source in @('crates/gwt/playwright/tests/socket-receive-backpressure-live.spec.ts', 'scripts/diagnostics/issue-5531-browser-check.ps1')) {
        $testHashes[$source] = Get-SourceDigest (Join-Path $checkout $source)
    }
    @{label=$Label;baseline_sha=$baselineSha;head_sha=(& git rev-parse HEAD).Trim();owned_pid=$process.Id;source_sha256=$sourceHashes;test_sha256=$testHashes;
        checkout=$checkout;isolated_home=$checkHome;background_milliseconds=$BackgroundMilliseconds;
        acceptance_duration=($BackgroundMilliseconds -ge 1800000);playwright_version=$playwrightVersion;
        native_visibility='isolated runner: focus emulation disabled; browser background defaults restored'} | ConvertTo-Json |
        Set-Content "$checkHome/launch.json" -Encoding utf8NoBOM
    & node "$runDeps/node_modules/@playwright/test/cli.js" test --config crates/gwt/playwright/playwright.config.ts --output $artifacts --workers=1 --headed socket-receive-backpressure-live.spec.ts @PlaywrightArguments
    if ($LASTEXITCODE -ne 0) { throw "Playwright failed with exit $LASTEXITCODE" }
    Write-Output "Browser evidence: $artifacts"
} finally {
    if ($keeper) { $keeper.Dispose() }
    if ($process -and !$process.HasExited) { $null = $process.WaitForExit(12000) }
    if ($process -and !$process.HasExited) {
        # Only the recorded owned process and its descendants may be stopped.
        & taskkill.exe /PID $process.Id /T /F 2>$null | Out-Null
        $null = $process.WaitForExit(5000)
        if (!$process.HasExited) { throw "Owned checkout PID $($process.Id) survived cleanup" }
    }
    foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
    # Playwright clears its output directory at startup. Preserve the launch
    # manifest outside it, and restore evidence only after owned cleanup.
    if (Test-Path "$checkHome/launch.json") {
        New-Item -ItemType Directory -Path $artifacts -Force | Out-Null
        Copy-Item -LiteralPath "$checkHome/launch.json" -Destination "$artifacts/launch.json"
    }
}
