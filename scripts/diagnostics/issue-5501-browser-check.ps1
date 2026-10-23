[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('before', 'after')][string]$Label = 'after',
    [int]$SampleMilliseconds = 10000,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwtBinary = Join-Path $checkout 'target/debug/gwt.exe'
$gwtdBinary = Join-Path $checkout 'target/debug/gwtd.exe'
$deps = Join-Path $env:LOCALAPPDATA 'Temp/gwt-playwright-win-1.49.1'
$realProfile = $env:USERPROFILE
$runId = [Guid]::NewGuid().ToString('N')
$checkHome = Join-Path $checkout "target/issue-5501-home-$runId"
$artifacts = Join-Path $checkout "crates/gwt/playwright/test-results/issue-5501-$Label-$runId"
$process = $null
$daemon = $null
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
try {
    Set-Location $checkout
    if (!(Test-Path $gwtBinary) -or !(Test-Path $gwtdBinary)) { throw 'Build checkout gwt and gwtd before browser-check.' }
    if (!(Test-Path "$deps/node_modules/@playwright/test/cli.js") -or !(Test-Path "$deps/resolve-playwright.cjs")) {
        throw "Install pinned Playwright dependencies and resolver at $deps before browser-check."
    }
    New-Item -ItemType Directory -Path "$checkHome/.gwt", $artifacts -Force | Out-Null
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
    @{tabs=@(@{id='terminal-output-5501';title='Terminal output';project_root=$checkout;kind='git'});recent_projects=@()} |
        ConvertTo-Json -Depth 8 | Set-Content "$checkHome/.gwt/session.json" -Encoding utf8NoBOM
    $hook = (Invoke-CheckoutEnvelope 'hook.doctor' @{}).output | ConvertFrom-Json
    $hookBinary = $hook.managed_hook_bin
    if (!$hookBinary -or $hookBinary -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable hook fallback missing' }
    $hookParams = @{repair=$true;expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    $repaired = (Invoke-CheckoutEnvelope 'hook.doctor' $hookParams).output | ConvertFrom-Json
    Assert-HookHealth $repaired.health
    $childVariables = @{HOME=$checkHome;USERPROFILE=$checkHome;GWT_BROWSER_URL_FILE="$checkHome/url.txt";
        GWT_HOOK_BIN=$hookBinary;GIT_TERMINAL_PROMPT='0';GH_PROMPT_DISABLED='1';GWT_BIN_PATH=$null}
    foreach ($key in $childVariables.Keys) {
        $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $childVariables[$key])
    }
    # A disabled Monitor intentionally does not start a daemon. Start only
    # the isolated IPC server so the CPU sample exercises real publish/ack.
    '{"schema_version":1,"operation":"daemon.start","params":{}}' |
        Set-Content "$checkHome/daemon-start.json" -Encoding utf8NoBOM
    $daemon = Start-Process -FilePath $gwtdBinary -WorkingDirectory $checkout -WindowStyle Hidden -PassThru -RedirectStandardInput "$checkHome/daemon-start.json" -RedirectStandardOutput "$checkHome/daemon-stdout.log" -RedirectStandardError "$checkHome/daemon-stderr.log"
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    do {
        $daemonStatus = (Invoke-CheckoutEnvelope 'daemon.status' @{}).output
        if ($daemonStatus -match '^running pid=(\d+).*probe=ok') { break }
        if ($daemon.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw 'Isolated daemon did not become ready' }
        Start-Sleep -Milliseconds 200
    } while ($true)
    if ([int]$Matches[1] -ne $daemon.Id) { throw 'Isolated daemon PID does not match its owner' }
    $daemonStatus | Set-Content "$artifacts/daemon-status.txt" -Encoding utf8NoBOM
    $process = Start-Process -FilePath $gwtBinary -ArgumentList '--no-tray','--no-open' -WorkingDirectory $checkout -WindowStyle Hidden -PassThru -RedirectStandardOutput "$checkHome/stdout.log" -RedirectStandardError "$checkHome/stderr.log"
    foreach ($key in $childVariables.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
    Write-Output "Owned checkout gwt PID: $($process.Id); isolated HOME: $checkHome"
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    while (!(Test-Path "$checkHome/url.txt")) {
        if ($process.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw "Checkout startup failed; inspect $checkHome" }
        Start-Sleep -Milliseconds 200
    }
    $baseUrl = (Get-Content "$checkHome/url.txt" -Raw).Trim()
    if ((Invoke-WebRequest -Method Head -Uri $baseUrl).StatusCode -ne 200) { throw 'Fresh checkout URL is not ready' }
    # --no-tray exits after its last browser disconnects. Keep the owning
    # check session alive across Playwright's dark/light context replacement.
    $keeper = [System.Net.WebSockets.ClientWebSocket]::new()
    $keeper.Options.SetRequestHeader('Origin', $baseUrl.TrimEnd('/'))
    $null = $keeper.ConnectAsync([Uri](($baseUrl -replace '^http', 'ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    $healthParams = @{expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    $health = (Invoke-CheckoutEnvelope 'hook.health' $healthParams).output | ConvertFrom-Json
    Assert-HookHealth $health
    $testVariables = @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PID=[string]$process.Id;GWT_PLAYWRIGHT_CHECKOUT_ROOT=$checkout;
        GWT_BATCHING_DAEMON_PID=[string]$daemon.Id;GWT_BATCHING_DAEMON_STATUS=$daemonStatus;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;GWT_BATCHING_CPU_LABEL=$Label;
        GWT_BATCHING_SAMPLE_MS=[string]$SampleMilliseconds;GWT_PLAYWRIGHT_NODE_MODULES="$deps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs";
        NODE_PATH="$deps/node_modules"}
    foreach ($key in $testVariables.Keys) {
        $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $testVariables[$key])
    }
    & node "$deps/node_modules/@playwright/test/cli.js" test --config crates/gwt/playwright/playwright.config.ts --output $artifacts --workers=1 terminal-output-batching-live.spec.ts @PlaywrightArguments
    if ($LASTEXITCODE -ne 0) { throw "Playwright failed with exit $LASTEXITCODE" }
    Write-Output "Browser evidence: $artifacts"
} finally {
    if ($keeper) { $keeper.Dispose() }
    if ($process -and !$process.HasExited) { $null = $process.WaitForExit(12000) }
    if ($process -and !$process.HasExited) {
        # Only this exact owned process and its descendants are terminated.
        & taskkill.exe /PID $process.Id /T /F 2>$null | Out-Null
        $null = $process.WaitForExit(5000)
        if (!$process.HasExited) { throw "Owned checkout PID $($process.Id) survived cleanup" }
    }
    if ($daemon -and !$daemon.HasExited) {
        Stop-Process -Id $daemon.Id
        if (!$daemon.WaitForExit(5000)) { throw 'Owned isolated daemon survived cleanup' }
    }
    foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
}
